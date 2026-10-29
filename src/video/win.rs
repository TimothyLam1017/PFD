//! Windows video thread: Media Foundation media engine (frame-server mode) + Direct3D 11.
//!
//! Everything here runs on one dedicated thread, which also owns (and pumps messages
//! for) the fullscreen window.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, ColorImage};
use windows::core::{w, Interface, BOOL, BSTR, HRESULT, PCSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::implement;

use super::{display_size, fit_within, lock, Cmd, FitMode, FullscreenKey, Shared, UiEvent};

/// (engine generation, event, param1, param2) from IMFMediaEngineNotify.
type EngineEvent = (u64, u32, usize, u32);

const WM_WAKE: u32 = WM_APP + 1;
const TOPMOST_TIMER: usize = 1;
const HINT_TIMER: usize = 2;
/// Mid-point of the 3–5 s board range for the one-time exit hint.
const HINT_DURATION: Duration = Duration::from_secs(4);
const HINT_TEXT: &str = "Double-click to exit fullscreen";
const CLASS_NAME: windows::core::PCWSTR = w!("APlayerFullscreenVideo");
const HINT_CLASS: windows::core::PCWSTR = w!("APlayerFullscreenExitHint");
/// A file that shows no picture for this long is reported as failed (and skipped).
const NO_PICTURE_LIMIT: Duration = Duration::from_secs(20);

/// Ask Windows to relaunch the player with these arguments if it ever crashes or stops
/// responding (Windows does this for programs that ran at least 60 seconds). Called
/// again whenever the position changes, so it always continues from the current item.
pub(super) fn register_os_restart(args: &[String]) {
    use windows::Win32::System::Recovery::{
        RegisterApplicationRestart, RESTART_NO_PATCH, RESTART_NO_REBOOT,
    };
    let line: Vec<u16> = args
        .join(" ")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let _ = RegisterApplicationRestart(
            windows::core::PCWSTR(line.as_ptr()),
            RESTART_NO_PATCH | RESTART_NO_REBOOT,
        );
    }
}

/// Private (committed) memory of this process in MB.
pub(super) fn private_memory_mb() -> Option<u64> {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) }.ok()?;
    Some((counters.PagefileUsage / (1024 * 1024)) as u64)
}

/// Named mutex preventing a second player instance on Windows.
///
/// Released briefly before a watchdog/self restart so the replacement can acquire it;
/// re-acquired if spawn fails and this process keeps running.
const SINGLE_INSTANCE_MUTEX: windows::core::PCWSTR = w!("Local\\A-Player-audio-player-single");

/// Opaque Windows HANDLE wrapper so a process-wide named-mutex handle can live in a
/// `static Mutex` (raw `HANDLE` is not `Send`/`Sync` in the `windows` crate).
#[derive(Clone, Copy)]
struct SendSyncHandle(HANDLE);

// SAFETY: `HANDLE` is an opaque pointer-sized OS handle value. We only store the
// named-mutex handle for acquire/release across threads under `SINGLE_INSTANCE`.
unsafe impl Send for SendSyncHandle {}
unsafe impl Sync for SendSyncHandle {}

static SINGLE_INSTANCE: std::sync::Mutex<Option<SendSyncHandle>> = std::sync::Mutex::new(None);

pub(super) fn acquire_single_instance() -> Result<(), String> {
    unsafe {
        let handle = CreateMutexW(None, true, SINGLE_INSTANCE_MUTEX).map_err(|e| {
            format!("CreateMutexW(single-instance) failed: {e}")
        })?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(handle);
            return Err(
                "Another audio-player instance is already running (single-instance mutex held)"
                    .into(),
            );
        }
        *SINGLE_INSTANCE
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(SendSyncHandle(handle));
        Ok(())
    }
}

pub(super) fn release_single_instance_for_restart() {
    let mut guard = SINGLE_INSTANCE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(SendSyncHandle(handle)) = guard.take() {
        unsafe {
            let _ = CloseHandle(handle);
        }
    }
}

pub(super) fn reacquire_single_instance() -> Result<(), String> {
    acquire_single_instance()
}

/// Wake the video thread so a command is handled immediately.
pub(super) fn wake(shared: &Shared) {
    let id = shared.thread_id.load(Ordering::Relaxed);
    if id != 0 {
        unsafe {
            let _ = PostThreadMessageW(id, WM_WAKE, WPARAM(0), LPARAM(0));
        }
    }
}

// ------------------------------------------------------------ media engine events

#[implement(IMFMediaEngineNotify)]
struct Notify {
    tx: Sender<EngineEvent>,
    thread_id: u32,
    generation: u64,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, param1: usize, param2: u32) -> windows::core::Result<()> {
        let _ = self.tx.send((self.generation, event, param1, param2));
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_WAKE, WPARAM(0), LPARAM(0));
        }
        Ok(())
    }
}

// ------------------------------------------------------------- fullscreen window

#[derive(Default)]
struct WindowInput {
    double_click: bool,
    keys: Vec<FullscreenKey>,
    display_changed: bool,
    redraw: bool,
}

