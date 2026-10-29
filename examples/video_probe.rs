//! Phase 0 proof of concept: fullscreen video playback with Windows Media Foundation.
//!
//! ```text
//! video-probe.exe [--fit fit|fill|stretch] [--monitor N] <video files | folders | playlist.json>...
//! ```
//!
//! * Plays the given videos fullscreen, borderless and topmost on one monitor, looping
//!   the whole list forever. Nothing is drawn except the video picture.
//! * Double-click exits. (Alt+F4 also works as a safety net.)
//! * Silent test keys (no on-screen display): 1 / 2 / 3 = fit / fill / stretch,
//!   Space = pause / play, N or → = next video.
//! * Writes `video-probe.log` next to the exe: formats that opened or failed (and why),
//!   video size and pixel aspect ratio, CPU use, frame rate and Windows' notification state.

#[cfg(not(windows))]
fn main() {
    eprintln!("video-probe uses Windows Media Foundation and only runs on Windows.");
}

#[cfg(windows)]
fn main() {
    if let Err(e) = win::run() {
        win::log(&format!("FATAL: {e}"));
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod win {
    use std::cell::RefCell;
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

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
    use windows::Win32::UI::Shell::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows_core::implement;

    use super::display_size;

    const VIDEO_EXTENSIONS: &[&str] = &[
        "mp4", "m4v", "mov", "mkv", "wmv", "asf", "avi", "webm", "ts", "m2ts", "mts", "mpg",
        "mpeg", "3gp", "3g2",
    ];
    const WM_ENGINE_EVENT: u32 = WM_APP + 1;
    const TOPMOST_TIMER: usize = 1;

    /// (engine generation, event, param1, param2) as delivered by IMFMediaEngineNotify.
    type EngineEvent = (u64, u32, usize, u32);

    // ------------------------------------------------------------------ logging

    static LOG: Mutex<Option<File>> = Mutex::new(None);

    /// Milliseconds since start, updated every main-loop pass; watched by the watchdog.
    static HEARTBEAT_MS: AtomicU64 = AtomicU64::new(0);
    static NOW_PLAYING: Mutex<String> = Mutex::new(String::new());
    const WATCHDOG_LIMIT: Duration = Duration::from_secs(15);
    const NO_PICTURE_LIMIT: Duration = Duration::from_secs(15);

    /// If the main loop stops responding (e.g. a decoder hangs), log it and exit, so an
    /// unattended screen is never left with a frozen, topmost black window.
    fn start_watchdog(started: Instant) {
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(2));
            let now = started.elapsed().as_millis() as u64;
            let beat = HEARTBEAT_MS.load(Ordering::Relaxed);
            if now.saturating_sub(beat) > WATCHDOG_LIMIT.as_millis() as u64 {
                let current = NOW_PLAYING.lock().map(|s| s.clone()).unwrap_or_default();
                log(&format!(
                    "WATCHDOG: main loop blocked for {} s while playing {current} — exiting",
                    (now - beat) / 1000
                ));
                std::process::exit(3);
            }
        });
    }

    pub fn log(line: &str) {
        let stamp = chrono_like_now();
        let line = format!("[{stamp}] {line}");
        eprintln!("{line}");
        if let Ok(mut guard) = LOG.lock() {
            if let Some(f) = guard.as_mut() {
                let _ = writeln!(f, "{line}");
                let _ = f.flush();
            }
        }
    }

    fn chrono_like_now() -> String {
        let st = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
        )
    }

    fn open_log() {
        let path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("video-probe.log")))
            .unwrap_or_else(|| PathBuf::from("video-probe.log"));
        if let Ok(f) = OpenOptions::new().create(true).append(true).open(&path) {
            *LOG.lock().unwrap() = Some(f);
        }
        log(&format!(
            "===== video-probe started, log: {}",
            path.display()
        ));
    }

    // ------------------------------------------------------------- arguments

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum FitMode {
        Fit,
        Fill,
        Stretch,
    }

    struct Args {
        fit: FitMode,
        monitor: usize,
        files: Vec<PathBuf>,
    }

    fn is_video(p: &Path) -> bool {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| VIDEO_EXTENSIONS.iter().any(|v| v.eq_ignore_ascii_case(e)))
    }

    fn scan(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                scan(&p, out);
            } else if is_video(&p) {
                out.push(p);
            }
        }
    }

    fn parse_args() -> Result<Args, String> {
        let mut fit = FitMode::Fit;
        let mut monitor = 0;
        let mut files = Vec::new();
        let mut it = std::env::args_os().skip(1);
        while let Some(arg) = it.next() {
            match arg.to_str() {
                Some("--fit") => {
                    fit = match it.next().and_then(|v| v.into_string().ok()).as_deref() {
                        Some("fit") => FitMode::Fit,
                        Some("fill") => FitMode::Fill,
                        Some("stretch") => FitMode::Stretch,
                        other => {
                            return Err(format!("--fit expects fit|fill|stretch, got {other:?}"))
                        }
                    }
                }
                Some("--monitor") => {
                    monitor = it
                        .next()
                        .and_then(|v| v.into_string().ok())
                        .and_then(|v| v.parse().ok())
                        .ok_or("--monitor expects a number (0 = primary)")?;
                }
                _ => {
                    let p = PathBuf::from(arg);
                    if p.is_dir() {
                        scan(&p, &mut files);
                    } else if p
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
                    {
                        files.extend(read_playlist(&p)?);
                    } else {
                        files.push(p);
                    }
                }
            }
        }
        if files.is_empty() {
            return Err("usage: video-probe [--fit fit|fill|stretch] [--monitor N] <videos | folder | playlist.json>".into());
        }
        Ok(Args {
            fit,
            monitor,
            files,
        })
    }

    /// Same JSON format as the player's playlists: entries with a "path", or plain strings.
    fn read_playlist(path: &Path) -> Result<Vec<PathBuf>, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let value: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let base = path.parent().unwrap_or(Path::new(""));
        Ok(value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().or_else(|| v.get("path")?.as_str()))
                    .map(|s| {
                        let p = PathBuf::from(s);
                        if p.is_absolute() {
                            p
                        } else {
                            base.join(p)
                        }
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    // ---------------------------------------------------------------- window

    #[derive(Default)]
    struct WindowInput {
        keys: Vec<u16>,
        redraw: bool,
    }

    thread_local! {
        static INPUT: RefCell<WindowInput> = RefCell::new(WindowInput::default());
    }

    extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        unsafe {
            match msg {
                // The only way out: double-click anywhere on the video.
                WM_LBUTTONDBLCLK => {
                    log("double-click: exiting");
                    let _ = DestroyWindow(hwnd);
                    LRESULT(0)
                }
                WM_DESTROY => {
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                // Hide the mouse cursor over the video.
                WM_SETCURSOR => {
                    SetCursor(None);
                    LRESULT(1)
                }
                // Block the screensaver and monitor power-off while we are up.
                WM_SYSCOMMAND
                    if (wparam.0 & 0xFFF0) as u32 == SC_SCREENSAVE
                        || (wparam.0 & 0xFFF0) as u32 == SC_MONITORPOWER =>
                {
                    LRESULT(0)
                }
                WM_TIMER if wparam.0 == TOPMOST_TIMER => {
                    // Re-assert "always on top" in case another window grabbed it.
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
                    INPUT.with(|i| i.borrow_mut().keys.push(wparam.0 as u16));
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

    fn monitors() -> Vec<(RECT, bool)> {
        unsafe extern "system" fn cb(hmon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
            let list = unsafe { &mut *(data.0 as *mut Vec<(RECT, bool)>) };
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if unsafe { GetMonitorInfoW(hmon, &mut info) }.as_bool() {
                list.push((info.rcMonitor, info.dwFlags & MONITORINFOF_PRIMARY != 0));
            }
            TRUE
        }
        let mut list: Vec<(RECT, bool)> = Vec::new();
        unsafe {
            let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut list as *mut _ as isize));
        }
        // Primary first, so "--monitor 0" is always the main screen.
        list.sort_by_key(|(_, primary)| !*primary);
        list
    }

    fn create_window(rect: RECT) -> windows::core::Result<HWND> {
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None)?.into();
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_DBLCLKS | CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                lpszClassName: w!("AudioPlayerVideoProbe"),
                ..Default::default()
            };
            RegisterClassExW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_APPWINDOW,
                w!("AudioPlayerVideoProbe"),
                w!("video-probe"),
                WS_POPUP | WS_VISIBLE,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                None,
                None,
                Some(instance),
                None,
            )?;
            let _ = SetForegroundWindow(hwnd);
            SetTimer(Some(hwnd), TOPMOST_TIMER, 2000, None);
            Ok(hwnd)
        }
    }

    // ------------------------------------------------------- media engine events

    #[implement(IMFMediaEngineNotify)]
    struct Notify {
        tx: Sender<EngineEvent>,
        hwnd: isize,
        /// Which engine instance this callback belongs to; stale events are ignored.
        generation: u64,
    }

    impl IMFMediaEngineNotify_Impl for Notify_Impl {
        fn EventNotify(&self, event: u32, param1: usize, param2: u32) -> windows::core::Result<()> {
            let _ = self.tx.send((self.generation, event, param1, param2));
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.hwnd as *mut _)),
                    WM_ENGINE_EVENT,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
            Ok(())
        }
    }

    // ------------------------------------------------------------------- D3D

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
        let blob = code.unwrap();
        Ok(unsafe {
            std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize())
                .to_vec()
        })
    }

    struct Gfx {
        device: ID3D11Device,
        ctx: ID3D11DeviceContext,
        swapchain: IDXGISwapChain1,
        backbuffer_rtv: ID3D11RenderTargetView,
        width: u32,
        height: u32,
        vs: ID3D11VertexShader,
        ps: ID3D11PixelShader,
        cbuffer: ID3D11Buffer,
        sampler: ID3D11SamplerState,
        /// The current video frame at its correct display size.
        frame: Option<(ID3D11Texture2D, ID3D11ShaderResourceView, u32, u32)>,
    }

    fn create_device() -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext, &'static str)> {
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
                "hardware GPU with video decode support",
            ),
            (
                D3D_DRIVER_TYPE_HARDWARE,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                "hardware GPU WITHOUT video decode support",
            ),
            (
                D3D_DRIVER_TYPE_WARP,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                "software (WARP) renderer",
            ),
        ];
        let mut last = None;
        for (driver, flags, label) in attempts {
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
                Ok(()) => return Ok((device.unwrap(), ctx.unwrap(), label)),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap())
    }

    impl Gfx {
        fn new(
            device: ID3D11Device,
            ctx: ID3D11DeviceContext,
            hwnd: HWND,
            width: u32,
            height: u32,
        ) -> Result<Self, String> {
            unsafe {
                let factory: IDXGIFactory2 =
                    CreateDXGIFactory1().map_err(|e| format!("DXGI factory: {e}"))?;
                let mut desc = DXGI_SWAP_CHAIN_DESC1 {
                    Width: width,
                    Height: height,
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
                let swapchain =
                    match factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None) {
                        Ok(s) => {
                            log("swap chain: flip-discard (2 buffers)");
                            s
                        }
                        Err(e) => {
                            log(&format!(
                            "flip-discard swap chain unavailable ({e}); falling back to blt model"
                        ));
                            desc.SwapEffect = DXGI_SWAP_EFFECT_DISCARD;
                            desc.BufferCount = 1;
                            factory
                                .CreateSwapChainForHwnd(&device, hwnd, &desc, None, None)
                                .map_err(|e| format!("swap chain: {e}"))?
                        }
                    };
                let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);

                let backbuffer: ID3D11Texture2D =
                    swapchain.GetBuffer(0).map_err(|e| e.to_string())?;
                let mut rtv = None;
                device
                    .CreateRenderTargetView(&backbuffer, None, Some(&mut rtv))
                    .map_err(|e| e.to_string())?;

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
                    swapchain,
                    backbuffer_rtv: rtv.unwrap(),
                    width,
                    height,
                    vs: vs.unwrap(),
                    ps: ps.unwrap(),
                    cbuffer: cbuffer.unwrap(),
                    sampler: sampler.unwrap(),
                    frame: None,
                })
            }
        }

        /// (Re)create the intermediate texture the media engine copies frames into.
        fn ensure_frame_texture(&mut self, w: u32, h: u32) -> windows::core::Result<()> {
            if matches!(self.frame, Some((_, _, fw, fh)) if fw == w && fh == h) {
                return Ok(());
            }
            unsafe {
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
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                    ..Default::default()
                };
                let mut tex = None;
                self.device.CreateTexture2D(&desc, None, Some(&mut tex))?;
                let tex = tex.unwrap();
                let mut srv = None;
                self.device
                    .CreateShaderResourceView(&tex, None, Some(&mut srv))?;
                self.frame = Some((tex, srv.unwrap(), w, h));
            }
            Ok(())
        }

        /// Draw the current frame to the screen using the chosen aspect mode, then present.
        fn draw(&self, mode: FitMode) {
            unsafe {
                self.ctx
                    .ClearRenderTargetView(&self.backbuffer_rtv, &[0.0, 0.0, 0.0, 1.0]);
                if let Some((_, srv, fw, fh)) = &self.frame {
                    let (sw, sh) = (self.width as f32, self.height as f32);
                    let video_aspect = *fw as f32 / *fh as f32;
                    let screen_aspect = sw / sh;

                    // Viewport = where on screen; src = which part of the frame (uv offset, scale).
                    let (viewport, src) = match mode {
                        FitMode::Stretch => ((0.0, 0.0, sw, sh), [0.0, 0.0, 1.0, 1.0]),
                        FitMode::Fit => {
                            let (w, h) = if screen_aspect > video_aspect {
                                (sh * video_aspect, sh) // pillarbox: bars left/right
                            } else {
                                (sw, sw / video_aspect) // letterbox: bars top/bottom
                            };
                            (((sw - w) / 2.0, (sh - h) / 2.0, w, h), [0.0, 0.0, 1.0, 1.0])
                        }
                        FitMode::Fill => {
                            // Crop the frame so it covers the whole screen.
                            let src = if screen_aspect > video_aspect {
                                let visible = video_aspect / screen_aspect; // fraction of height shown
                                [0.0, (1.0 - visible) / 2.0, 1.0, visible]
                            } else {
                                let visible = screen_aspect / video_aspect; // fraction of width shown
                                [(1.0 - visible) / 2.0, 0.0, visible, 1.0]
                            };
                            ((0.0, 0.0, sw, sh), src)
                        }
                    };

                    self.ctx
                        .UpdateSubresource(&self.cbuffer, 0, None, src.as_ptr().cast(), 0, 0);
                    self.ctx
                        .OMSetRenderTargets(Some(&[Some(self.backbuffer_rtv.clone())]), None);
                    self.ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                        TopLeftX: viewport.0.round(),
                        TopLeftY: viewport.1.round(),
                        Width: viewport.2.round(),
                        Height: viewport.3.round(),
                        MinDepth: 0.0,
                        MaxDepth: 1.0,
                    }]));
                    self.ctx
                        .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                    self.ctx.IASetInputLayout(None);
                    self.ctx.VSSetShader(&self.vs, None);
                    self.ctx.PSSetShader(&self.ps, None);
                    self.ctx.PSSetShaderResources(0, Some(&[Some(srv.clone())]));
                    self.ctx
                        .PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
                    self.ctx
                        .VSSetConstantBuffers(0, Some(&[Some(self.cbuffer.clone())]));
                    self.ctx.Draw(3, 0);
                    // Unbind so the media engine can write into the texture again.
                    self.ctx.PSSetShaderResources(0, Some(&[None]));
                }
                let _ = self.swapchain.Present(1, DXGI_PRESENT(0));
            }
        }
    }

    // ------------------------------------------------------------- helpers

    fn describe_hresult(hr: HRESULT) -> String {
        let hint = match hr.0 as u32 {
            0xC00D5212 => " — codec not installed (HEVC / VP9 / AV1 need the matching Video Extension from the Microsoft Store)",
            0xC00D36C4 => " — file type not supported by Media Foundation",
            0xC00D36B4 => " — media type not supported",
            0x80070002 | 0x80070003 => " — file not found",
            0x80070005 => " — access denied",
            0xC00D36E6 => " — no decoder for this format",
            _ => "",
        };
        format!("HRESULT 0x{:08X}{hint}", hr.0 as u32)
    }

    fn notification_state() -> String {
        match unsafe { SHQueryUserNotificationState() } {
            Ok(s) => {
                let name = match s {
                    QUNS_NOT_PRESENT => "NOT_PRESENT (screen locked / screensaver)",
                    QUNS_BUSY => "BUSY (fullscreen app detected — banners suppressed)",
                    QUNS_RUNNING_D3D_FULL_SCREEN => "RUNNING_D3D_FULL_SCREEN (banners suppressed)",
                    QUNS_PRESENTATION_MODE => "PRESENTATION_MODE (banners suppressed)",
                    QUNS_ACCEPTS_NOTIFICATIONS => "ACCEPTS_NOTIFICATIONS (banners CAN appear!)",
                    QUNS_QUIET_TIME => "QUIET_TIME (first hour after setup)",
                    QUNS_APP => "APP (Windows Store app fullscreen)",
                    _ => "unknown",
                };
                format!("{} = {name}", s.0)
            }
            Err(e) => format!("query failed: {e}"),
        }
    }

    fn cpu_time() -> Duration {
        unsafe {
            let (mut c, mut e, mut k, mut u) = Default::default();
            if GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u).is_err() {
                return Duration::ZERO;
            }
            let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) * 100;
            Duration::from_nanos(ft(k) + ft(u))
        }
    }

    fn engine_error(engine: &IMFMediaEngine) -> String {
        unsafe {
            match engine.GetError() {
                Ok(err) => {
                    let code = err.GetErrorCode();
                    let name = match code {
                        1 => "ABORTED",
                        2 => "NETWORK",
                        3 => "DECODE",
                        4 => "SRC_NOT_SUPPORTED",
                        5 => "ENCRYPTED",
                        _ => "UNKNOWN",
                    };
                    let ext = match err.GetExtendedErrorCode() {
                        Ok(()) => String::new(),
                        Err(e) => format!(", {}", describe_hresult(e.code())),
                    };
                    format!("{name} ({code}){ext}")
                }
                Err(e) => format!("unknown error ({e})"),
            }
        }
    }

    // ------------------------------------------------------------------- run

    pub fn run() -> Result<(), String> {
        open_log();
        let args = parse_args()?;
        log(&format!(
            "{} file(s), aspect mode {:?}",
            args.files.len(),
            args.fit
        ));
        for (i, f) in args.files.iter().enumerate() {
            log(&format!("  [{i}] {}", f.display()));
        }

        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|e| format!("COM: {e}"))?;
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| format!("MFStartup: {e}"))?;
        }

        let mons = monitors();
        for (i, (r, primary)) in mons.iter().enumerate() {
            log(&format!(
                "monitor {i}: {}x{} at ({},{}){}",
                r.right - r.left,
                r.bottom - r.top,
                r.left,
                r.top,
                if *primary { " primary" } else { "" }
            ));
        }
        let (rect, _) = *mons
            .get(args.monitor)
            .or_else(|| mons.first())
            .ok_or("no monitor found")?;
        let (width, height) = (
            (rect.right - rect.left) as u32,
            (rect.bottom - rect.top) as u32,
        );

        // ---- "highest priority" settings while fullscreen video plays
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED);
            match SetPriorityClass(GetCurrentProcess(), HIGH_PRIORITY_CLASS) {
                Ok(()) => log("process priority: HIGH"),
                Err(e) => log(&format!("could not raise priority: {e}")),
            }
            timeBeginPeriod(1);
        }
        let mut mmcss_index = 0u32;
        let mmcss = unsafe { AvSetMmThreadCharacteristicsW(w!("Playback"), &mut mmcss_index) };
        log(&match &mmcss {
            Ok(_) => "render thread registered with MMCSS \"Playback\"".to_owned(),
            Err(e) => format!("MMCSS registration failed: {e}"),
        });

        let hwnd = create_window(rect).map_err(|e| format!("window: {e}"))?;

        let (device, ctx, label) = create_device().map_err(|e| format!("Direct3D 11: {e}"))?;
        log(&format!("Direct3D 11 device: {label}"));
        unsafe {
            if let Ok(mt) = device.cast::<ID3D11Multithread>() {
                let _ = mt.SetMultithreadProtected(true);
            }
        }
        let mut gfx = Gfx::new(device.clone(), ctx, hwnd, width, height)?;

        // ---- Media Foundation media engine in "frame server" mode
        let (tx, rx): (Sender<EngineEvent>, Receiver<EngineEvent>) = channel();
        let (manager, factory) = unsafe {
            let mut token = 0u32;
            let mut manager = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)
                .map_err(|e| format!("DXGI manager: {e}"))?;
            let manager = manager.unwrap();
            manager
                .ResetDevice(&device, token)
                .map_err(|e| format!("DXGI manager reset: {e}"))?;
            let factory: IMFMediaEngineClassFactory =
                CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_ALL)
                    .map_err(|e| format!("media engine factory: {e}"))?;
            (manager, factory)
        };
        let single_file = args.files.len() == 1;
        // A fresh engine per file: reusing one after ENDED is unreliable on some systems,
        // and Phase 3 (seamless looping) pre-loads the next file in a second engine anyway.
        let make_engine = |generation: u64| -> Result<IMFMediaEngine, String> {
            unsafe {
                let notify: IMFMediaEngineNotify = Notify {
                    tx: tx.clone(),
                    hwnd: hwnd.0 as isize,
                    generation,
                }
                .into();
                let mut attrs = None;
                MFCreateAttributes(&mut attrs, 3).map_err(|e| e.to_string())?;
                let attrs = attrs.unwrap();
                attrs
                    .SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager)
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
                let engine = factory
                    .CreateInstance(0, &attrs)
                    .map_err(|e| format!("media engine: {e}"))?;
                let _ = engine.SetAutoPlay(true);
                let _ = engine.SetVolume(1.0);
                // A single file loops inside the engine; a list is looped by us.
                let _ = engine.SetLoop(single_file);
                Ok(engine)
            }
        };

        let files = args.files;
        let mut mode = args.fit;
        let mut index = 0usize;
        let mut generation = 1u64;
        let mut consecutive_failures = 0usize;
        let mut loaded_at = Instant::now();
        let mut first_frame_logged = false;
        let open = |i: usize, generation: u64| -> Result<IMFMediaEngine, String> {
            let engine = make_engine(generation)?;
            let path = files[i].to_string_lossy().into_owned();
            log(&format!("opening [{i}] {path}"));
            if let Ok(mut now) = NOW_PLAYING.lock() {
                *now = format!("[{i}] {path}");
            }
            if let Err(e) = unsafe { engine.SetSource(&BSTR::from(path.as_str())) } {
                log(&format!(
                    "  SetSource failed: {}",
                    describe_hresult(e.code())
                ));
            }
            Ok(engine)
        };
        let mut engine = open(index, generation)?;

        let started = Instant::now();
        start_watchdog(started);
        let mut notif_logged = false;
        let mut last_notif = String::new();
        let mut stats_at = Instant::now();
        let mut stats_cpu = cpu_time();
        let mut frames = 0u64;
        let cpus = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;

        'outer: loop {
            HEARTBEAT_MS.store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
            // -- window messages
            unsafe {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        break 'outer;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }

            // -- silent test keys
            let (keys, redraw) = INPUT.with(|i| {
                let mut i = i.borrow_mut();
                (std::mem::take(&mut i.keys), std::mem::take(&mut i.redraw))
            });
            let mut need_draw = redraw;
            let mut next = false;
            for k in keys {
                match k {
                    0x31 => mode = FitMode::Fit,
                    0x32 => mode = FitMode::Fill,
                    0x33 => mode = FitMode::Stretch,
                    k if k == VK_SPACE.0 => unsafe {
                        if engine.IsPaused().as_bool() {
                            let _ = engine.Play();
                        } else {
                            let _ = engine.Pause();
                        }
                    },
                    k if k == VK_RIGHT.0 || k == 0x4E => next = true,
                    _ => continue,
                }
                if matches!(k, 0x31..=0x33) {
                    log(&format!("aspect mode -> {mode:?}"));
                    need_draw = true;
                }
            }

            // -- media engine events
            while let Ok((event_generation, event, p1, p2)) = rx.try_recv() {
                if event_generation != generation {
                    continue; // late event from the previous file's engine
                }
                let event = MF_MEDIA_ENGINE_EVENT(event as i32);
                match event {
                    MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA => unsafe {
                        let (mut w, mut h, mut ax, mut ay) = (0u32, 0u32, 0u32, 0u32);
                        let _ = engine.GetNativeVideoSize(Some(&mut w), Some(&mut h));
                        let _ = engine.GetVideoAspectRatio(Some(&mut ax), Some(&mut ay));
                        let (dw, dh) = display_size(w, h, ax, ay);
                        log(&format!(
                            "  metadata: {w}x{h}, aspect ratio reported {ax}:{ay} -> displayed as {dw}x{dh}, duration {:.1}s ({:.0} ms to open)",
                            engine.GetDuration(),
                            loaded_at.elapsed().as_millis()
                        ));
                        if w == 0 || h == 0 {
                            log("  (no video stream — audio only?)");
                        }
                    },
                    MF_MEDIA_ENGINE_EVENT_CANPLAY => unsafe {
                        // Autoplay is set, but start explicitly too: after ENDED some engines
                        // are neither paused nor playing and would otherwise sit still.
                        let _ = engine.Play();
                    },
                    MF_MEDIA_ENGINE_EVENT_ENDED => {
                        log(&format!("  ended [{index}]"));
                        next = true;
                    }
                    MF_MEDIA_ENGINE_EVENT_ERROR => {
                        log(&format!(
                            "  ERROR [{index}]: {} (p1={p1}, p2=0x{p2:08X})",
                            engine_error(&engine)
                        ));
                        consecutive_failures += 1;
                        if consecutive_failures >= files.len() {
                            log("no file in the list could be played — exiting");
                            break 'outer;
                        }
                        next = true;
                    }
                    _ => {}
                }
                // Every event except the very frequent progress / time updates.
                if !matches!(event.0, 2 | 18) {
                    log(&format!("  event {} ({})", event.0, event_name(event.0)));
                }
            }
            if !next && !first_frame_logged && loaded_at.elapsed() > NO_PICTURE_LIMIT {
                log(&format!(
                    "  no picture after {} s — skipping [{index}]",
                    NO_PICTURE_LIMIT.as_secs()
                ));
                consecutive_failures += 1;
                if consecutive_failures >= files.len() {
                    log("no file in the list could be played — exiting");
                    break 'outer;
                }
                next = true;
            }
            if next {
                // Loop all. (A single file normally loops inside the engine and never
                // ends; if the loop is not honoured it is simply reopened.)
                index = (index + 1) % files.len();
                unsafe {
                    let _ = engine.Shutdown();
                }
                generation += 1;
                loaded_at = Instant::now();
                first_frame_logged = false;
                engine = open(index, generation)?;
            }

            // -- new video frame?
            let mut pts = 0i64;
            let hr = unsafe {
                (Interface::vtable(&engine).OnVideoStreamTick)(Interface::as_raw(&engine), &mut pts)
            };
            if hr == S_OK {
                unsafe {
                    let (mut w, mut h, mut ax, mut ay) = (0u32, 0u32, 0u32, 0u32);
                    let _ = engine.GetNativeVideoSize(Some(&mut w), Some(&mut h));
                    let _ = engine.GetVideoAspectRatio(Some(&mut ax), Some(&mut ay));
                    if w > 0 && h > 0 {
                        let (dw, dh) = display_size(w, h, ax, ay);
                        if gfx.ensure_frame_texture(dw, dh).is_ok() {
                            let (tex, ..) = gfx.frame.as_ref().unwrap();
                            let dst = RECT {
                                left: 0,
                                top: 0,
                                right: dw as i32,
                                bottom: dh as i32,
                            };
                            let black = MFARGB {
                                rgbBlue: 0,
                                rgbGreen: 0,
                                rgbRed: 0,
                                rgbAlpha: 255,
                            };
                            match engine.TransferVideoFrame(tex, None, &dst, Some(&black)) {
                                Ok(()) => {
                                    need_draw = true;
                                    frames += 1;
                                    consecutive_failures = 0;
                                    if !first_frame_logged {
                                        first_frame_logged = true;
                                        log(&format!(
                                            "  first frame after {} ms",
                                            loaded_at.elapsed().as_millis()
                                        ));
                                    }
                                }
                                Err(e) => log(&format!(
                                    "  TransferVideoFrame failed: {}",
                                    describe_hresult(e.code())
                                )),
                            }
                        }
                    }
                }
            }
            if need_draw {
                gfx.draw(mode);
            }

            // -- periodic diagnostics
            if !notif_logged && started.elapsed() > Duration::from_secs(3) {
                notif_logged = true;
                last_notif = notification_state();
                log(&format!("notification state: {last_notif}"));
            }
            if stats_at.elapsed() >= Duration::from_secs(10) {
                let wall = stats_at.elapsed().as_secs_f64();
                let cpu = cpu_time();
                let cpu_pct = (cpu - stats_cpu).as_secs_f64() / wall / cpus * 100.0;
                let pos = unsafe { engine.GetCurrentTime() };
                log(&format!(
                    "stats: [{index}] at {pos:.1}s, {:.1} fps shown, CPU {cpu_pct:.1}% (of all cores)",
                    frames as f64 / wall
                ));
                let n = notification_state();
                if n != last_notif {
                    log(&format!("notification state changed: {n}"));
                    last_notif = n;
                }
                stats_at = Instant::now();
                stats_cpu = cpu;
                frames = 0;
            }

            unsafe {
                MsgWaitForMultipleObjects(None, false, 2, QS_ALLINPUT);
            }
        }

        // ---- restore everything
        unsafe {
            let _ = engine.Shutdown();
            let _ = MFShutdown();
            if let Ok(h) = mmcss {
                let _ = AvRevertMmThreadCharacteristics(h);
            }
            timeEndPeriod(1);
            let _ = SetPriorityClass(GetCurrentProcess(), NORMAL_PRIORITY_CLASS);
            SetThreadExecutionState(ES_CONTINUOUS);
        }
        log("exited cleanly");
        Ok(())
    }

    fn event_name(e: i32) -> &'static str {
        match e {
            1 => "LOADSTART",
            3 => "SUSPEND",
            4 => "ABORT",
            5 => "ERROR",
            6 => "EMPTIED",
            7 => "STALLED",
            8 => "PLAY",
            9 => "PAUSE",
            10 => "LOADEDMETADATA",
            11 => "LOADEDDATA",
            12 => "WAITING",
            13 => "PLAYING",
            14 => "CANPLAY",
            15 => "CANPLAYTHROUGH",
            16 => "SEEKING",
            17 => "SEEKED",
            19 => "ENDED",
            20 => "RATECHANGE",
            21 => "DURATIONCHANGE",
            22 => "VOLUMECHANGE",
            1000 => "FORMATCHANGE",
            1005 => "BUFFERINGSTARTED",
            1006 => "BUFFERINGENDED",
            1009 => "FIRSTFRAMEREADY",
            1010 => "TRACKSCHANGE",
            1012 => "RESOURCELOST",
            1014 => "STREAMRENDERINGERROR",
            1016 => "AUDIOENDPOINTCHANGE",
            _ => "other",
        }
    }
}

