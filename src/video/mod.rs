//! Video playback.
//!
//! On Windows, videos are decoded by Media Foundation (the engine behind the Films & TV
//! app) on a dedicated thread, using the GPU. That thread owns everything video-related:
//! the media engines, Direct3D 11, and the fullscreen window. The rest of the app talks
//! to it only through non-blocking commands and a small shared status, so a stuck
//! decoder can never freeze the UI or the playback engine.
//!
//! * Windowed: each new frame is scaled down on the GPU and handed to egui as a texture.
//! * Fullscreen: a borderless, topmost window that shows nothing but the picture
//!   (no title, timeline, subtitles or cursor). Double-click leaves fullscreen.
//!
//! On other platforms the same API exists but every video reports an error.

// The non-Windows stub never reads most of these.
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage};
use serde::{Deserialize, Serialize};

#[cfg(not(windows))]
mod stub;
#[cfg(windows)]
mod win;

/// Video playback is only implemented for Windows (Media Foundation).
pub const AVAILABLE: bool = cfg!(windows);

/// File types handed to the video engine (on Windows).
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "wmv", "asf", "avi", "webm", "ts", "m2ts", "mts", "mpg", "mpeg",
    "3gp", "3g2",
];

pub fn is_video_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.iter().any(|v| v.eq_ignore_ascii_case(e)))
}

/// How the picture is fitted to the screen or the video area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FitMode {
    /// Whole picture, true shape, black bars where needed.
    #[default]
    Fit,
    /// Cover the whole area, cropping the overflowing edges.
    Fill,
    /// Stretch to the area's shape (distorts).
    Stretch,
}

impl FitMode {
    pub const ALL: [FitMode; 3] = [FitMode::Fit, FitMode::Fill, FitMode::Stretch];

    pub fn label(self) -> &'static str {
        match self {
            FitMode::Fit => "Fit (black bars)",
            FitMode::Fill => "Fill (crop edges)",
            FitMode::Stretch => "Stretch",
        }
    }

    /// Where to draw a `video_aspect` picture inside an area of `area_aspect`,
    /// as (viewport, uv) rectangles: `viewport` in 0..1 of the area,
    /// `uv` = the part of the picture shown (x, y, width, height in 0..1).
    pub fn placement(self, video_aspect: f32, area_aspect: f32) -> ([f32; 4], [f32; 4]) {
        const FULL: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
        match self {
            FitMode::Stretch => (FULL, FULL),
            FitMode::Fit => {
                if area_aspect > video_aspect {
                    let w = video_aspect / area_aspect; // pillarbox
                    ([(1.0 - w) / 2.0, 0.0, w, 1.0], FULL)
                } else {
                    let h = area_aspect / video_aspect; // letterbox
                    ([0.0, (1.0 - h) / 2.0, 1.0, h], FULL)
                }
            }
            FitMode::Fill => {
                if area_aspect > video_aspect {
                    let h = video_aspect / area_aspect; // crop top/bottom
                    (FULL, [0.0, (1.0 - h) / 2.0, 1.0, h])
                } else {
                    let w = area_aspect / video_aspect; // crop sides
                    (FULL, [(1.0 - w) / 2.0, 0.0, w, 1.0])
                }
            }
        }
    }
}

/// Keys pressed while the fullscreen window has focus (there is no on-screen display,
/// so these are silent).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullscreenKey {
    PlayPause,
    Next,
    Previous,
    SeekForward,
    SeekBack,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiEvent {
    /// The user double-clicked the fullscreen picture.
    FullscreenExited,
    Key(FullscreenKey),
}

/// What the video thread reports back. Read by the playback engine and the UI.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// Generation of the file currently playing (0 = nothing).
    pub generation: u64,
    pub loaded: bool,
    pub has_video: bool,
    pub position: f64,
    pub duration: Option<f64>,
    pub paused: bool,
    /// The current file reached its end (and nothing was pre-loaded to follow it).
    pub ended: bool,
    pub error: Option<String>,
    /// Picture size after correcting for non-square pixels.
    pub display_size: (u32, u32),
    pub fullscreen: bool,
}

/// Where to continue if the watchdog has to restart the program.
#[derive(Clone, Debug, Default)]
pub struct RestartHint {
    pub next_index: Option<usize>,
    pub fullscreen_monitor: Option<usize>,
}