thread_local! {
    static INPUT: RefCell<WindowInput> = RefCell::new(WindowInput::default());
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // Double-click (or Alt+F4) leaves fullscreen; it never quits the player.
            WM_LBUTTONDBLCLK | WM_CLOSE => {
                INPUT.with(|i| i.borrow_mut().double_click = true);
                LRESULT(0)
            }
            WM_SETCURSOR => {
                SetCursor(None);
                LRESULT(1)
            }
            WM_SYSCOMMAND
                if (wparam.0 & 0xFFF0) as u32 == SC_SCREENSAVE
                    || (wparam.0 & 0xFFF0) as u32 == SC_MONITORPOWER =>
            {
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TOPMOST_TIMER => {
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                let vk = VIRTUAL_KEY(wparam.0 as u16);
                let key = match vk {
                    VK_SPACE | VK_MEDIA_PLAY_PAUSE => Some(FullscreenKey::PlayPause),
                    VK_RIGHT if ctrl => Some(FullscreenKey::Next),
                    VK_LEFT if ctrl => Some(FullscreenKey::Previous),
                    VK_RIGHT => Some(FullscreenKey::SeekForward),
                    VK_LEFT => Some(FullscreenKey::SeekBack),
                    VK_N | VK_MEDIA_NEXT_TRACK => Some(FullscreenKey::Next),
                    VK_P | VK_MEDIA_PREV_TRACK => Some(FullscreenKey::Previous),
                    _ => None,
                };
                if let Some(key) = key {
                    INPUT.with(|i| i.borrow_mut().keys.push(key));
                }
                LRESULT(0)
            }
            WM_DISPLAYCHANGE => {
                INPUT.with(|i| i.borrow_mut().display_changed = true);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let _ = ValidateRect(Some(hwnd), None);
                INPUT.with(|i| i.borrow_mut().redraw = true);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Connected monitors as (rectangle, is_primary, friendly_name), primary first.
pub(super) fn monitors() -> Vec<(RECT, bool, String)> {
    unsafe extern "system" fn cb(hmon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<(RECT, bool, String)>) };
        let mut info = MONITORINFOEXW {
            monitorInfo: MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        let ok = unsafe {
            GetMonitorInfoW(
                hmon,
                &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
            )
        };
        if ok.as_bool() {
            let primary = info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0;
            let name = monitor_friendly_name(&info.szDevice);
            list.push((info.monitorInfo.rcMonitor, primary, name));
        }
        TRUE
    }
    let mut list: Vec<(RECT, bool, String)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut list as *mut _ as isize));
    }
    list.sort_by_key(|(r, primary, _)| (!*primary, r.left, r.top));
    list
}

/// Friendly adapter name for a monitor device path (`\\.\DISPLAYn`), or empty if unknown.
fn monitor_friendly_name(device: &[u16]) -> String {
    let mut dd = DISPLAY_DEVICEW {
        cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        EnumDisplayDevicesW(
            windows::core::PCWSTR(device.as_ptr()),
            0,
            &mut dd,
            0,
        )
    };
    if !ok.as_bool() {
        return String::new();
    }
    let end = dd
        .DeviceString
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(dd.DeviceString.len());
    String::from_utf16_lossy(&dd.DeviceString[..end])
        .trim()
        .to_owned()
}

/// Tiny layered popup that shows [`HINT_TEXT`] at the bottom of the fullscreen monitor
/// for [`HINT_DURATION`], fading out in the last second.
fn create_exit_hint(instance: HINSTANCE, monitor: RECT) -> Option<ExitHint> {
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(hint_wndproc),
            hInstance: instance,
            lpszClassName: HINT_CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);

        let hint_w = 420i32;
        let hint_h = 36i32;
        let mon_w = monitor.right - monitor.left;
        let x = monitor.left + (mon_w - hint_w).max(0) / 2;
        let y = monitor.bottom - hint_h - 48;
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            HINT_CLASS,
            w!(""),
            WS_POPUP | WS_VISIBLE,
            x,
            y,
            hint_w,
            hint_h,
            None,
            None,
            Some(instance),
            None,
        )
        .ok()?;

        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 230, LWA_ALPHA);
        let _ = InvalidateRect(Some(hwnd), None, true);
        SetTimer(Some(hwnd), HINT_TIMER, 50, None);
        Some(ExitHint {
            hwnd,
            shown_at: Instant::now(),
        })
    }
}

fn destroy_exit_hint(hint: ExitHint) {
    unsafe {
        let _ = KillTimer(Some(hint.hwnd), HINT_TIMER);
        let _ = DestroyWindow(hint.hwnd);
    }
}

fn tick_exit_hint(hint: &ExitHint) -> bool {
    let elapsed = hint.shown_at.elapsed();
    if elapsed >= HINT_DURATION {
        return false;
    }
    let alpha = if elapsed + Duration::from_secs(1) >= HINT_DURATION {
        let left = (HINT_DURATION - elapsed).as_secs_f32().clamp(0.0, 1.0);
        (230.0 * left) as u8
    } else {
        230
    };
    unsafe {
        let _ = SetLayeredWindowAttributes(hint.hwnd, COLORREF(0), alpha, LWA_ALPHA);
    }
    true
}

extern "system" fn hint_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER if wparam.0 == HINT_TIMER => LRESULT(0),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let bg = CreateSolidBrush(COLORREF(0x0020_2020));
                let _ = FillRect(hdc, &rect, bg);
                let _ = DeleteObject(bg.into());
                let _ = SetBkMode(hdc, TRANSPARENT);
                let _ = SetTextColor(hdc, COLORREF(0x00FF_FFFF));
                let mut text: Vec<u16> = HINT_TEXT.encode_utf16().collect();
                let _ = DrawTextW(
                    hdc,
                    &mut text,
                    &mut rect,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                );
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn register_class() -> Result<HINSTANCE, String> {
    unsafe {
        let instance: HINSTANCE = GetModuleHandleW(None)
            .map_err(|e| format!("GetModuleHandleW failed while registering fullscreen window class: {e}"))?
            .into();
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS | CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        let atom = RegisterClassExW(&class);
        if atom == 0 {
            let err = GetLastError();
            // Already registered in this process (e.g. video thread re-entry) is fine.
            if err != ERROR_CLASS_ALREADY_EXISTS {
                return Err(format!(
                    "RegisterClassExW(APlayerFullscreenVideo) failed: {err:?} (refusing to continue with a default/null HINSTANCE)"
                ));
            }
        }
        Ok(instance)
    }
}

// ------------------------------------------------------------------ Direct3D

