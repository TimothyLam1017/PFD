//! Non-Windows video thread via system libmpv (dynloaded).
//!
//! Windowed preview: software render (`vo=libmpv`) → egui `ColorImage`.
//! Fullscreen: mpv's own borderless/ontop window (`force-window` + `fullscreen`);
//! double-click is bound to a script-message that exits fullscreen (Esc does not).
//!
//! Gapless: a second idle mpv handle preloads the next file (best-effort). There may
//! still be a short gap versus Windows Media Foundation's dual-engine switch.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, ColorImage};

use super::mpv_ffi::{
    self, Handle, MpvLib, MPV_END_FILE_REASON_EOF, MPV_END_FILE_REASON_ERROR, MPV_EVENT_CLIENT_MESSAGE,
    MPV_EVENT_END_FILE, MPV_EVENT_FILE_LOADED, MPV_EVENT_NONE, MPV_EVENT_PROPERTY_CHANGE,
    MPV_EVENT_SHUTDOWN, MPV_EVENT_VIDEO_RECONFIG, MPV_FORMAT_DOUBLE, MPV_FORMAT_FLAG,
};
use super::{fit_within, lock, Cmd, FitMode, FullscreenKey, Shared, UiEvent};

const NO_PICTURE_LIMIT: Duration = Duration::from_secs(20);
const FS_EXIT_MSG: &str = "pfd-exit-fs";

struct Slot {
    handle: Handle,
    generation: u64,
    path: PathBuf,
    fit: FitMode,
    opened: Instant,
    loaded: bool,
    has_video: bool,
    first_frame: bool,
    /// True when this slot owns the on-screen fullscreen window.
    shows_window: bool,
}

impl Slot {
    fn apply_fit(&self) {
        let _ = match self.fit {
            FitMode::Fit => {
                let _ = self.handle.set_prop_flag("keepaspect", true);
                self.handle.set_prop_f64("panscan", 0.0)
            }
            FitMode::Fill => {
                let _ = self.handle.set_prop_flag("keepaspect", true);
                self.handle.set_prop_f64("panscan", 1.0)
            }
            FitMode::Stretch => {
                let _ = self.handle.set_prop_f64("panscan", 0.0);
                self.handle.set_prop_flag("keepaspect", false)
            }
        };
    }

    fn set_volume(&self, linear: f32) {
        // mpv volume is 0..=100 perceptual-ish; map linear amplitude roughly.
        let v = (linear.clamp(0.0, 1.0) * 100.0) as f64;
        let _ = self.handle.set_prop_f64("volume", v);
        let _ = self.handle.set_prop_flag("mute", linear <= 0.0001);
    }
}

struct VideoThread {
    shared: Arc<Shared>,
    ctx: egui::Context,
    lib: Arc<MpvLib>,

    current: Option<Slot>,
    preload: Option<Slot>,
    looping: bool,
    paused: bool,
    volume: f32,
    fit: FitMode,
    preview_max: (u32, u32),
    fullscreen_monitor: Option<usize>,
    /// Last time we pushed a windowed frame (throttle SW render).
    last_frame_at: Instant,
    pixels: Vec<u8>,
}

pub(super) fn run(rx: Receiver<Cmd>, shared: Arc<Shared>, ctx: egui::Context) {
    shared.beat();
    let lib = match MpvLib::load() {
        Ok(lib) => lib,
        Err(e) => {
            log::error!("Video unavailable: {e}");
            return failed_loop(rx, shared, ctx, e);
        }
    };
    let mut vt = VideoThread {
        shared,
        ctx,
        lib,
        current: None,
        preload: None,
        looping: false,
        paused: false,
        volume: 0.5,
        fit: FitMode::default(),
        preview_max: (0, 0),
        fullscreen_monitor: None,
        last_frame_at: Instant::now() - Duration::from_secs(1),
        pixels: Vec::new(),
    };
    vt.run(rx);
}

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