/// Display size of a video frame after correcting for non-square pixels.
///
/// `ax:ay` is what the media engine reports. Depending on the decoder this is either
/// the *pixel* aspect ratio (32:27 for a widescreen DVD) or the *display* aspect ratio
/// (16:9). Both readings are tried and the one that lands closest to a standard screen
/// shape wins. Square pixels keep the native size.
#[cfg_attr(not(windows), allow(dead_code))]
fn display_size(w: u32, h: u32, ax: u32, ay: u32) -> (u32, u32) {
    if w == 0 || h == 0 || ax == 0 || ay == 0 || ax == ay {
        return (w.max(1), h.max(1));
    }
    let storage = w as f64 / h as f64;
    let reported = ax as f64 / ay as f64;
    if (storage - reported).abs() / storage < 0.01 {
        return (w, h); // engine reported w:h itself, so pixels are square
    }

    // Standard shapes, most common first; the small weights break ties between two
    // readings that are both "standard" (e.g. 1440x1080 HDV: 16:9 beats 4:3).
    const COMMON: [(f64, f64); 12] = [
        (16.0 / 9.0, 0.0),
        (4.0 / 3.0, 0.001),
        (2.39, 0.002),
        (1.85, 0.002),
        (2.35, 0.002),
        (1.6, 0.003),
        (7.0 / 3.0, 0.003),
        (1.5, 0.004),
        (1.25, 0.004),
        (1.0, 0.004),
        (2.0, 0.005),
        (2.2, 0.005),
    ];
    // Distance (in log space) from the nearest standard shape, portrait or landscape.
    let off_standard = |a: f64| {
        COMMON
            .iter()
            .flat_map(|&(c, weight)| [(c, weight), (1.0 / c, weight)])
            .map(|(c, weight)| (a / c).ln().abs() + weight)
            .fold(f64::INFINITY, f64::min)
    };
    let as_pixel_ratio = storage * reported;
    let as_display_ratio = reported;
    let aspect = if off_standard(as_display_ratio) < off_standard(as_pixel_ratio) {
        as_display_ratio
    } else {
        as_pixel_ratio
    };
    (((h as f64) * aspect).round().max(1.0) as u32, h)
}