const SHADER: &str = r#"
cbuffer Params : register(b0) { float4 src; };   // uv offset (xy) and scale (zw)
Texture2D tex : register(t0);
SamplerState smp : register(s0);
struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
VSOut vs_main(uint id : SV_VertexID) {
    VSOut o;
    float2 t = float2((id << 1) & 2, id & 2);          // one triangle covering the viewport
    o.pos = float4(t * float2(2, -2) + float2(-1, 1), 0, 1);
    o.uv = src.xy + t * src.zw;
    return o;
}
float4 ps_main(VSOut i) : SV_Target { return float4(tex.Sample(smp, i.uv).rgb, 1); }
"#;

fn compile(entry: &str, target: &str) -> Result<Vec<u8>, String> {
    let entry = std::ffi::CString::new(entry).unwrap();
    let target = std::ffi::CString::new(target).unwrap();
    let mut code = None;
    let mut errors = None;
    let result = unsafe {
        D3DCompile(
            SHADER.as_ptr().cast(),
            SHADER.len(),
            PCSTR::null(),
            None,
            None,
            PCSTR(entry.as_ptr().cast()),
            PCSTR(target.as_ptr().cast()),
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(e) = result {
        let msg = errors
            .map(|b| unsafe {
                String::from_utf8_lossy(std::slice::from_raw_parts(
                    b.GetBufferPointer() as *const u8,
                    b.GetBufferSize(),
                ))
                .into_owned()
            })
            .unwrap_or_default();
        return Err(format!("shader compile failed: {e} {msg}"));
    }
    let blob = code.ok_or("shader compile returned nothing")?;
    Ok(unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize())
            .to_vec()
    })
}

fn create_device() -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let levels = [
        D3D_FEATURE_LEVEL_11_1,
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
    ];
    let attempts = [
        (
            D3D_DRIVER_TYPE_HARDWARE,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
        ),
        (D3D_DRIVER_TYPE_HARDWARE, D3D11_CREATE_DEVICE_BGRA_SUPPORT),
        (D3D_DRIVER_TYPE_WARP, D3D11_CREATE_DEVICE_BGRA_SUPPORT),
    ];
    let mut last = None;
    for (driver, flags) in attempts {
        let mut device = None;
        let mut ctx = None;
        match unsafe {
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                flags,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )
        } {
            Ok(()) => {
                log::info!("Direct3D 11 device: {driver:?} {flags:?}");
                return Ok((device.unwrap(), ctx.unwrap()));
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap())
}

struct Gpu {
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    factory: IDXGIFactory2,
    manager: IMFDXGIDeviceManager,
    engine_factory: IMFMediaEngineClassFactory,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    cbuffer: ID3D11Buffer,
    sampler: ID3D11SamplerState,
}

impl Gpu {
    fn new() -> Result<Self, String> {
        unsafe {
            let (device, ctx) = create_device().map_err(|e| format!("Direct3D 11: {e}"))?;
            if let Ok(mt) = device.cast::<ID3D11Multithread>() {
                let _ = mt.SetMultithreadProtected(true);
            }
            let factory: IDXGIFactory2 =
                CreateDXGIFactory1().map_err(|e| format!("DXGI factory: {e}"))?;

            let mut token = 0u32;
            let mut manager = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)
                .map_err(|e| format!("DXGI manager: {e}"))?;
            let manager = manager.ok_or("DXGI manager missing")?;
            manager
                .ResetDevice(&device, token)
                .map_err(|e| format!("DXGI manager reset: {e}"))?;
            let engine_factory: IMFMediaEngineClassFactory =
                CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_ALL)
                    .map_err(|e| format!("media engine factory: {e}"))?;

            let vs_code = compile("vs_main", "vs_4_0")?;
            let ps_code = compile("ps_main", "ps_4_0")?;
            let mut vs = None;
            let mut ps = None;
            device
                .CreateVertexShader(&vs_code, None, Some(&mut vs))
                .map_err(|e| e.to_string())?;
            device
                .CreatePixelShader(&ps_code, None, Some(&mut ps))
                .map_err(|e| e.to_string())?;
            let mut cbuffer = None;
            device
                .CreateBuffer(
                    &D3D11_BUFFER_DESC {
                        ByteWidth: 16,
                        Usage: D3D11_USAGE_DEFAULT,
                        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                        ..Default::default()
                    },
                    None,
                    Some(&mut cbuffer),
                )
                .map_err(|e| e.to_string())?;
            let mut sampler = None;
            device
                .CreateSamplerState(
                    &D3D11_SAMPLER_DESC {
                        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                        MaxLOD: f32::MAX,
                        ..Default::default()
                    },
                    Some(&mut sampler),
                )
                .map_err(|e| e.to_string())?;
            Ok(Self {
                device,
                ctx,
                factory,
                manager,
                engine_factory,
                vs: vs.unwrap(),
                ps: ps.unwrap(),
                cbuffer: cbuffer.unwrap(),
                sampler: sampler.unwrap(),
            })
        }
    }

    fn texture(
        &self,
        w: u32,
        h: u32,
        bind: u32,
        usage: D3D11_USAGE,
        cpu: u32,
    ) -> windows::core::Result<ID3D11Texture2D> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: usage,
            BindFlags: bind,
            CPUAccessFlags: cpu,
            MiscFlags: 0,
        };
        let mut tex = None;
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut tex))? };
        Ok(tex.unwrap())
    }
}

/// A video frame at its correct display size, used as the source for fullscreen drawing.
struct FrameTex {
    tex: ID3D11Texture2D,
    srv: ID3D11ShaderResourceView,
    w: u32,
    h: u32,
}

/// Small render target + CPU-readable copy for the windowed preview.
struct Preview {
    target: ID3D11Texture2D,
    staging: ID3D11Texture2D,
    w: u32,
    h: u32,
}

struct Fullscreen {
    hwnd: HWND,
    swapchain: IDXGISwapChain1,
    rtv: ID3D11RenderTargetView,
    w: u32,
    h: u32,
    monitor: usize,
    /// One-time bottom exit hint popup (layered), if armed for this entry.
    exit_hint: Option<ExitHint>,
}