pub(crate) enum Cmd {
    Open {
        path: PathBuf,
        generation: u64,
        fit: FitMode,
    },
    /// File to start the moment the current one ends (gapless). `None` cancels.
    Preload(Option<(PathBuf, u64, FitMode)>),
    SetLoop(bool),
    Play,
    Pause,
    Stop,
    Seek(f64),
    Volume(f32),
    Fit(FitMode),
    /// `Some(monitor)` enters fullscreen on that monitor, `None` leaves it.
    Fullscreen(Option<usize>),
    /// Largest size wanted for windowed frames, in physical pixels (0 = none).
    PreviewSize(u32, u32),
}

pub(crate) struct Shared {
    pub status: Mutex<Status>,
    /// Latest windowed frame, taken by the UI. The video thread only produces a new
    /// one after the previous one was taken.
    pub frame: Mutex<Option<ColorImage>>,
    pub ui_events: Mutex<Vec<UiEvent>>,
    pub heartbeat_ms: AtomicU64,
    /// Win32 thread id of the video thread, used to wake it up (0 = unknown).
    pub thread_id: AtomicU32,
    pub restart_hint: Mutex<RestartHint>,
    pub started: Instant,
    /// Native handle of the main player window (0 = unknown), restored after fullscreen.
    pub main_window: AtomicUsize,
    /// Restart cleanly when private memory exceeds this many MB (0 = never).
    pub memory_limit_mb: AtomicU64,
    /// Set by the watchdog when the memory limit is exceeded; the engine restarts at
    /// the next item boundary so playback is barely interrupted.
    pub restart_requested: AtomicBool,
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    pub fn beat(&self) {
        self.heartbeat_ms
            .store(self.started.elapsed().as_millis() as u64, Ordering::Relaxed);
    }
}

/// Handle to the video thread. Cheap to call from any thread; never blocks on video work.
pub struct Video {
    tx: Sender<Cmd>,
    shared: Arc<Shared>,
}

/// If the video thread stops responding for this long, the program restarts itself.
const WATCHDOG_LIMIT: Duration = Duration::from_secs(20);

impl Video {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        let shared = Arc::new(Shared {
            status: Mutex::new(Status::default()),
            frame: Mutex::new(None),
            ui_events: Mutex::new(Vec::new()),
            heartbeat_ms: AtomicU64::new(0),
            thread_id: AtomicU32::new(0),
            restart_hint: Mutex::new(RestartHint::default()),
            started: Instant::now(),
            main_window: AtomicUsize::new(0),
            memory_limit_mb: AtomicU64::new(0),
            restart_requested: AtomicBool::new(false),
        });

        let thread_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("video".into())
            .spawn(move || {
                #[cfg(windows)]
                win::run(rx, thread_shared, ctx);
                #[cfg(not(windows))]
                stub::run(rx, thread_shared, ctx);
            })
            .expect("failed to spawn video thread");

        let watch = Arc::clone(&shared);
        thread::Builder::new()
            .name("video-watchdog".into())
            .spawn(move || watchdog(watch))
            .expect("failed to spawn watchdog thread");