fn configure_base(h: &Handle, for_window: bool) -> Result<(), String> {
    // Keep the player quiet and under our control.
    h.set_option("config", "no")?;
    h.set_option("load-scripts", "no")?;
    h.set_option("ytdl", "no")?;
    h.set_option("terminal", "no")?;
    h.set_option("msg-level", "all=error")?;
    h.set_option("osc", "no")?;
    h.set_option("osd-level", "0")?;
    h.set_option("input-default-bindings", "no")?;
    h.set_option("input-terminal", "no")?;
    h.set_option("idle", "yes")?;
    h.set_option("keep-open", "yes")?;
    h.set_option("keep-open-pause", "no")?;
    h.set_option("hr-seek", "yes")?;
    h.set_option("sub-visibility", "no")?;
    h.set_option("audio-display", "no")?;
    if for_window {
        // Fullscreen / on-screen picture path.
        h.set_option("force-window", "yes")?;
        h.set_option("ontop", "yes")?;
        h.set_option("cursor-autohide", "always")?;
        h.set_option("title", "PFD video")?;
        // Default VO (gpu/x11/wayland) — creates a real window we can fullscreen.
        h.set_option("vo", "gpu")?;
    } else {
        // Windowed egui frames via software render.
        h.set_option("vo", "libmpv")?;
        h.set_option("force-window", "no")?;
        h.set_option("hwdec", "auto")?;
        h.set_option("sw-fast", "yes")?;
    }
    Ok(())
}

fn bind_fs_exit(h: &Handle) {
    // Double-click only; Esc must not exit (product rule).
    let _ = h.command(&[
        "keybind",
        "MOUSE_BTN0_DBL",
        &format!("script-message {FS_EXIT_MSG}"),
    ]);
    let _ = h.command(&["keybind", "ESC", "ignore"]);
    let _ = h.command(&["keybind", "FULLSCREEN", "ignore"]);
}

fn bind_fs_keys(h: &Handle) {
    let _ = h.command(&[
        "keybind",
        "SPACE",
        &format!("script-message pfd-key playpause"),
    ]);
    let _ = h.command(&[
        "keybind",
        "RIGHT",
        &format!("script-message pfd-key seekfwd"),
    ]);
    let _ = h.command(&[
        "keybind",
        "LEFT",
        &format!("script-message pfd-key seekback"),
    ]);
    let _ = h.command(&[
        "keybind",
        "Ctrl+RIGHT",
        &format!("script-message pfd-key next"),
    ]);
    let _ = h.command(&[
        "keybind",
        "Ctrl+LEFT",
        &format!("script-message pfd-key prev"),
    ]);
    let _ = h.command(&["keybind", "n", &format!("script-message pfd-key next")]);
    let _ = h.command(&["keybind", "p", &format!("script-message pfd-key prev")]);
}

impl VideoThread {
    fn open_slot(
        &self,
        path: PathBuf,
        generation: u64,
        fit: FitMode,
        for_window: bool,
        start_paused: bool,
    ) -> Result<Slot, String> {
        let mut handle = Handle::create(Arc::clone(&self.lib))?;
        configure_base(&handle, for_window)?;
        handle.initialize()?;
        if for_window {
            bind_fs_exit(&handle);
            bind_fs_keys(&handle);
        } else {
            handle.create_sw_render()?;
        }
        let _ = handle.observe("time-pos", MPV_FORMAT_DOUBLE);
        let _ = handle.observe("duration", MPV_FORMAT_DOUBLE);
        let _ = handle.observe("pause", MPV_FORMAT_FLAG);
        let _ = handle.observe("eof-reached", MPV_FORMAT_FLAG);

        let path_str = path.to_string_lossy().into_owned();
        handle.command(&["loadfile", &path_str, "replace"])?;
        if start_paused {
            let _ = handle.set_prop_flag("pause", true);
        }
        let slot = Slot {
            handle,
            generation,
            path,
            fit,
            opened: Instant::now(),
            loaded: false,
            has_video: false,
            first_frame: false,
            shows_window: for_window,
        };
        slot.apply_fit();
        slot.set_volume(self.volume);
        if !start_paused && self.paused {
            let _ = slot.handle.set_prop_flag("pause", true);
        } else if !start_paused {
            let _ = slot.handle.set_prop_flag("pause", false);
        }
        Ok(slot)
    }