struct ExitHint {
    hwnd: HWND,
    shown_at: Instant,
}

/// One media engine playing (or pre-loaded with) one file.
struct Slot {
    engine: IMFMediaEngine,
    generation: u64,
    path: PathBuf,
    fit: FitMode,
    opened: Instant,
    loaded: bool,
    has_video: bool,
    first_frame: bool,
}

impl Slot {
    fn shutdown(self) {
        unsafe {
            let _ = self.engine.Shutdown();
        }
    }
}

struct VideoThread {
    shared: Arc<Shared>,
    ctx: egui::Context,
    gpu: Gpu,
    thread_id: u32,
    events_tx: Sender<EngineEvent>,
    events_rx: Receiver<EngineEvent>,
    instance: HINSTANCE,

    current: Option<Slot>,
    preload: Option<Slot>,
    looping: bool,
    paused: bool,
    volume: f32,
    fit: FitMode,

    frame: Option<FrameTex>,
    preview: Option<Preview>,
    preview_max: (u32, u32),
    /// Produce a windowed frame even without a new video frame (after seek/exit fullscreen).
    force_preview: bool,

    fullscreen: Option<Fullscreen>,
    /// Fullscreen requested but no monitor available yet (e.g. right after boot).
    fullscreen_pending: Option<(usize, Instant)>,
    keep_awake: bool,
}

pub(super) fn run(rx: Receiver<Cmd>, shared: Arc<Shared>, ctx: egui::Context) {
    let thread_id = unsafe { GetCurrentThreadId() };
    shared.thread_id.store(thread_id, Ordering::Relaxed);
    shared.beat();

    let setup = unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| format!("COM: {e}"))
            .and_then(|()| {
                MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| format!("Media Foundation: {e}"))
            })
            .and_then(|()| Gpu::new())
    };
    let gpu = match setup {
        Ok(gpu) => gpu,
        Err(e) => {
            log::error!("Video unavailable: {e}");
            return failed_loop(rx, shared, ctx, e);
        }
    };
    let mut mmcss_index = 0u32;
    let mmcss = unsafe { AvSetMmThreadCharacteristicsW(w!("Playback"), &mut mmcss_index) };
    let instance = match register_class() {
        Ok(instance) => instance,
        Err(e) => {
            log::error!("Video unavailable: {e}");
            return failed_loop(rx, shared, ctx, e);
        }
    };

    let (events_tx, events_rx) = channel();
    let mut vt = VideoThread {
        shared,
        ctx,
        gpu,
        thread_id,
        events_tx,
        events_rx,
        instance,
        current: None,
        preload: None,
        looping: false,
        paused: false,
        volume: 0.5,
        fit: FitMode::Fit,
        frame: None,
        preview: None,
        preview_max: (0, 0),
        force_preview: false,
        fullscreen: None,
        fullscreen_pending: None,
        keep_awake: false,
    };
    vt.run(rx);

    unsafe {
        if let Ok(h) = mmcss {
            let _ = AvRevertMmThreadCharacteristics(h);
        }
        let _ = MFShutdown();
    }
}