        Self { tx, shared }
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
        #[cfg(windows)]
        win::wake(&self.shared);
    }

    pub fn open(&self, path: &Path, generation: u64, fit: FitMode) {
        // Reset immediately so stale "ended"/"error" flags are never seen for the new file.
        {
            let mut s = lock(&self.shared.status);
            let fullscreen = s.fullscreen;
            *s = Status {
                generation,
                fullscreen,
                ..Status::default()
            };
        }
        self.send(Cmd::Open {
            path: path.to_path_buf(),
            generation,
            fit,
        });
    }

    pub fn preload(&self, next: Option<(&Path, u64, FitMode)>) {
        self.send(Cmd::Preload(next.map(|(p, g, f)| (p.to_path_buf(), g, f))));
    }

    pub fn set_loop(&self, looping: bool) {
        self.send(Cmd::SetLoop(looping));
    }

    pub fn play(&self) {
        self.send(Cmd::Play);
    }

    pub fn pause(&self) {
        self.send(Cmd::Pause);
    }

    pub fn stop(&self) {
        {
            let mut s = lock(&self.shared.status);
            let fullscreen = s.fullscreen;
            *s = Status {
                fullscreen,
                ..Status::default()
            };
        }
        *lock(&self.shared.frame) = None;
        self.send(Cmd::Stop);
    }

    pub fn seek(&self, secs: f64) {
        lock(&self.shared.status).position = secs;
        self.send(Cmd::Seek(secs));
    }

    /// Linear amplitude 0..=1 (already curved by the caller).
    pub fn set_volume(&self, volume: f32) {
        self.send(Cmd::Volume(volume));
    }

    pub fn set_fit(&self, fit: FitMode) {
        self.send(Cmd::Fit(fit));
    }

    pub fn set_fullscreen(&self, monitor: Option<usize>) {
        lock(&self.shared.status).fullscreen = monitor.is_some();
        self.send(Cmd::Fullscreen(monitor));
    }

    pub fn set_preview_size(&self, w: u32, h: u32) {
        self.send(Cmd::PreviewSize(w, h));
    }

    pub fn status(&self) -> Status {
        lock(&self.shared.status).clone()
    }

    pub fn take_frame(&self) -> Option<ColorImage> {
        lock(&self.shared.frame).take()
    }

    pub fn take_ui_events(&self) -> Vec<UiEvent> {
        std::mem::take(&mut *lock(&self.shared.ui_events))
    }

    /// The main window's native handle (HWND on Windows), so that leaving fullscreen can
    /// bring the player back even while eframe is not drawing (minimized window).
    pub fn set_main_window(&self, handle: usize) {
        self.shared.main_window.store(handle, Ordering::Relaxed);
    }

    pub fn set_memory_limit_mb(&self, mb: u64) {
        self.shared.memory_limit_mb.store(mb, Ordering::Relaxed);
    }

    /// True once memory use passed the limit: restart at the next item boundary.
    pub fn restart_requested(&self) -> bool {
        self.shared.restart_requested.load(Ordering::Relaxed)
    }

    /// Restart the program now and continue from `hint`.
    ///
    /// On success the process exits after the replacement has been spawned.
    /// On spawn failure, returns `Err` and **keeps running** (see [`try_restart_process`]).
    pub fn restart_now(&self, hint: &RestartHint) -> Result<(), String> {
        try_restart_process(hint)
    }

    pub fn set_restart_hint(&self, hint: RestartHint) {
        #[cfg(windows)]
        win::register_os_restart(&restart_args(&hint));
        *lock(&self.shared.restart_hint) = hint;
    }
}

/// Acquire the Windows named mutex that prevents a second player instance.
///
/// No-op on non-Windows. On Windows, returns `Err` if another instance holds the mutex.
#[cfg(windows)]
pub fn acquire_single_instance() -> Result<(), String> {
    win::acquire_single_instance()
}

#[cfg(not(windows))]
pub fn acquire_single_instance() -> Result<(), String> {
    Ok(())
}