#[cfg(test)]
mod tests {
    use super::display_size;

    #[test]
    fn aspect_ratio_readings() {
        // Square pixels, reported either as 1:1 or as the frame's own shape.
        assert_eq!(display_size(1920, 1080, 1, 1), (1920, 1080));
        assert_eq!(display_size(1920, 1080, 16, 9), (1920, 1080));
        assert_eq!(display_size(1080, 1920, 9, 16), (1080, 1920));
        assert_eq!(display_size(640, 480, 0, 0), (640, 480));
        // Widescreen NTSC DVD: pixel ratio 32:27 or display ratio 16:9 -> 853x480.
        assert_eq!(display_size(720, 480, 32, 27), (853, 480));
        assert_eq!(display_size(720, 480, 16, 9), (853, 480));
        // 4:3 NTSC DVD: pixel ratio 8:9 or display ratio 4:3 -> 640x480.
        assert_eq!(display_size(720, 480, 8, 9), (640, 480));
        assert_eq!(display_size(720, 480, 4, 3), (640, 480));
        // Widescreen PAL DVD (720x576, pixel 64:45) and HDV reported as 16:9.
        // (HDV reported as pixel ratio 4:3 is indistinguishable from "1440x1080 is
        // already 4:3"; the Phase 0 log on real Windows settles which one it reports.)
        assert_eq!(display_size(720, 576, 64, 45), (1024, 576));
        assert_eq!(display_size(1440, 1080, 16, 9), (1920, 1080));
    }
}