    fn run(&mut self, rx: Receiver<Cmd>) {
        loop {
            self.shared.beat();

            loop {
                match rx.try_recv() {
                    Ok(cmd) => self.command(cmd),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.current = None;
                        self.preload = None;
                        return;
                    }
                }
            }

            self.poll_events();
            self.refresh_status();
            self.maybe_emit_frame();
            self.check_no_picture();

            // Small sleep so we do not spin; mpv_wait_event uses 0 timeout in poll_events.
            std::thread::sleep(Duration::from_millis(8));
        }
    }

    fn command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Open {
                path,
                generation,
                fit,
            } => {
                self.fit = fit;
                self.preload = None;
                let want_fs = self.fullscreen_monitor.is_some();
                match self.open_slot(path, generation, fit, want_fs, false) {
                    Ok(slot) => {
                        if want_fs {
                            let _ = slot.handle.set_prop_flag("fullscreen", true);
                            self.maybe_show_exit_hint();
                        }
                        self.current = Some(slot);
                        self.publish_loading(generation);
                    }
                    Err(e) => self.publish_error(generation, e),
                }
                self.ctx.request_repaint();
            }
            Cmd::Preload(next) => match next {
                Some((path, generation, fit)) => {
                    // Best-effort second engine (libmpv, paused). Documented vs MF gapless.
                    if let Some(old) = self.preload.take() {
                        drop(old);
                    }
                    match self.open_slot(path, generation, fit, false, true) {
                        Ok(slot) => self.preload = Some(slot),
                        Err(e) => log::warn!("Video preload failed: {e}"),
                    }
                }
                None => {
                    self.preload = None;
                }
            },
            Cmd::SetLoop(looping) => {
                self.looping = looping;
                if let Some(s) = &self.current {
                    let _ = s.handle.set_prop_flag("loop-file", looping);
                }
            }
            Cmd::Play => {
                self.paused = false;
                if let Some(s) = &self.current {
                    let _ = s.handle.set_prop_flag("pause", false);
                }
            }
            Cmd::Pause => {
                self.paused = true;
                if let Some(s) = &self.current {
                    let _ = s.handle.set_prop_flag("pause", true);
                }
            }
            Cmd::Stop => {
                self.current = None;
                self.preload = None;
                *lock(&self.shared.frame) = None;
            }
            Cmd::Seek(secs) => {
                if let Some(s) = &self.current {
                    let _ = s.handle.set_prop_f64("time-pos", secs.max(0.0));
                }
            }
            Cmd::Volume(v) => {
                self.volume = v;
                if let Some(s) = &self.current {
                    s.set_volume(v);
                }
                if let Some(s) = &self.preload {
                    s.set_volume(v);
                }
            }
            Cmd::Fit(fit) => {
                self.fit = fit;
                if let Some(s) = &mut self.current {
                    s.fit = fit;
                    s.apply_fit();
                }
                if let Some(s) = &mut self.preload {
                    s.fit = fit;
                    s.apply_fit();
                }
            }
            Cmd::Fullscreen(Some(monitor)) => {
                self.enter_fullscreen(monitor);
            }
            Cmd::Fullscreen(None) => {
                self.exit_fullscreen(false);
            }
            Cmd::PreviewSize(w, h) => {
                self.preview_max = (w, h);
            }
        }
    }

    fn enter_fullscreen(&mut self, monitor: usize) {
        self.fullscreen_monitor = Some(monitor);
        lock(&self.shared.status).fullscreen = true;
        let Some(cur) = self.current.take() else {
            return;
        };
        let pos = cur.handle.get_f64("time-pos").unwrap_or(0.0);
        let gen = cur.generation;
        let path = cur.path.clone();
        let fit = cur.fit;
        let paused = self.paused;
        drop(cur);
        match self.open_slot(path, gen, fit, true, paused) {
            Ok(slot) => {
                let _ = slot.handle.set_prop_flag("fullscreen", true);
                // Monitor index: mpv uses fullscreen-screen on some platforms.
                let _ = slot
                    .handle
                    .set_prop_str("fullscreen-screen", &monitor.to_string());
                if pos > 0.05 {
                    let _ = slot.handle.set_prop_f64("time-pos", pos);
                }
                self.current = Some(slot);
                self.maybe_show_exit_hint();
                *lock(&self.shared.frame) = None;
            }
            Err(e) => {
                self.publish_error(gen, e);
                self.fullscreen_monitor = None;
                lock(&self.shared.status).fullscreen = false;
            }
        }
        self.ctx.request_repaint();
    }

    fn exit_fullscreen(&mut self, by_user: bool) {
        let was = self.fullscreen_monitor.take();
        lock(&self.shared.status).fullscreen = false;
        if was.is_none() {
            return;
        }
        let Some(cur) = self.current.take() else {
            if by_user {
                lock(&self.shared.ui_events).push(UiEvent::FullscreenExited);
            }
            return;
        };
        let pos = cur.handle.get_f64("time-pos").unwrap_or(0.0);
        let gen = cur.generation;
        let path = cur.path.clone();
        let fit = cur.fit;
        let paused = self.paused;
        drop(cur);
        match self.open_slot(path, gen, fit, false, paused) {
            Ok(slot) => {
                if pos > 0.05 {
                    let _ = slot.handle.set_prop_f64("time-pos", pos);
                }
                self.current = Some(slot);
            }
            Err(e) => self.publish_error(gen, e),
        }
        if by_user {
            lock(&self.shared.ui_events).push(UiEvent::FullscreenExited);
        }
        self.ctx.request_repaint();
    }

    fn maybe_show_exit_hint(&self) {
        if self
            .shared
            .show_fullscreen_exit_hint
            .swap(false, Ordering::Relaxed)
        {
            // No native OSD hint window on Linux/mac yet; still clear the config flag
            // so the UX toggle behaves, and log once.
            log::info!("Fullscreen: double-click the video to exit (Esc does nothing)");
            lock(&self.shared.ui_events).push(UiEvent::FullscreenExitHintShown);
            self.ctx.request_repaint();
        }
    }

    fn poll_events(&mut self) {
        // Drain current then preload.
        self.drain_handle_events(true);
        self.drain_handle_events(false);
    }

    fn drain_handle_events(&mut self, current: bool) {
        loop {
            let handle_ptr = {
                let slot = if current {
                    self.current.as_ref()
                } else {
                    self.preload.as_ref()
                };
                match slot {
                    Some(s) => s.handle.ptr,
                    None => return,
                }
            };
            let lib = &self.lib;
            let ev = unsafe { (lib.wait_event)(handle_ptr, 0.0) };
            if ev.is_null() {
                return;
            }
            let event = unsafe { &*ev };
            match event.event_id {
                MPV_EVENT_NONE => return,
                MPV_EVENT_SHUTDOWN => return,
                MPV_EVENT_FILE_LOADED | MPV_EVENT_VIDEO_RECONFIG => {
                    if current {
                        if let Some(s) = self.current.as_mut() {
                            s.loaded = true;
                            let w = s.handle.get_i64("width").unwrap_or(0) as u32;
                            let h = s.handle.get_i64("height").unwrap_or(0) as u32;
                            s.has_video = w > 0 && h > 0;
                            if s.has_video {
                                s.first_frame = true;
                            }
                        }
                    } else if let Some(s) = self.preload.as_mut() {
                        s.loaded = true;
                    }
                }
                MPV_EVENT_END_FILE => {
                    if !current {
                        continue;
                    }
                    let reason = if event.data.is_null() {
                        MPV_END_FILE_REASON_EOF
                    } else {
                        unsafe { (*(event.data as *const mpv_ffi::mpv_event_end_file)).reason }
                    };
                    if reason == MPV_END_FILE_REASON_ERROR {
                        let gen = self.current.as_ref().map(|s| s.generation).unwrap_or(0);
                        self.publish_error(gen, "libmpv failed to play this file".into());
                    } else if reason == MPV_END_FILE_REASON_EOF {
                        if self.looping {
                            if let Some(s) = &self.current {
                                let _ = s.handle.set_prop_f64("time-pos", 0.0);
                                let _ = s.handle.set_prop_flag("pause", false);
                            }
                        } else if self.preload.is_some() {
                            self.switch_to_preload();
                        } else {
                            let mut st = lock(&self.shared.status);
                            st.ended = true;
                            st.paused = true;
                        }
                    }
                    self.ctx.request_repaint();
                }
                MPV_EVENT_CLIENT_MESSAGE => {
                    if !current || event.data.is_null() {
                        continue;
                    }
                    let msg = unsafe { &*(event.data as *const mpv_ffi::mpv_event_client_message) };
                    if msg.num_args <= 0 || msg.args.is_null() {
                        continue;
                    }
                    let args: Vec<String> = (0..msg.num_args)
                        .filter_map(|i| {
                            let p = unsafe { *msg.args.offset(i as isize) };
                            if p.is_null() {
                                None
                            } else {
                                Some(unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned())
                            }
                        })
                        .collect();
                    if args.first().map(|s| s.as_str()) == Some(FS_EXIT_MSG)
                        || (args.len() >= 2 && args[0] == "pfd-exit-fs")
                        || args.iter().any(|a| a == FS_EXIT_MSG)
                    {
                        self.exit_fullscreen(true);
                    } else if args.first().map(|s| s.as_str()) == Some("pfd-key") {
                        if let Some(key) = args.get(1) {
                            let k = match key.as_str() {
                                "playpause" => Some(FullscreenKey::PlayPause),
                                "seekfwd" => Some(FullscreenKey::SeekForward),
                                "seekback" => Some(FullscreenKey::SeekBack),
                                "next" => Some(FullscreenKey::Next),
                                "prev" => Some(FullscreenKey::Previous),
                                _ => None,
                            };
                            if let Some(k) = k {
                                lock(&self.shared.ui_events).push(UiEvent::Key(k));
                                self.ctx.request_repaint();
                            }
                        }
                    }
                }
                MPV_EVENT_PROPERTY_CHANGE => {
                    // refresh_status reads properties; just wake UI occasionally.
                    self.ctx.request_repaint();
                }
                _ => {}
            }
        }
    }

    fn switch_to_preload(&mut self) {
        let Some(next) = self.preload.take() else {
            return;
        };
        let want_fs = self.fullscreen_monitor.is_some();
        if want_fs != next.shows_window {
            // Rebuild in the correct display mode.
            let gen = next.generation;
            let path = next.path.clone();
            let fit = next.fit;
            drop(next);
            match self.open_slot(path, gen, fit, want_fs, false) {
                Ok(slot) => {
                    if want_fs {
                        let _ = slot.handle.set_prop_flag("fullscreen", true);
                    }
                    self.current = Some(slot);
                }
                Err(e) => self.publish_error(gen, e),
            }
        } else {
            let _ = next.handle.set_prop_flag("pause", false);
            self.current = Some(next);
        }
        self.publish_loading(
            self.current
                .as_ref()
                .map(|s| s.generation)
                .unwrap_or(0),
        );
    }

    fn refresh_status(&mut self) {
        let Some(s) = &self.current else {
            return;
        };
        let pos = s.handle.get_f64("time-pos").unwrap_or(0.0);
        let duration = s.handle.get_f64("duration").filter(|d| *d > 0.0);
        let paused = s.handle.get_flag("pause").unwrap_or(self.paused);
        let w = s.handle.get_i64("dwidth").or_else(|| s.handle.get_i64("width")).unwrap_or(0) as u32;
        let h = s.handle.get_i64("dheight").or_else(|| s.handle.get_i64("height")).unwrap_or(0) as u32;
        let display = if w > 0 && h > 0 { (w, h) } else { (0, 0) };
        let mut st = lock(&self.shared.status);
        if st.generation != s.generation {
            return;
        }
        st.loaded = s.loaded;
        st.has_video = s.has_video || display.0 > 0;
        st.position = pos;
        st.duration = duration;
        st.paused = paused;
        st.fullscreen = self.fullscreen_monitor.is_some();
        if display.0 > 0 {
            st.display_size = display;
        }
    }

    fn maybe_emit_frame(&mut self) {
        if self.fullscreen_monitor.is_some() {
            return; // fullscreen uses mpv's own window
        }
        if self.preview_max.0 < 2 || self.preview_max.1 < 2 {
            return;
        }
        // Only produce a new frame once the UI has taken the previous one.
        if lock(&self.shared.frame).is_some() {
            return;
        }
        if self.last_frame_at.elapsed() < Duration::from_millis(33) {
            return;
        }
        let Some(s) = self.current.as_ref() else {
            return;
        };
        if s.shows_window || s.handle.render.is_null() {
            return;
        }
        let dw = s.handle.get_i64("dwidth").or_else(|| s.handle.get_i64("width")).unwrap_or(0) as u32;
        let dh = s.handle.get_i64("dheight").or_else(|| s.handle.get_i64("height")).unwrap_or(0) as u32;
        if dw == 0 || dh == 0 {
            return;
        }
        let (fw, fh) = fit_within((dw, dh), self.preview_max);
        let stride = (fw as usize) * 4;
        let needed = stride * fh as usize;
        if self.pixels.len() != needed {
            self.pixels.resize(needed, 0);
        }
        // Drain render update.
        unsafe {
            let _ = (s.handle.lib.render_update)(s.handle.render);
        }
        if let Err(e) = s.handle.render_sw(fw as i32, fh as i32, stride, &mut self.pixels) {
            log::trace!("sw render: {e}");
            return;
        }
        let mut img = ColorImage::new([fw as usize, fh as usize], Color32::BLACK);
        for (dst, src) in img.pixels.iter_mut().zip(self.pixels.chunks_exact(4)) {
            *dst = Color32::from_rgba_unmultiplied(src[0], src[1], src[2], 255);
        }
        *lock(&self.shared.frame) = Some(img);
        self.last_frame_at = Instant::now();
        if let Some(s) = self.current.as_mut() {
            s.first_frame = true;
            s.has_video = true;
        }
        self.ctx.request_repaint();
    }

    fn check_no_picture(&mut self) {
        let Some(s) = &self.current else {
            return;
        };
        if s.first_frame || !s.loaded {
            return;
        }
        if s.opened.elapsed() < NO_PICTURE_LIMIT {
            return;
        }
        let gen = s.generation;
        self.publish_error(
            gen,
            "No picture from this file for 20 s (skipped)".into(),
        );
        self.current = None;
        self.ctx.request_repaint();
    }

    fn publish_loading(&self, generation: u64) {
        let mut st = lock(&self.shared.status);
        if st.generation == generation || st.generation == 0 {
            st.generation = generation;
            st.loaded = false;
            st.has_video = false;
            st.ended = false;
            st.error = None;
            st.paused = self.paused;
            st.fullscreen = self.fullscreen_monitor.is_some();
        }
    }

    fn publish_error(&self, generation: u64, msg: String) {
        let mut st = lock(&self.shared.status);
        if st.generation == generation || generation == st.generation {
            st.error = Some(msg);
            st.ended = false;
        }
    }
}

/// Best-effort monitor list for the Video menu (index passed to mpv `fullscreen-screen`).
pub(super) fn monitors() -> Vec<String> {
    // Without a display-server probe, expose a single generic entry so the UI can still
    // request fullscreen. Real multi-monitor names are a follow-up.
    vec!["Display 0 (main)".to_owned()]
}

pub(super) fn private_memory_mb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest
                .split_whitespace()
                .next()?
                .parse()
                .ok()?;
            return Some(kb / 1024);
        }
    }
    None
}