/// Names of the connected monitors, primary first (index 0).
pub fn monitors() -> Vec<String> {
    #[cfg(windows)]
    {
        win::monitors()
            .into_iter()
            .map(|(r, primary)| {
                format!(
                    "{}×{}{}",
                    r.right - r.left,
                    r.bottom - r.top,
                    if primary { " (main)" } else { "" }
                )
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Two jobs, so an unattended screen keeps running for days:
///
/// * If the video thread hangs (a broken file or driver), restart the program and carry
///   on with the next item, in fullscreen again if it was fullscreen.
/// * If memory use passes the configured limit (a leaking codec, say), ask the engine to
///   restart at the next item boundary; if that doesn't happen within a minute,
///   restart anyway.
fn watchdog(shared: Arc<Shared>) {
    let mut last_check = Instant::now();
    let mut stale_checks = 0;
    let mut memory_flagged_at: Option<Instant> = None;
    loop {
        thread::sleep(Duration::from_secs(2));
        // A long gap means the PC was asleep: don't mistake that for a hang.
        let gap = last_check.elapsed();
        last_check = Instant::now();
        if gap > Duration::from_secs(6) {
            stale_checks = 0;
            continue;
        }

        let limit = shared.memory_limit_mb.load(Ordering::Relaxed);
        if limit > 0 {
            if let Some(used) = private_memory_mb() {
                if used > limit && memory_flagged_at.is_none() {
                    log::warn!(
                        "Memory use {used} MB exceeds {limit} MB; restarting at the next item"
                    );
                    shared.restart_requested.store(true, Ordering::Relaxed);
                    memory_flagged_at = Some(Instant::now());
                }
            }
            if memory_flagged_at.is_some_and(|t| t.elapsed() > Duration::from_secs(60)) {
                let hint = lock(&shared.restart_hint).clone();
                log::warn!("No item boundary within a minute; restarting now");
                if let Err(e) = try_restart_process(&hint) {
                    // Keep running; retry after another minute of no boundary.
                    log::error!("Watchdog restart failed; keeping process alive: {e}");
                    memory_flagged_at = Some(Instant::now());
                }
            }
        }

        let now = shared.started.elapsed().as_millis() as u64;
        let beat = shared.heartbeat_ms.load(Ordering::Relaxed);
        if beat == 0 || now.saturating_sub(beat) < WATCHDOG_LIMIT.as_millis() as u64 {
            stale_checks = 0;
            continue;
        }
        stale_checks += 1;
        if stale_checks < 3 {
            continue;
        }
        let hint = lock(&shared.restart_hint).clone();
        log::error!(
            "Video thread unresponsive for {} s; restarting (hint: {hint:?})",
            (now - beat) / 1000
        );
        if let Err(e) = try_restart_process(&hint) {
            // Keep running; require another stretch of missed heartbeats before retrying.
            log::error!("Watchdog restart failed; keeping process alive: {e}");
            stale_checks = 0;
        }
    }
}

/// Command-line arguments that make a new process continue from `hint`.
fn restart_args(hint: &RestartHint) -> Vec<String> {
    let mut args = vec!["--resume".to_owned()];
    if let Some(i) = hint.next_index {
        args.extend(["--start-at".to_owned(), i.to_string()]);
    }
    if let Some(m) = hint.fullscreen_monitor {
        args.extend([
            "--fullscreen".to_owned(),
            "--monitor".to_owned(),
            m.to_string(),
        ]);
    }
    args
}

/// Whether this process should exit after a restart spawn attempt.
///
/// Extracted so unit tests can lock the decision without spawning processes:
/// exit **only** when the replacement was spawned successfully.
pub(crate) fn exit_after_restart_spawn(spawn_succeeded: bool) -> bool {
    spawn_succeeded
}

/// Start a fresh copy of the player that continues from `hint`.
///
/// # Spawn-failure behavior
/// * **Spawn succeeds** → this process exits with code 3 (does not return).
/// * **Spawn fails** (or `current_exe` cannot be resolved) → logs the error and
///   returns `Err`. The current process **keeps running** so playback/watchdog
///   can continue; callers must not assume the process has ended.
///
/// On Windows, the single-instance mutex is released before spawn so the
/// replacement can acquire it; if spawn fails the mutex is re-acquired.
fn try_restart_process(hint: &RestartHint) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| {
        let msg = format!("Restart aborted: cannot resolve current executable: {e}");
        log::error!("{msg}");
        msg
    })?;

    #[cfg(windows)]
    win::release_single_instance_for_restart();

    let mut cmd = std::process::Command::new(&exe);
    cmd.args(restart_args(hint));
    match cmd.spawn() {
        Ok(_child) if exit_after_restart_spawn(true) => {
            // Replacement is running; end this process.
            std::process::exit(3);
        }
        Ok(_) => unreachable!("exit_after_restart_spawn(true) is always true"),
        Err(e) => {
            #[cfg(windows)]
            if let Err(re) = win::reacquire_single_instance() {
                log::error!("Failed to reacquire single-instance mutex after spawn error: {re}");
            }
            let msg = format!("Restart failed (keeping current process): {e}");
            log::error!("{msg}");
            Err(msg)
        }
    }
}

/// Private (committed) memory of this process in MB.
fn private_memory_mb() -> Option<u64> {
    #[cfg(windows)]
    {
        win::private_memory_mb()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Display size of a video frame after correcting for non-square pixels.
///
/// `ax:ay` is what the media engine reports. Depending on the decoder this is either
/// the *pixel* aspect ratio (32:27 for a widescreen DVD) or the *display* aspect ratio
/// (16:9). Both readings are tried and the one closest to a standard screen shape wins.
/// Square pixels keep the native size.
pub fn display_size(w: u32, h: u32, ax: u32, ay: u32) -> (u32, u32) {
    if w == 0 || h == 0 || ax == 0 || ay == 0 || ax == ay {
        return (w.max(1), h.max(1));
    }
    let storage = w as f64 / h as f64;
    let reported = ax as f64 / ay as f64;
    if (storage - reported).abs() / storage < 0.01 {
        return (w, h); // engine reported w:h itself, so pixels are square
    }
    // Standard shapes, most common first; the small weights break ties.
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

/// Largest size with the picture's shape that fits in `max` (never upscales beyond it).
pub fn fit_within(display: (u32, u32), max: (u32, u32)) -> (u32, u32) {
    let (dw, dh) = (display.0.max(1) as f64, display.1.max(1) as f64);
    let scale = (max.0 as f64 / dw).min(max.1 as f64 / dh).min(1.0);
    (
        ((dw * scale).round() as u32).max(2),
        ((dh * scale).round() as u32).max(2),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_ratio_readings() {
        assert_eq!(display_size(1920, 1080, 1, 1), (1920, 1080));
        assert_eq!(display_size(1920, 1080, 16, 9), (1920, 1080));
        assert_eq!(display_size(1080, 1920, 9, 16), (1080, 1920));
        assert_eq!(display_size(640, 480, 0, 0), (640, 480));
        assert_eq!(display_size(720, 480, 32, 27), (853, 480));
        assert_eq!(display_size(720, 480, 16, 9), (853, 480));
        assert_eq!(display_size(720, 480, 8, 9), (640, 480));
        assert_eq!(display_size(720, 480, 4, 3), (640, 480));
        assert_eq!(display_size(720, 576, 64, 45), (1024, 576));
        assert_eq!(display_size(1440, 1080, 16, 9), (1920, 1080));
    }

    #[test]
    fn placement_modes() {
        // 4:3 video on a 16:9 screen.
        let (vp, uv) = FitMode::Fit.placement(4.0 / 3.0, 16.0 / 9.0);
        assert!((vp[2] - 0.75).abs() < 1e-6 && (vp[0] - 0.125).abs() < 1e-6);
        assert_eq!(uv, [0.0, 0.0, 1.0, 1.0]);
        let (vp, uv) = FitMode::Fill.placement(4.0 / 3.0, 16.0 / 9.0);
        assert_eq!(vp, [0.0, 0.0, 1.0, 1.0]);
        assert!((uv[3] - 0.75).abs() < 1e-6 && (uv[1] - 0.125).abs() < 1e-6);
        // Portrait video on a landscape screen: narrow column.
        let (vp, _) = FitMode::Fit.placement(9.0 / 16.0, 16.0 / 9.0);
        assert!(vp[2] < 0.33);
        assert_eq!(FitMode::Stretch.placement(1.0, 2.0).0, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn video_extensions() {
        assert!(is_video_path(Path::new("a/b/Clip.MKV")));
        assert!(is_video_path(Path::new("x.mp4")));
        assert!(!is_video_path(Path::new("song.flac")));
    }

    #[test]
    fn fit_within_keeps_shape() {
        assert_eq!(fit_within((1920, 1080), (960, 960)), (960, 540));
        assert_eq!(fit_within((640, 480), (1280, 720)), (640, 480));
        assert_eq!(fit_within((1080, 1920), (800, 450)), (253, 450));
    }

    #[test]
    fn restart_exits_only_after_successful_spawn() {
        assert!(exit_after_restart_spawn(true));
        assert!(!exit_after_restart_spawn(false));
    }

    #[test]
    fn restart_args_encode_hint() {
        assert_eq!(restart_args(&RestartHint::default()), vec!["--resume".to_owned()]);
        assert_eq!(
            restart_args(&RestartHint {
                next_index: Some(3),
                fullscreen_monitor: Some(1),
            }),
            vec![
                "--resume".to_owned(),
                "--start-at".to_owned(),
                "3".to_owned(),
                "--fullscreen".to_owned(),
                "--monitor".to_owned(),
                "1".to_owned(),
            ]
        );
    }
}