/// Media Foundation or Direct3D could not start: report an error for every video.
fn failed_loop(rx: Receiver<Cmd>, shared: Arc<Shared>, ctx: egui::Context, reason: String) {
    loop {
        shared.beat();
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Cmd::Open { generation, .. }) => {
                let mut s = lock(&shared.status);
                if s.generation == generation {
                    s.error = Some(format!("Video unavailable: {reason}"));
                }
                drop(s);
                ctx.request_repaint();
            }
            Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn describe_hresult(hr: HRESULT) -> String {
    let hint = match hr.0 as u32 {
        0xC00D5212 => " (codec not installed — HEVC / VP9 / AV1 need the matching Video Extension from the Microsoft Store)",
        0xC00D36C4 => " (file type not supported)",
        0xC00D36B4 => " (media type not supported)",
        0x80070002 | 0x80070003 => " (file not found)",
        0x80070005 => " (access denied)",
        0xC00D36E6 => " (no decoder for this format)",
        _ => "",
    };
    format!("0x{:08X}{hint}", hr.0 as u32)
}

impl VideoThread {
    fn run(&mut self, rx: Receiver<Cmd>) {
        loop {
            self.shared.beat();

            // Commands from the app.
            loop {
                match rx.try_recv() {
                    Ok(cmd) => self.command(cmd),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.exit_fullscreen(false);
                        if let Some(s) = self.current.take() {
                            s.shutdown();
                        }
                        if let Some(s) = self.preload.take() {
                            s.shutdown();
                        }
                        return;
                    }
                }
            }

            // Window / thread messages.
            unsafe {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            self.window_input();
            self.engine_events();
            self.check_stalled();
            self.frame_tick();
            self.tick_fullscreen_hint();
            self.update_status();
            self.update_power();

            if let Some((monitor, since)) = self.fullscreen_pending {
                if since.elapsed() > Duration::from_secs(1) {
                    self.fullscreen_pending = None;
                    self.enter_fullscreen(monitor);
                }
            }

            let busy = self.current.is_some() || self.fullscreen.is_some();
            unsafe {
                MsgWaitForMultipleObjects(None, false, if busy { 2 } else { 50 }, QS_ALLINPUT);
            }
        }
    }

    // ---------------------------------------------------------------- commands

    fn command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Open {
                path,
                generation,
                fit,
            } => {
                for slot in [self.current.take(), self.preload.take()]
                    .into_iter()
                    .flatten()
                {
                    slot.shutdown();
                }
                self.paused = false;
                self.fit = fit;
                match self.open_slot(&path, generation, fit, true) {
                    Ok(slot) => {
                        unsafe {
                            let _ = slot.engine.SetLoop(self.looping);
                        }
                        self.current = Some(slot);
                    }
                    Err(e) => self.report_error(generation, e),
                }
            }
            Cmd::Preload(next) => match next {
                Some((path, generation, fit)) => {
                    if self
                        .preload
                        .as_ref()
                        .is_some_and(|p| p.generation == generation)
                    {
                        return;
                    }
                    if let Some(p) = self.preload.take() {
                        p.shutdown();
                    }
                    match self.open_slot(&path, generation, fit, false) {
                        Ok(slot) => {
                            unsafe {
                                let _ = slot.engine.SetVolume(self.volume as f64);
                            }
                            self.preload = Some(slot);
                        }
                        Err(e) => log::warn!("Pre-loading {} failed: {e}", path.display()),
                    }
                }
                None => {
                    if let Some(p) = self.preload.take() {
                        p.shutdown();
                    }
                }
            },
            Cmd::SetLoop(looping) => {
                self.looping = looping;
                if let Some(s) = &self.current {
                    unsafe {
                        let _ = s.engine.SetLoop(looping);
                    }
                }
            }
            Cmd::Play => {
                self.paused = false;
                if let Some(s) = &self.current {
                    unsafe {
                        let _ = s.engine.Play();
                    }
                }
            }
            Cmd::Pause => {
                self.paused = true;
                if let Some(s) = &self.current {
                    unsafe {
                        let _ = s.engine.Pause();
                    }
                }
            }
            Cmd::Stop => {
                self.paused = false;
                if let Some(s) = self.current.take() {
                    s.shutdown();
                }
                if let Some(s) = self.preload.take() {
                    s.shutdown();
                }
                self.frame = None;
                self.draw_fullscreen();
            }
            Cmd::Seek(secs) => {
                if let Some(s) = &self.current {
                    unsafe {
                        let _ = s.engine.SetCurrentTime(secs.max(0.0));
                    }
                    self.force_preview = true;
                }
            }
            Cmd::Volume(v) => {
                self.volume = v.clamp(0.0, 1.0);
                for s in [&self.current, &self.preload].into_iter().flatten() {
                    unsafe {
                        let _ = s.engine.SetVolume(self.volume as f64);
                    }
                }
            }
            Cmd::Fit(fit) => {
                self.fit = fit;
                if let Some(s) = &mut self.current {
                    s.fit = fit;
                }
                self.draw_fullscreen();
            }
            Cmd::Fullscreen(Some(monitor)) => {
                if self
                    .fullscreen
                    .as_ref()
                    .is_some_and(|f| f.monitor == monitor)
                {
                    return;
                }
                self.exit_fullscreen(false);
                self.enter_fullscreen(monitor);
            }
            Cmd::Fullscreen(None) => self.exit_fullscreen(false),
            Cmd::PreviewSize(w, h) => {
                let max = (w.min(1920), h.min(1080));
                if max != self.preview_max {
                    self.preview_max = max;
                    self.force_preview = true;
                }
            }
        }
    }

    fn open_slot(
        &self,
        path: &std::path::Path,
        generation: u64,
        fit: FitMode,
        autoplay: bool,
    ) -> Result<Slot, String> {
        unsafe {
            let notify: IMFMediaEngineNotify = Notify {
                tx: self.events_tx.clone(),
                thread_id: self.thread_id,
                generation,
            }
            .into();
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 3).map_err(|e| e.to_string())?;
            let attrs = attrs.ok_or("no attributes")?;
            attrs
                .SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &self.gpu.manager)
                .map_err(|e| e.to_string())?;
            attrs
                .SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)
                .map_err(|e| e.to_string())?;
            attrs
                .SetUINT32(
                    &MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
                    DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32,
                )
                .map_err(|e| e.to_string())?;
            let engine = self
                .gpu
                .engine_factory
                .CreateInstance(0, &attrs)
                .map_err(|e| format!("Cannot create video engine: {e}"))?;
            let _ = engine.SetAutoPlay(autoplay);
            let _ = engine.SetVolume(self.volume as f64);
            let source = path.to_string_lossy().into_owned();
            if let Err(e) = engine.SetSource(&BSTR::from(source.as_str())) {
                let _ = engine.Shutdown();
                return Err(format!("Cannot open video: {}", describe_hresult(e.code())));
            }
            Ok(Slot {
                engine,
                generation,
                path: path.to_path_buf(),
                fit,
                opened: Instant::now(),
                loaded: false,
                has_video: false,
                first_frame: false,
            })
        }
    }

    fn report_error(&mut self, generation: u64, message: String) {
        log::warn!("Video error: {message}");
        let mut s = lock(&self.shared.status);
        if s.generation == generation {
            s.error = Some(message);
        }
        drop(s);
        self.ctx.request_repaint();
    }

    // ------------------------------------------------------------------ events

    fn engine_events(&mut self) {
        while let Ok((generation, event, _p1, _p2)) = self.events_rx.try_recv() {
            let event = MF_MEDIA_ENGINE_EVENT(event as i32);
            if self
                .current
                .as_ref()
                .is_some_and(|s| s.generation == generation)
            {
                self.current_event(event);
            } else if self
                .preload
                .as_ref()
                .is_some_and(|s| s.generation == generation)
            {
                match event {
                    MF_MEDIA_ENGINE_EVENT_ERROR => {
                        let p = self.preload.take().unwrap();
                        log::warn!("Pre-load of {} failed", p.path.display());
                        p.shutdown();
                    }
                    MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA => {
                        if let Some(p) = &mut self.preload {
                            p.loaded = true;
                        }
                    }
                    _ => {}
                }
            }
            // Anything else is a late event from an engine that was already replaced.
        }
    }

    fn current_event(&mut self, event: MF_MEDIA_ENGINE_EVENT) {
        let Some(slot) = &mut self.current else {
            return;
        };
        match event {
            MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA => unsafe {
                let (mut w, mut h) = (0u32, 0u32);
                let _ = slot.engine.GetNativeVideoSize(Some(&mut w), Some(&mut h));
                slot.loaded = true;
                slot.has_video = w > 0 && h > 0;
                let mut s = lock(&self.shared.status);
                s.loaded = true;
                s.has_video = slot.has_video;
            },
            MF_MEDIA_ENGINE_EVENT_CANPLAY => {
                // Autoplay is on, but start explicitly as well: after ENDED some engines
                // are neither paused nor playing and would otherwise sit still.
                if !self.paused {
                    unsafe {
                        let _ = slot.engine.Play();
                    }
                }
            }
            MF_MEDIA_ENGINE_EVENT_ENDED => {
                if self.looping {
                    // The engine's own loop normally means ENDED never fires; if it does,
                    // reopen the same file under the same generation.
                    let (path, generation, fit) = (slot.path.clone(), slot.generation, slot.fit);
                    if let Some(old) = self.current.take() {
                        old.shutdown();
                    }
                    match self.open_slot(&path, generation, fit, true) {
                        Ok(s) => {
                            unsafe {
                                let _ = s.engine.SetLoop(true);
                            }
                            self.current = Some(s);
                        }
                        Err(e) => self.report_error(generation, e),
                    }
                } else if self.preload.is_some() {
                    self.switch_to_preload();
                } else {
                    lock(&self.shared.status).ended = true;
                    self.ctx.request_repaint();
                }
            }
            MF_MEDIA_ENGINE_EVENT_ERROR => {
                let msg = unsafe { engine_error(&slot.engine) };
                let generation = slot.generation;
                self.report_error(generation, msg);
            }
            _ => {}
        }
    }

    /// Gapless hand-over: the pre-loaded engine is already prepared, so it starts at once
    /// and the last frame of the old file stays on screen until its first frame arrives.
    fn switch_to_preload(&mut self) {
        let Some(next) = self.preload.take() else {
            return;
        };
        unsafe {
            let _ = next.engine.SetLoop(self.looping);
            let _ = next.engine.SetVolume(self.volume as f64);
            let _ = next.engine.Play();
        }
        self.paused = false;
        self.fit = next.fit;
        {
            let mut s = lock(&self.shared.status);
            let fullscreen = s.fullscreen;
            *s = super::Status {
                generation: next.generation,
                loaded: next.loaded,
                fullscreen,
                ..Default::default()
            };
        }
        if let Some(old) = self.current.replace(next) {
            old.shutdown();
        }
        self.ctx.request_repaint();
    }

    fn check_stalled(&mut self) {
        let Some(slot) = &self.current else { return };
        let waiting_for_picture = !slot.first_frame && (!slot.loaded || slot.has_video);
        if !self.paused && waiting_for_picture && slot.opened.elapsed() > NO_PICTURE_LIMIT {
            let generation = slot.generation;
            let name = slot
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Some(s) = self.current.take() {
                s.shutdown();
            }
            self.report_error(
                generation,
                format!(
                    "No picture from \"{name}\" after {} s",
                    NO_PICTURE_LIMIT.as_secs()
                ),
            );
        }
    }

    // ------------------------------------------------------------------ frames

    fn frame_tick(&mut self) {
        let Some(slot) = &mut self.current else {
            return;
        };
        let mut pts = 0i64;
        let hr = unsafe {
            (Interface::vtable(&slot.engine).OnVideoStreamTick)(
                Interface::as_raw(&slot.engine),
                &mut pts,
            )
        };
        let new_frame = hr == S_OK;
        if !new_frame && !self.force_preview {
            return;
        }
        let (mut w, mut h, mut ax, mut ay) = (0u32, 0u32, 0u32, 0u32);
        unsafe {
            let _ = slot.engine.GetNativeVideoSize(Some(&mut w), Some(&mut h));
            let _ = slot
                .engine
                .GetVideoAspectRatio(Some(&mut ax), Some(&mut ay));
        }
        if w == 0 || h == 0 {
            return;
        }
        if new_frame {
            slot.first_frame = true;
        }
        let display = display_size(w, h, ax, ay);
        lock(&self.shared.status).display_size = display;
        let engine = slot.engine.clone();

        if self.fullscreen.is_some() {
            if self.transfer_frame(&engine, display) {
                self.draw_fullscreen();
            }
        } else if self.preview_max.0 > 0 && lock(&self.shared.frame).is_none() {
            let size = fit_within(display, self.preview_max);
            if let Some(image) = self.read_preview(&engine, size) {
                *lock(&self.shared.frame) = Some(image);
                self.force_preview = false;
                self.ctx.request_repaint();
            }
        }
    }

    fn transfer_frame(&mut self, engine: &IMFMediaEngine, (w, h): (u32, u32)) -> bool {
        if !self.frame.as_ref().is_some_and(|f| f.w == w && f.h == h) {
            let made = (|| -> windows::core::Result<FrameTex> {
                let tex = self.gpu.texture(
                    w,
                    h,
                    (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                    D3D11_USAGE_DEFAULT,
                    0,
                )?;
                let mut srv = None;
                unsafe {
                    self.gpu
                        .device
                        .CreateShaderResourceView(&tex, None, Some(&mut srv))?
                };
                Ok(FrameTex {
                    tex,
                    srv: srv.unwrap(),
                    w,
                    h,
                })
            })();
            match made {
                Ok(f) => self.frame = Some(f),
                Err(e) => {
                    log::error!("Frame texture: {e}");
                    return false;
                }
            }
        }
        let frame = self.frame.as_ref().unwrap();
        let dst = RECT {
            left: 0,
            top: 0,
            right: w as i32,
            bottom: h as i32,
        };
        let black = MFARGB {
            rgbBlue: 0,
            rgbGreen: 0,
            rgbRed: 0,
            rgbAlpha: 255,
        };
        unsafe { engine.TransferVideoFrame(&frame.tex, None, &dst, Some(&black)) }.is_ok()
    }

    fn read_preview(&mut self, engine: &IMFMediaEngine, (w, h): (u32, u32)) -> Option<ColorImage> {
        if !self.preview.as_ref().is_some_and(|p| p.w == w && p.h == h) {
            let target = self
                .gpu
                .texture(
                    w,
                    h,
                    D3D11_BIND_RENDER_TARGET.0 as u32,
                    D3D11_USAGE_DEFAULT,
                    0,
                )
                .ok()?;
            let staging = self
                .gpu
                .texture(w, h, 0, D3D11_USAGE_STAGING, D3D11_CPU_ACCESS_READ.0 as u32)
                .ok()?;
            self.preview = Some(Preview {
                target,
                staging,
                w,
                h,
            });
        }
        let p = self.preview.as_ref().unwrap();
        let dst = RECT {
            left: 0,
            top: 0,
            right: w as i32,
            bottom: h as i32,
        };
        let black = MFARGB {
            rgbBlue: 0,
            rgbGreen: 0,
            rgbRed: 0,
            rgbAlpha: 255,
        };
        unsafe {
            engine
                .TransferVideoFrame(&p.target, None, &dst, Some(&black))
                .ok()?;
            self.gpu.ctx.CopyResource(&p.staging, &p.target);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.gpu
                .ctx
                .Map(&p.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .ok()?;
            let mut pixels = Vec::with_capacity((w * h) as usize);
            for y in 0..h as usize {
                let row = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                    w as usize * 4,
                );
                pixels.extend(
                    row.chunks_exact(4)
                        .map(|bgra| Color32::from_rgb(bgra[2], bgra[1], bgra[0])),
                );
            }
            self.gpu.ctx.Unmap(&p.staging, 0);
            Some(ColorImage {
                size: [w as usize, h as usize],
                pixels,
            })
        }
    }

    fn draw_fullscreen(&self) {
        let Some(fs) = &self.fullscreen else { return };
        let ctx = &self.gpu.ctx;
        unsafe {
            ctx.ClearRenderTargetView(&fs.rtv, &[0.0, 0.0, 0.0, 1.0]);
            if let (Some(frame), true) = (&self.frame, self.current.is_some()) {
                let (vp, uv) = self
                    .fit
                    .placement(frame.w as f32 / frame.h as f32, fs.w as f32 / fs.h as f32);
                let (sw, sh) = (fs.w as f32, fs.h as f32);
                ctx.UpdateSubresource(&self.gpu.cbuffer, 0, None, uv.as_ptr().cast(), 0, 0);
                ctx.OMSetRenderTargets(Some(&[Some(fs.rtv.clone())]), None);
                ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                    TopLeftX: (vp[0] * sw).round(),
                    TopLeftY: (vp[1] * sh).round(),
                    Width: (vp[2] * sw).round(),
                    Height: (vp[3] * sh).round(),
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                }]));
                ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                ctx.IASetInputLayout(None);
                ctx.VSSetShader(&self.gpu.vs, None);
                ctx.PSSetShader(&self.gpu.ps, None);
                ctx.VSSetConstantBuffers(0, Some(&[Some(self.gpu.cbuffer.clone())]));
                ctx.PSSetShaderResources(0, Some(&[Some(frame.srv.clone())]));
                ctx.PSSetSamplers(0, Some(&[Some(self.gpu.sampler.clone())]));
                ctx.Draw(3, 0);
                ctx.PSSetShaderResources(0, Some(&[None]));
            }
            let _ = fs.swapchain.Present(1, DXGI_PRESENT(0));
        }
    }

    fn tick_fullscreen_hint(&mut self) {
        let Some(fs) = &mut self.fullscreen else {
            return;
        };
        let Some(hint) = &fs.exit_hint else {
            return;
        };
        if !tick_exit_hint(hint) {
            if let Some(h) = fs.exit_hint.take() {
                destroy_exit_hint(h);
            }
        }
    }

    // -------------------------------------------------------------- fullscreen

    fn enter_fullscreen(&mut self, monitor: usize) {
        let mons = monitors();
        let Some((rect_ref, _, _)) = mons.get(monitor).or_else(|| mons.first()) else {
            // No display yet (just after boot): try again shortly.
            self.fullscreen_pending = Some((monitor, Instant::now()));
            return;
        };
        let rect = *rect_ref;
        let (w, h) = (
            (rect.right - rect.left) as u32,
            (rect.bottom - rect.top) as u32,
        );
        let made = unsafe {
            (|| -> Result<Fullscreen, String> {
                let hwnd = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_APPWINDOW,
                    CLASS_NAME,
                    w!("A-Player"),
                    WS_POPUP | WS_VISIBLE,
                    rect.left,
                    rect.top,
                    w as i32,
                    h as i32,
                    None,
                    None,
                    Some(self.instance),
                    None,
                )
                .map_err(|e| format!("fullscreen window: {e}"))?;
                let mut desc = DXGI_SWAP_CHAIN_DESC1 {
                    Width: w,
                    Height: h,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                    BufferCount: 2,
                    Scaling: DXGI_SCALING_STRETCH,
                    SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                    AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                    Flags: 0,
                    ..Default::default()
                };
                let swapchain = match self.gpu.factory.CreateSwapChainForHwnd(
                    &self.gpu.device,
                    hwnd,
                    &desc,
                    None,
                    None,
                ) {
                    Ok(s) => s,
                    Err(_) => {
                        desc.SwapEffect = DXGI_SWAP_EFFECT_DISCARD;
                        desc.BufferCount = 1;
                        self.gpu
                            .factory
                            .CreateSwapChainForHwnd(&self.gpu.device, hwnd, &desc, None, None)
                            .map_err(|e| {
                                let _ = DestroyWindow(hwnd);
                                format!("swap chain: {e}")
                            })?
                    }
                };
                let _ = self
                    .gpu
                    .factory
                    .MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
                let backbuffer: ID3D11Texture2D =
                    swapchain.GetBuffer(0).map_err(|e| e.to_string())?;
                let mut rtv = None;
                self.gpu
                    .device
                    .CreateRenderTargetView(&backbuffer, None, Some(&mut rtv))
                    .map_err(|e| e.to_string())?;
                let _ = SetForegroundWindow(hwnd);
                SetTimer(Some(hwnd), TOPMOST_TIMER, 2000, None);
                Ok(Fullscreen {
                    hwnd,
                    swapchain,
                    rtv: rtv.unwrap(),
                    w,
                    h,
                    monitor,
                    exit_hint: None,
                })
            })()
        };
        match made {
            Ok(mut fs) => {
                log::info!("Fullscreen on monitor {monitor} ({w}x{h})");
                let show_hint = self
                    .shared
                    .show_fullscreen_exit_hint
                    .swap(false, Ordering::Relaxed);
                if show_hint {
                    if let Some(hint) = create_exit_hint(self.instance, rect) {
                        fs.exit_hint = Some(hint);
                        lock(&self.shared.ui_events).push(UiEvent::FullscreenExitHintShown);
                        self.ctx.request_repaint();
                    }
                }
                self.fullscreen = Some(fs);
                unsafe {
                    let _ = SetPriorityClass(GetCurrentProcess(), HIGH_PRIORITY_CLASS);
                    timeBeginPeriod(1);
                }
                lock(&self.shared.status).fullscreen = true;
                // Show the current picture straight away (also when paused).
                if let Some(slot) = &self.current {
                    let engine = slot.engine.clone();
                    let display = lock(&self.shared.status).display_size;
                    if display.0 > 0 {
                        self.transfer_frame(&engine, display);
                    }
                }
                self.draw_fullscreen();
            }
            Err(e) => {
                log::error!("Cannot enter fullscreen: {e}");
                lock(&self.shared.status).fullscreen = false;
                lock(&self.shared.ui_events).push(UiEvent::FullscreenExited);
                self.ctx.request_repaint();
            }
        }
    }

    fn exit_fullscreen(&mut self, by_user: bool) {
        self.fullscreen_pending = None;
        let Some(fs) = self.fullscreen.take() else {
            return;
        };
        if let Some(hint) = fs.exit_hint {
            destroy_exit_hint(hint);
        }
        unsafe {
            let _ = KillTimer(Some(fs.hwnd), TOPMOST_TIMER);
            drop(fs.rtv);
            drop(fs.swapchain);
            let _ = DestroyWindow(fs.hwnd);
            let _ = SetPriorityClass(GetCurrentProcess(), NORMAL_PRIORITY_CLASS);
            timeEndPeriod(1);
        }
        lock(&self.shared.status).fullscreen = false;
        if by_user {
            lock(&self.shared.ui_events).push(UiEvent::FullscreenExited);
            // Bring the player window back (it may be minimized, e.g. after auto-start).
            let main = self.shared.main_window.load(Ordering::Relaxed);
            if main != 0 {
                let hwnd = HWND(main as *mut _);
                unsafe {
                    if IsIconic(hwnd).as_bool() {
                        let _ = ShowWindow(hwnd, SW_RESTORE);
                    }
                    let _ = SetForegroundWindow(hwnd);
                }
            }
        }
        self.force_preview = true;
        self.ctx.request_repaint();
    }

    fn window_input(&mut self) {
        let input = INPUT.with(|i| std::mem::take(&mut *i.borrow_mut()));
        if input.double_click {
            log::info!("Fullscreen: double-click, leaving fullscreen");
            self.exit_fullscreen(true);
            return;
        }
        if input.display_changed {
            if let Some(monitor) = self.fullscreen.as_ref().map(|f| f.monitor) {
                log::info!("Display configuration changed; re-creating fullscreen window");
                self.exit_fullscreen(false);
                self.enter_fullscreen(monitor);
            }
        }
        if !input.keys.is_empty() {
            lock(&self.shared.ui_events).extend(input.keys.into_iter().map(UiEvent::Key));
            self.ctx.request_repaint();
        }
        if input.redraw {
            self.draw_fullscreen();
        }
    }

    // ------------------------------------------------------------ status/power

    fn update_status(&mut self) {
        let Some(slot) = &self.current else { return };
        let (position, duration, paused) = unsafe {
            (
                slot.engine.GetCurrentTime(),
                slot.engine.GetDuration(),
                slot.engine.IsPaused().as_bool(),
            )
        };
        let mut s = lock(&self.shared.status);
        if s.generation == slot.generation {
            if position.is_finite() {
                s.position = position.max(0.0);
            }
            if duration.is_finite() && duration > 0.0 {
                s.duration = Some(duration);
            }
            s.paused = paused || self.paused;
        }
    }

    /// Keep the display and system awake while video plays or fullscreen is up.
    fn update_power(&mut self) {
        let want = self.fullscreen.is_some()
            || self
                .current
                .as_ref()
                .is_some_and(|s| s.has_video && !self.paused);
        if want != self.keep_awake {
            self.keep_awake = want;
            unsafe {
                if want {
                    SetThreadExecutionState(
                        ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED,
                    );
                } else {
                    SetThreadExecutionState(ES_CONTINUOUS);
                }
            }
        }
    }
}

unsafe fn engine_error(engine: &IMFMediaEngine) -> String {
    match unsafe { engine.GetError() } {
        Ok(err) => {
            let code = unsafe { err.GetErrorCode() };
            let name = match code {
                1 => "aborted",
                2 => "network error",
                3 => "decode error",
                4 => "format not supported",
                5 => "encrypted",
                _ => "unknown error",
            };
            match unsafe { err.GetExtendedErrorCode() } {
                Ok(()) => format!("Video {name}"),
                Err(e) => format!("Video {name}: {}", describe_hresult(e.code())),
            }
        }
        Err(e) => format!("Video error ({e})"),
    }
}
