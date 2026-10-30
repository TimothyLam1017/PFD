use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, Key, KeyboardShortcut, Layout, Modifiers, RichText, Sense,
};
use egui_extras::{Column, TableBuilder};

use crate::autostart;
use crate::config::{self, Config};
use crate::engine::{lock, Engine, EngineEvent, PlaybackState, RepeatMode, SharedEngine};
use crate::fonts;
use crate::launch::LaunchOptions;
use crate::metadata::MetaLoader;
use crate::playlist::{self, remap_index, Track};
use crate::video::{self, FitMode, FullscreenKey, UiEvent};

const SEEK_STEP_SECS: f32 = 5.0;
const VOLUME_STEP: f32 = 5.0;
const STATUS_TIMEOUT: Duration = Duration::from_secs(8);
/// Settings and the queue are saved this long after the last change.
const PERSIST_DELAY: Duration = Duration::from_millis(1500);
/// After auto-start, keep retrying playback this long (audio device / disks may be late).
const AUTOSTART_RETRY_FOR: Duration = Duration::from_secs(90);

const SC_OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SC_SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SC_SAVE_AS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const SC_NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const SC_NEXT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::ArrowRight);
const SC_PREV: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::ArrowLeft);
const SC_VOL_UP: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::ArrowUp);
const SC_VOL_DOWN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::ArrowDown);
const SC_MOVE_UP: KeyboardShortcut = KeyboardShortcut::new(Modifiers::ALT, Key::ArrowUp);
const SC_MOVE_DOWN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::ALT, Key::ArrowDown);

/// Everything the UI can ask for. The UI only reads engine state while drawing
/// and queues actions; they run after the frame with the engine lock released,
/// so native file dialogs never block the playback watcher thread.
enum Action {
    PlayPause,
    Stop,
    Next,
    Previous,
    PlayIndex(usize),
    PlaySelected,
    Seek(f32),
    SeekRelative(f32),
    SetVolume(f32),
    ToggleMute,
    CycleRepeat,
    SetRepeat(RepeatMode),
    Select(usize),
    SelectDelta(isize),
    Remove(usize),
    RemoveSelected,
    Move(usize, usize),
    MoveSelected(isize),
    CopyPath(usize),
    NewPlaylist,
    OpenPlaylist,
    SavePlaylist,
    SavePlaylistAs,
    AddFiles,
    AddFolder,
    Import(Vec<PathBuf>),
    Quit,
    ToggleFullscreen,
    SetFit(FitMode),
    SetMonitor(usize),
    TogglePlaylistPanel,
    SetAutostart(bool),
    SetAutostartResume(bool),
}

struct Status {
    text: String,
    is_error: bool,
    at: Instant,
}

pub struct AudioPlayerApp {
    engine: SharedEngine,
    config: Config,
    meta: MetaLoader,
    /// UI volume, 0..=100.
    volume: f32,
    selected: Option<usize>,
    /// Slider value while the user is dragging the seek bar.
    seek_drag: Option<f32>,
    scroll_to: Option<usize>,
    status: Option<Status>,
    playlist_path: Option<PathBuf>,
    dirty: bool,
    window_title: String,
    persisted: bool,
    /// Save settings + queue at this time (debounced after changes).
    persist_at: Option<Instant>,
    /// Latest video frame for the windowed video area.
    video_tex: Option<egui::TextureHandle>,
    /// Preview size last sent to the video thread (physical pixels).
    preview_sent: (u32, u32),
    autostart_enabled: bool,
    /// Minimize the player window on the first frame (auto-start goes straight to fullscreen).
    minimize_on_start: bool,
    /// Auto-start: (queue position, retry until, last attempt) while playback can't start yet.
    autostart_retry: Option<(usize, Instant, Instant)>,
}

impl AudioPlayerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = &cc.egui_ctx;
        let config = Config::load();

        fonts::install(ctx, config.font_path.as_deref());
        ctx.style_mut(|style| {
            // Labels must not swallow clicks meant for playlist rows.
            style.interaction.selectable_labels = false;
        });

        let launch = LaunchOptions::parse(std::env::args_os().skip(1));

        let engine = Engine::spawn(ctx.clone());
        {
            let mut eng = lock(&engine);
            eng.set_volume(f32::from(config.volume) / 100.0);
            eng.set_muted(config.muted);
            eng.repeat = launch.repeat.unwrap_or(config.repeat);
            eng.set_fit(launch.fit.unwrap_or(config.fit));
            eng.set_memory_limit_mb(config.memory_limit_mb);
            #[cfg(windows)]
            {
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                if let Ok(handle) = cc.window_handle() {
                    if let RawWindowHandle::Win32(h) = handle.as_raw() {
                        eng.video().set_main_window(h.hwnd.get() as usize);
                    }
                }
            }
        }

        autostart::refresh();
        let mut app = Self {
            engine,
            meta: MetaLoader::new(ctx.clone()),
            volume: f32::from(config.volume.min(100)),
            selected: None,
            seek_drag: None,
            scroll_to: None,
            status: None,
            playlist_path: None,
            dirty: false,
            window_title: String::new(),
            persisted: false,
            persist_at: None,
            video_tex: None,
            preview_sent: (0, 0),
            autostart_enabled: autostart::is_enabled(),
            minimize_on_start: false,
            autostart_retry: None,
            config,
        };
        app.start_up(launch);
        app
    }

    /// Decide what to play at launch: files from the command line, or the saved queue
    /// (auto-start / watchdog restart), optionally straight into fullscreen.
    fn start_up(&mut self, launch: LaunchOptions) {
        let mut start_index = None;
        let mut fullscreen = launch.fullscreen;

        // Files passed on the command line (e.g. "Open with…") replace the queue and play.
        if !launch.files.is_empty() {
            let tracks = self.collect_tracks(&launch.files);
            if !tracks.is_empty() {
                self.request_metadata(&tracks);
                lock(&self.engine).set_tracks(tracks);
                self.dirty = true;
                start_index = Some(launch.start_at.unwrap_or(0));
            }
        }

        // Otherwise restore the previous session.
        if start_index.is_none() {
            if let Some(tracks) = config::load_queue() {
                self.request_metadata(&tracks);
                let mut eng = lock(&self.engine);
                eng.set_tracks(tracks);
                eng.set_current(self.config.last_track);
                self.selected = eng.current();
                self.scroll_to = eng.current();
                let empty = eng.tracks().is_empty();
                drop(eng);
                self.playlist_path = self.config.last_playlist.clone();
                self.dirty = self.config.queue_dirty;

                // Autostart / watchdog / Options "Continue from the last item played":
                // when Continue is on, a normal open that restored the saved queue also
                // starts at the last item (not only --autostart / --resume).
                if !empty && (launch.autostart || launch.resume || self.config.autostart_resume)
                {
                    let resume_last = launch.resume || self.config.autostart_resume;
                    start_index = launch
                        .start_at
                        .or(if resume_last {
                            self.config.last_track
                        } else {
                            None
                        })
                        .or(Some(0));
                    if launch.autostart {
                        fullscreen = video::AVAILABLE;
                        self.minimize_on_start = true;
                        if launch.repeat.is_none() {
                            lock(&self.engine).repeat = RepeatMode::All;
                        }
                    }
                }
            }
        }

        if let Some(i) = start_index {
            let mut eng = lock(&self.engine);
            let i = i.min(eng.tracks().len().saturating_sub(1));
            eng.toggle_play_pause(Some(i));
            if launch.autostart || launch.resume {
                let now = Instant::now();
                self.autostart_retry = Some((i, now + AUTOSTART_RETRY_FOR, now));
            }
        }
        if fullscreen && video::AVAILABLE {
            let monitor = launch.monitor.unwrap_or(self.config.fullscreen_monitor);
            let mut eng = lock(&self.engine);
            if self.config.show_fullscreen_exit_hint {
                eng.video().arm_fullscreen_exit_hint();
            }
            eng.set_fullscreen(Some(monitor));
        }
    }

    // ----------------------------------------------------------- helpers

    fn info(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            is_error: false,
            at: Instant::now(),
        });
    }

    fn error(&mut self, text: impl Into<String>) {
        let text = text.into();
        log::error!("{text}");
        self.status = Some(Status {
            text,
            is_error: true,
            at: Instant::now(),
        });
    }

    /// Read tags in the background. Every track is checked (not only those with
    /// missing fields) so that missing files are flagged right away; values
    /// already present in the playlist are never overwritten.
    fn request_metadata(&mut self, tracks: &[Track]) {
        for t in tracks {
            self.meta.request(&t.path);
        }
    }

    fn apply_metadata(&mut self) {
        let results = self.meta.poll();
        if results.is_empty() {
            return;
        }
        let results: HashMap<_, _> = results.into_iter().collect();
        let mut eng = lock(&self.engine);
        for track in eng.tracks_mut() {
            match results.get(&track.path) {
                Some(Ok(meta)) => track.fill_from(meta),
                Some(Err(_)) if !track.path.is_file() => track.unavailable = true,
                _ => {}
            }
        }
    }

    /// Expand files / folders / playlist files into a list of tracks.
    fn collect_tracks(&mut self, paths: &[PathBuf]) -> Vec<Track> {
        let mut tracks = Vec::new();
        for path in paths {
            if path.is_dir() {
                tracks.extend(playlist::scan_dir(path).into_iter().map(Track::from_path));
            } else if playlist::is_playlist_file(path) {
                match playlist::load(path) {
                    Ok(list) => tracks.extend(list),
                    Err(e) => self.error(e),
                }
            } else if playlist::is_supported_media(path) {
                tracks.push(Track::from_path(path.clone()));
            }
        }
        tracks
    }

    fn add_tracks(&mut self, tracks: Vec<Track>) {
        if tracks.is_empty() {
            self.info("No supported files found");
            return;
        }
        let n = tracks.len();
        self.request_metadata(&tracks);
        lock(&self.engine).add_tracks(tracks);
        self.dirty = true;
        self.info(format!("Added {n} track{}", if n == 1 { "" } else { "s" }));
    }

    fn load_playlist(&mut self, path: &Path) {
        match playlist::load(path) {
            Ok(tracks) => {
                let n = tracks.len();
                self.request_metadata(&tracks);
                lock(&self.engine).set_tracks(tracks);
                self.playlist_path = Some(path.to_path_buf());
                self.dirty = false;
                self.selected = None;
                self.scroll_to = Some(0);
                self.info(format!("Opened {} ({n} tracks)", file_name(path)));
            }
            Err(e) => self.error(e),
        }
    }

    fn save_playlist(&mut self, path: &Path) {
        let result = playlist::save(path, lock(&self.engine).tracks());
        match result {
            Ok(()) => {
                self.playlist_path = Some(path.to_path_buf());
                self.dirty = false;
                self.info(format!("Saved {}", file_name(path)));
            }
            Err(e) => self.error(e),
        }
    }

    fn file_dialog(&self) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new();
        match self.config.last_dir.as_deref().filter(|d| d.is_dir()) {
            Some(dir) => dialog.set_directory(dir),
            None => dialog,
        }
    }

    fn remember_dir(&mut self, path: &Path) {
        let dir = if path.is_dir() {
            Some(path)
        } else {
            path.parent()
        };
        self.config.last_dir = dir.map(Path::to_path_buf);
    }

    fn request_persist(&mut self) {
        if self.persist_at.is_none() {
            self.persist_at = Some(Instant::now() + PERSIST_DELAY);
        }
    }

    fn persist(&mut self) {
        self.persist_at = None;
        {
            let eng = lock(&self.engine);
            self.config.volume = self.volume.round().clamp(0.0, 100.0) as u8;
            self.config.muted = eng.is_muted();
            self.config.repeat = eng.repeat;
            self.config.last_track = eng.current().or(self.selected);
            self.config.last_playlist = self.playlist_path.clone();
            self.config.queue_dirty = self.dirty;
            self.config.fit = eng.fit();
            if let Err(e) = config::save_queue(eng.tracks()) {
                log::error!("Saving queue failed: {e}");
            }
        }
        if let Err(e) = self.config.save() {
            log::error!("Saving config failed: {e}");
        }
        self.persisted = true;
    }

    // ----------------------------------------------------------- actions

    fn perform(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            // Actions that open native dialogs: run without holding the engine lock.
            Action::OpenPlaylist => {
                if let Some(path) = self
                    .file_dialog()
                    .add_filter("Playlist (JSON)", &["json"])
                    .pick_file()
                {
                    self.remember_dir(&path);
                    self.load_playlist(&path);
                }
            }
            Action::SavePlaylist => match self.playlist_path.clone() {
                Some(path) => self.save_playlist(&path),
                None => self.perform(ctx, Action::SavePlaylistAs),
            },
            Action::SavePlaylistAs => {
                let name = self
                    .playlist_path
                    .as_deref()
                    .map_or_else(|| "playlist.json".to_owned(), file_name);
                if let Some(mut path) = self
                    .file_dialog()
                    .add_filter("Playlist (JSON)", &["json"])
                    .set_file_name(name)
                    .save_file()
                {
                    if path.extension().is_none() {
                        path.set_extension("json");
                    }
                    self.remember_dir(&path);
                    self.save_playlist(&path);
                }
            }
            Action::AddFiles => {
                if let Some(files) = self
                    .file_dialog()
                    .add_filter(
                        if video::AVAILABLE {
                            "Audio & video"
                        } else {
                            "Audio"
                        },
                        &playlist::supported_extensions(),
                    )
                    .add_filter("All files", &["*"])
                    .pick_files()
                {
                    if let Some(first) = files.first() {
                        self.remember_dir(first);
                    }
                    let tracks = self.collect_tracks(&files);
                    self.add_tracks(tracks);
                }
            }
            Action::AddFolder => {
                if let Some(dir) = self.file_dialog().pick_folder() {
                    self.remember_dir(&dir);
                    let tracks = self.collect_tracks(&[dir]);
                    self.add_tracks(tracks);
                }
            }
            Action::Import(paths) => {
                // A single dropped .json opens it as the playlist; anything else is appended.
                if let [single] = paths.as_slice() {
                    if playlist::is_playlist_file(single) {
                        self.load_playlist(single);
                        return;
                    }
                }
                let tracks = self.collect_tracks(&paths);
                self.add_tracks(tracks);
            }
            Action::NewPlaylist => {
                lock(&self.engine).clear();
                self.selected = None;
                self.playlist_path = None;
                self.dirty = false;
                self.info("New playlist");
            }
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::Select(i) => self.selected = Some(i),
            Action::CopyPath(i) => {
                if let Some(t) = lock(&self.engine).tracks().get(i) {
                    ctx.copy_text(t.path.display().to_string());
                }
            }
            Action::TogglePlaylistPanel => self.config.show_playlist = !self.config.show_playlist,
            Action::SetAutostart(on) => match autostart::set_enabled(on) {
                Ok(()) => {
                    self.autostart_enabled = autostart::is_enabled();
                    self.info(if on {
                        "PFD will start with Windows and loop the last playlist fullscreen"
                    } else {
                        "PFD will no longer start with Windows"
                    });
                }
                Err(e) => self.error(e),
            },
            Action::SetAutostartResume(on) => {
                self.config.autostart_resume = on;
                self.info(if on {
                    "Will continue from the last item played"
                } else {
                    "Will start from the first item"
                });
            }
            Action::SetVolume(v) => {
                self.volume = v.clamp(0.0, 100.0);
                let mut eng = lock(&self.engine);
                eng.set_volume(self.volume / 100.0);
                if eng.is_muted() && self.volume > 0.0 {
                    eng.set_muted(false);
                }
            }

            // Everything else just manipulates the engine.
            action => {
                let engine = Arc::clone(&self.engine);
                let mut eng = lock(&engine);
                let len = eng.tracks().len();
                match action {
                    Action::PlayPause => eng.toggle_play_pause(self.selected),
                    Action::Stop => eng.stop(),
                    Action::Next => eng.next(),
                    Action::Previous => eng.previous(),
                    Action::PlayIndex(i) => {
                        self.selected = Some(i);
                        eng.play_index(i);
                    }
                    Action::PlaySelected => {
                        if let Some(i) = self.selected {
                            eng.play_index(i);
                        }
                    }
                    Action::Seek(secs) => {
                        if let Err(e) = eng.seek(Duration::from_secs_f32(secs.max(0.0))) {
                            drop(eng);
                            self.error(e);
                        }
                    }
                    Action::SeekRelative(delta) => {
                        if let Err(e) = eng.seek_relative(delta) {
                            drop(eng);
                            self.error(e);
                        }
                    }
                    Action::ToggleMute => {
                        let muted = !eng.is_muted();
                        eng.set_muted(muted);
                    }
                    Action::ToggleFullscreen if video::AVAILABLE => {
                        if eng.fullscreen().is_some() {
                            eng.set_fullscreen(None);
                        } else {
                            if self.config.show_fullscreen_exit_hint {
                                eng.video().arm_fullscreen_exit_hint();
                            }
                            eng.set_fullscreen(Some(self.config.fullscreen_monitor));
                        }
                    }
                    Action::SetFit(fit) => {
                        eng.set_fit(fit);
                        self.config.fit = fit;
                    }
                    Action::SetMonitor(m) => {
                        self.config.fullscreen_monitor = m;
                        if eng.fullscreen().is_some() {
                            if self.config.show_fullscreen_exit_hint {
                                eng.video().arm_fullscreen_exit_hint();
                            }
                            eng.set_fullscreen(Some(m));
                        }
                    }
                    Action::CycleRepeat => eng.repeat = eng.repeat.cycle(),
                    Action::SetRepeat(mode) => eng.repeat = mode,
                    Action::SelectDelta(delta) if len > 0 => {
                        let base = self.selected.or(eng.current());
                        let next = match base {
                            Some(s) => s.saturating_add_signed(delta).min(len - 1),
                            None if delta >= 0 => 0,
                            None => len - 1,
                        };
                        self.selected = Some(next);
                        self.scroll_to = Some(next);
                    }
                    Action::Remove(i) => {
                        eng.remove(i);
                        let len = eng.tracks().len();
                        self.selected = match self.selected {
                            Some(s) if s > i => Some(s - 1),
                            Some(s) if s == i && len > 0 => Some(i.min(len - 1)),
                            Some(s) if s == i => None,
                            other => other,
                        };
                        self.dirty = true;
                    }
                    Action::RemoveSelected => {
                        if let Some(i) = self.selected {
                            drop(eng);
                            self.perform(ctx, Action::Remove(i));
                        }
                    }
                    Action::Move(from, to) if from < len && to < len && from != to => {
                        eng.move_track(from, to);
                        self.selected = self.selected.map(|s| remap_index(s, from, to));
                        self.scroll_to = Some(to);
                        self.dirty = true;
                    }
                    Action::MoveSelected(delta) => {
                        if let Some(s) = self.selected {
                            if let Some(to) = s.checked_add_signed(delta).filter(|&t| t < len) {
                                drop(eng);
                                self.perform(ctx, Action::Move(s, to));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // ------------------------------------------------------------- input

    fn handle_input(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        // Drag & drop of files / folders / playlists.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            actions.push(Action::Import(dropped));
        }

        if ctx.wants_keyboard_input() {
            return;
        }
        ctx.input_mut(|i| {
            // Shortcuts with more modifiers first, so they aren't matched by shorter ones.
            if i.consume_shortcut(&SC_SAVE_AS) {
                actions.push(Action::SavePlaylistAs);
            }
            if i.consume_shortcut(&SC_OPEN) {
                actions.push(Action::OpenPlaylist);
            }
            if i.consume_shortcut(&SC_SAVE) {
                actions.push(Action::SavePlaylist);
            }
            if i.consume_shortcut(&SC_NEW) {
                actions.push(Action::NewPlaylist);
            }
            if i.consume_shortcut(&SC_NEXT) {
                actions.push(Action::Next);
            }
            if i.consume_shortcut(&SC_PREV) {
                actions.push(Action::Previous);
            }
            if i.consume_shortcut(&SC_VOL_UP) {
                actions.push(Action::SetVolume(self.volume + VOLUME_STEP));
            }
            if i.consume_shortcut(&SC_VOL_DOWN) {
                actions.push(Action::SetVolume(self.volume - VOLUME_STEP));
            }
            if i.consume_shortcut(&SC_MOVE_UP) {
                actions.push(Action::MoveSelected(-1));
            }
            if i.consume_shortcut(&SC_MOVE_DOWN) {
                actions.push(Action::MoveSelected(1));
            }
            if i.consume_key(Modifiers::NONE, Key::Space) {
                actions.push(Action::PlayPause);
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowRight) {
                actions.push(Action::SeekRelative(SEEK_STEP_SECS));
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowLeft) {
                actions.push(Action::SeekRelative(-SEEK_STEP_SECS));
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                actions.push(Action::SelectDelta(-1));
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                actions.push(Action::SelectDelta(1));
            }
            if i.consume_key(Modifiers::NONE, Key::Enter) {
                actions.push(Action::PlaySelected);
            }
            if i.consume_key(Modifiers::NONE, Key::Delete) {
                actions.push(Action::RemoveSelected);
            }
            if i.consume_key(Modifiers::NONE, Key::M) {
                actions.push(Action::ToggleMute);
            }
            if i.consume_key(Modifiers::NONE, Key::F) {
                actions.push(Action::ToggleFullscreen);
            }
        });
    }

    // ---------------------------------------------------------------- UI

    fn menu_bar(&self, ctx: &egui::Context, eng: &Engine, actions: &mut Vec<Action>) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    let mut item = |ui: &mut egui::Ui,
                                    label: &str,
                                    sc: Option<&KeyboardShortcut>,
                                    action: Action| {
                        let mut button = egui::Button::new(label);
                        if let Some(sc) = sc {
                            button = button.shortcut_text(ui.ctx().format_shortcut(sc));
                        }
                        if ui.add(button).clicked() {
                            actions.push(action);
                            ui.close_menu();
                        }
                    };
                    item(ui, "New playlist", Some(&SC_NEW), Action::NewPlaylist);
                    item(ui, "Open playlist…", Some(&SC_OPEN), Action::OpenPlaylist);
                    item(ui, "Save playlist", Some(&SC_SAVE), Action::SavePlaylist);
                    item(
                        ui,
                        "Save playlist as…",
                        Some(&SC_SAVE_AS),
                        Action::SavePlaylistAs,
                    );
                    ui.separator();
                    item(ui, "Add files…", None, Action::AddFiles);
                    item(ui, "Add folder…", None, Action::AddFolder);
                    ui.separator();
                    item(ui, "Quit", None, Action::Quit);
                });

                ui.menu_button("Playback", |ui| {
                    let play_label = if eng.state() == PlaybackState::Playing {
                        "Pause"
                    } else {
                        "Play"
                    };
                    let mut item = |ui: &mut egui::Ui, label: &str, sc: &str, action: Action| {
                        if ui.add(egui::Button::new(label).shortcut_text(sc)).clicked() {
                            actions.push(action);
                            ui.close_menu();
                        }
                    };
                    let next = ui.ctx().format_shortcut(&SC_NEXT);
                    let prev = ui.ctx().format_shortcut(&SC_PREV);
                    item(ui, play_label, "Space", Action::PlayPause);
                    item(ui, "Stop", "", Action::Stop);
                    item(ui, "Previous", &prev, Action::Previous);
                    item(ui, "Next", &next, Action::Next);
                    item(
                        ui,
                        "Seek forward 5 s",
                        "→",
                        Action::SeekRelative(SEEK_STEP_SECS),
                    );
                    item(
                        ui,
                        "Seek back 5 s",
                        "←",
                        Action::SeekRelative(-SEEK_STEP_SECS),
                    );
                    item(
                        ui,
                        if eng.is_muted() { "Unmute" } else { "Mute" },
                        "M",
                        Action::ToggleMute,
                    );
                    ui.separator();
                    for mode in [RepeatMode::Off, RepeatMode::All, RepeatMode::One] {
                        if ui.radio(eng.repeat == mode, mode.label()).clicked() {
                            actions.push(Action::SetRepeat(mode));
                            ui.close_menu();
                        }
                    }
                });

                ui.menu_button("Video", |ui| {
                    if !video::AVAILABLE {
                        ui.label(RichText::new("Video playback needs Windows").weak());
                        return;
                    }
                    let label = if eng.fullscreen().is_some() {
                        "Leave fullscreen"
                    } else {
                        "Fullscreen"
                    };
                    if ui
                        .add(egui::Button::new(label).shortcut_text("F / double-click"))
                        .clicked()
                    {
                        actions.push(Action::ToggleFullscreen);
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.label(RichText::new("Picture shape").weak());
                    for fit in FitMode::ALL {
                        if ui.radio(eng.fit() == fit, fit.label()).clicked() {
                            actions.push(Action::SetFit(fit));
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    ui.menu_button("Fullscreen monitor", |ui| {
                        let monitors = video::monitors();
                        if monitors.is_empty() {
                            ui.label("No monitor found");
                        }
                        for (i, name) in monitors.iter().enumerate() {
                            if ui.radio(self.config.fullscreen_monitor == i, name).clicked() {
                                actions.push(Action::SetMonitor(i));
                                ui.close_menu();
                            }
                        }
                    });
                    let mut show = self.config.show_playlist;
                    if ui.checkbox(&mut show, "Show playlist under the video").clicked() {
                        actions.push(Action::TogglePlaylistPanel);
                        ui.close_menu();
                    }
                });

                ui.menu_button("Options", |ui| {
                    ui.label(
                        "The first time you go fullscreen, a brief bottom hint says “Double-click to exit fullscreen”; it is not shown again.",
                    );

                    ui.add_enabled_ui(autostart::AVAILABLE, |ui| {
                        let mut on = self.autostart_enabled;
                        if ui
                            .checkbox(&mut on, "Start with Windows")
                            .on_hover_text(
                                "After you sign in to Windows, PFD opens in fullscreen \
                                 and loops the last playlist. Double-click the video to leave fullscreen.",
                            )
                            .clicked()
                        {
                            actions.push(Action::SetAutostart(on));
                            ui.close_menu();
                        }
                    });
                    let mut resume = self.config.autostart_resume;
                    if ui
                        .checkbox(&mut resume, "Continue from the last item played")
                        .clicked()
                    {
                        actions.push(Action::SetAutostartResume(resume));
                        ui.close_menu();
                    }
                });
            });
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context, eng: &Engine) {
        // Errors stick until the next successful action or the user clears them.
        if self
            .status
            .as_ref()
            .is_some_and(|s| !s.is_error && s.at.elapsed() > STATUS_TIMEOUT)
        {
            self.status = None;
        }
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(status) = &self.status {
                    let color = if status.is_error {
                        ui.visuals().error_fg_color
                    } else {
                        ui.visuals().weak_text_color()
                    };
                    let resp = ui.add(
                        egui::Label::new(RichText::new(&status.text).color(color))
                            .truncate()
                            .sense(egui::Sense::click()),
                    );
                    if resp.on_hover_text("Click to dismiss").clicked() {
                        self.status = None;
                    }
                } else if self.meta.pending() > 0 {
                    ui.label(
                        RichText::new(format!("Reading tags… {} left", self.meta.pending())).weak(),
                    );
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let tracks = eng.tracks();
                    let total: u64 = tracks.iter().filter_map(|t| t.duration_secs).sum();
                    ui.label(
                        RichText::new(format!(
                            "{} track{} · {}",
                            tracks.len(),
                            if tracks.len() == 1 { "" } else { "s" },
                            format_time(total as f32)
                        ))
                        .weak(),
                    );
                    if let Some(path) = &self.playlist_path {
                        let name = file_name(path);
                        let text = if self.dirty {
                            format!("{name} •")
                        } else {
                            name
                        };
                        ui.label(RichText::new(text).weak())
                            .on_hover_text(path.display().to_string());
                    }
                });
            });
        });
    }

    fn transport_panel(&mut self, ctx: &egui::Context, eng: &Engine, actions: &mut Vec<Action>) {
        egui::TopBottomPanel::bottom("transport")
            .frame(
                egui::Frame::side_top_panel(&ctx.style())
                    .inner_margin(egui::Margin::symmetric(12, 10)),
            )
            .show(ctx, |ui| {
                // Now playing.
                match eng.now_playing() {
                    Some(track) => {
                        ui.add(
                            egui::Label::new(
                                RichText::new(track.display_title()).size(17.0).strong(),
                            )
                            .truncate(),
                        );
                        let artist = match (&track.artist, &track.album) {
                            (Some(a), Some(al)) => format!("{a} — {al}"),
                            (Some(a), None) => a.clone(),
                            (None, Some(al)) => al.clone(),
                            (None, None) => String::new(),
                        };
                        ui.add(egui::Label::new(RichText::new(artist).weak()).truncate());
                    }
                    None => {
                        ui.label(RichText::new("Nothing playing").size(17.0).weak());
                        ui.label("");
                    }
                }
                ui.add_space(4.0);

                // Seek bar.
                let active = eng.state() != PlaybackState::Stopped;
                let total = eng.duration().map_or(0.0, |d| d.as_secs_f32());
                let position = eng.position().as_secs_f32();
                let mut value = self.seek_drag.unwrap_or(position).min(total);
                ui.horizontal(|ui| {
                    let time_width = 52.0;
                    ui.add_sized([time_width, 18.0], egui::Label::new(format_time(value)));
                    ui.spacing_mut().slider_width =
                        (ui.available_width() - time_width - 8.0).max(40.0);
                    let response = ui.add_enabled(
                        active && total > 0.0,
                        egui::Slider::new(&mut value, 0.0..=total.max(0.001))
                            .show_value(false)
                            .trailing_fill(true),
                    );
                    if response.dragged() {
                        self.seek_drag = Some(value);
                    }
                    if response.drag_stopped() || (response.changed() && !response.dragged()) {
                        actions.push(Action::Seek(value));
                        self.seek_drag = None;
                    }
                    ui.add_sized([time_width, 18.0], egui::Label::new(format_time(total)));
                });
                ui.add_space(4.0);

                // Buttons + volume.
                ui.horizontal(|ui| {
                    let big = |icon: &str| {
                        egui::Button::new(RichText::new(icon).size(20.0))
                            .min_size(egui::vec2(40.0, 32.0))
                    };
                    if ui.add(big("⏮")).on_hover_text("Previous — restart current if more than 3 seconds in").clicked() {
                        actions.push(Action::Previous);
                    }
                    let (icon, tip) = if eng.state() == PlaybackState::Playing {
                        ("⏸", "Pause")
                    } else {
                        ("▶", "Play")
                    };
                    if ui.add(big(icon)).on_hover_text(tip).clicked() {
                        actions.push(Action::PlayPause);
                    }
                    if ui.add(big("⏹")).on_hover_text("Stop").clicked() {
                        actions.push(Action::Stop);
                    }
                    if ui.add(big("⏭")).on_hover_text("Next").clicked() {
                        actions.push(Action::Next);
                    }
                    ui.add_space(8.0);
                    let (icon, on) = match eng.repeat {
                        RepeatMode::Off => ("🔁", false),
                        RepeatMode::All => ("🔁", true),
                        RepeatMode::One => ("🔂", true),
                    };
                    if ui
                        .add(big(icon).selected(on))
                        .on_hover_text(format!("{} (click to change)", eng.repeat.label()))
                        .clicked()
                    {
                        actions.push(Action::CycleRepeat);
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().slider_width = 110.0;
                        let mut volume = self.volume;
                        let response = ui.add(
                            egui::Slider::new(&mut volume, 0.0..=100.0)
                                .suffix("%")
                                .fixed_decimals(0)
                                .trailing_fill(true),
                        );
                        if response.changed() {
                            actions.push(Action::SetVolume(volume));
                        }
                        if response.hovered() {
                            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                            if scroll != 0.0 {
                                actions
                                    .push(Action::SetVolume(self.volume + scroll.signum() * 2.0));
                            }
                        }
                        let speaker = if eng.is_muted() || self.volume == 0.0 {
                            "🔇"
                        } else if self.volume < 50.0 {
                            "🔉"
                        } else {
                            "🔊"
                        };
                        if ui
                            .add(egui::Button::new(RichText::new(speaker).size(18.0)).frame(false))
                            .on_hover_text(if eng.is_muted() {
                                "Unmute (M)"
                            } else {
                                "Mute (M)"
                            })
                            .clicked()
                        {
                            actions.push(Action::ToggleMute);
                        }
                    });
                });
            });
    }

    fn playlist_ui(&mut self, ui: &mut egui::Ui, eng: &Engine, actions: &mut Vec<Action>) {
        let tracks = eng.tracks();
        if tracks.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new(if video::AVAILABLE {
                        "Drop audio or video files, folders or a .json playlist here\n\
                             or use File → Add files… / Open playlist…"
                    } else {
                        "Drop audio files, folders or a .json playlist here\n\
                             or use File → Add files… / Open playlist…"
                    })
                    .weak(),
                );
            });
            return;
        }

        let current = eng.current();
        let state = eng.state();
        let accent = ui.visuals().selection.stroke.color;
        let error = ui.visuals().error_fg_color;
        let weak = ui.visuals().weak_text_color();
        let row_height = ui.text_style_height(&egui::TextStyle::Body) + 10.0;

        let mut table = TableBuilder::new(ui)
            .striped(true)
            .sense(Sense::click())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::exact(40.0))
            .column(Column::remainder().at_least(120.0).clip(true))
            .column(
                Column::initial(200.0)
                    .at_least(60.0)
                    .clip(true)
                    .resizable(true),
            )
            .column(Column::exact(56.0))
            .min_scrolled_height(0.0)
            .max_scroll_height(f32::INFINITY);
        if let Some(row) = self.scroll_to.take() {
            table = table.scroll_to_row(row, None);
        }

        table
            .header(row_height, |mut header| {
                header.col(|ui| {
                    ui.strong("#");
                });
                header.col(|ui| {
                    ui.strong("Title");
                });
                header.col(|ui| {
                    ui.strong("Artist");
                });
                header.col(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.strong("Time");
                    });
                });
            })
            .body(|body| {
                body.rows(row_height, tracks.len(), |mut row| {
                    let i = row.index();
                    let track = &tracks[i];
                    let is_current = current == Some(i);
                    row.set_selected(self.selected == Some(i));

                    let style = |text: RichText| {
                        if track.unavailable {
                            text.color(error)
                        } else if is_current {
                            text.color(accent).strong()
                        } else {
                            text
                        }
                    };

                    row.col(|ui| {
                        let marker = match (is_current, state) {
                            (true, PlaybackState::Playing) => "▶".to_owned(),
                            (true, PlaybackState::Paused) => "⏸".to_owned(),
                            _ if track.unavailable => "⚠".to_owned(),
                            _ => (i + 1).to_string(),
                        };
                        let text = if is_current || track.unavailable {
                            style(RichText::new(marker))
                        } else {
                            RichText::new(marker).color(weak)
                        };
                        ui.label(text);
                    });
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(style(RichText::new(track.display_title())))
                                .truncate(),
                        );
                    });
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(style(RichText::new(track.display_artist())))
                                .truncate(),
                        );
                    });
                    row.col(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let time = track
                                .duration_secs
                                .map(|s| format_time(s as f32))
                                .unwrap_or_default();
                            ui.label(style(RichText::new(time)));
                        });
                    });

                    let response = row.response();
                    if response.double_clicked() {
                        actions.push(Action::PlayIndex(i));
                    } else if response.clicked() || response.secondary_clicked() {
                        actions.push(Action::Select(i));
                    }
                    let response = if track.unavailable {
                        response.on_hover_text(format!(
                            "Missing or unreadable:\n{}",
                            track.path.display()
                        ))
                    } else {
                        response
                    };
                    response.context_menu(|ui| {
                        if ui.button("▶  Play").clicked() {
                            actions.push(Action::PlayIndex(i));
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui
                            .add_enabled(i > 0, egui::Button::new("Move up"))
                            .clicked()
                        {
                            actions.push(Action::Move(i, i - 1));
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(i + 1 < tracks.len(), egui::Button::new("Move down"))
                            .clicked()
                        {
                            actions.push(Action::Move(i, i + 1));
                            ui.close_menu();
                        }
                        if ui.button("Copy file path").clicked() {
                            actions.push(Action::CopyPath(i));
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Remove from playlist").clicked() {
                            actions.push(Action::Remove(i));
                            ui.close_menu();
                        }
                    });
                });
            });
    }

    /// The windowed video picture. Double-click switches to fullscreen.
    fn video_ui(
        &mut self,
        ui: &mut egui::Ui,
        eng: &Engine,
        height: f32,
        actions: &mut Vec<Action>,
    ) {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::BLACK);

        // Ask the video thread for frames at the size actually shown.
        let ppp = ui.ctx().pixels_per_point();
        let wanted = if eng.fullscreen().is_some() {
            (0, 0)
        } else {
            ((rect.width() * ppp) as u32, (rect.height() * ppp) as u32)
        };
        self.send_preview_size(eng, wanted);

        if let Some(monitor) = eng.fullscreen() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                format!(
                    "Playing fullscreen on monitor {}\nDouble-click the video there to come back",
                    monitor + 1
                ),
                egui::FontId::proportional(15.0),
                Color32::from_gray(150),
            );
        } else if let Some(tex) = &self.video_tex {
            let [w, h] = tex.size();
            let (vp, uv) = eng
                .effective_fit()
                .placement(w as f32 / h as f32, rect.width() / rect.height());
            let target = egui::Rect::from_min_size(
                rect.min + egui::vec2(vp[0] * rect.width(), vp[1] * rect.height()),
                egui::vec2(vp[2] * rect.width(), vp[3] * rect.height()),
            );
            let uv = egui::Rect::from_min_size(egui::pos2(uv[0], uv[1]), egui::vec2(uv[2], uv[3]));
            painter.image(tex.id(), target, uv, Color32::WHITE);
        }

        if response.double_clicked() {
            actions.push(Action::ToggleFullscreen);
        }
        response.on_hover_text("Double-click for fullscreen (F)");
    }

    fn send_preview_size(&mut self, eng: &Engine, size: (u32, u32)) {
        // Avoid flooding the video thread while a window edge is being dragged.
        let changed = size.0.abs_diff(self.preview_sent.0) > 8
            || size.1.abs_diff(self.preview_sent.1) > 8
            || (size == (0, 0)) != (self.preview_sent == (0, 0));
        if changed {
            self.preview_sent = size;
            eng.video().set_preview_size(size.0, size.1);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context, eng: &Engine) {
        let title = match (eng.now_playing(), eng.state()) {
            (Some(t), PlaybackState::Playing | PlaybackState::Paused) if t.artist.is_some() => {
                format!(
                    "{} — {} · PFD",
                    t.display_title(),
                    t.display_artist()
                )
            }
            (Some(t), PlaybackState::Playing | PlaybackState::Paused) => {
                format!("{} · PFD", t.display_title())
            }
            _ => "PFD".to_owned(),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    fn drop_overlay(ctx: &egui::Context) {
        if ctx.input(|i| i.raw.hovered_files.is_empty()) {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop_overlay"),
        ));
        let rect = ctx.screen_rect();
        painter.rect_filled(rect, 0.0, Color32::from_black_alpha(170));
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Drop to add to the playlist",
            egui::FontId::proportional(22.0),
            Color32::WHITE,
        );
    }
}

impl eframe::App for AudioPlayerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if std::mem::take(&mut self.minimize_on_start) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        self.apply_metadata();

        let mut actions = Vec::new();
        self.handle_input(ctx, &mut actions);

        {
            let engine = Arc::clone(&self.engine);
            let mut eng = lock(&engine);
            for event in eng.take_events() {
                match event {
                    EngineEvent::TrackStarted(i) => {
                        self.scroll_to = Some(i);
                        self.request_persist();
                        if self.status.as_ref().is_some_and(|s| s.is_error) {
                            self.status = None;
                        }
                    }
                    EngineEvent::Error(e) => self.error(e),
                    EngineEvent::PlaylistEnded => self.info("End of playlist"),
                }
            }

            // Events from the fullscreen video window (it has no on-screen display).
            for event in eng.video().take_ui_events() {
                match event {
                    UiEvent::FullscreenExited => {
                        eng.fullscreen_closed();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    UiEvent::FullscreenExitHintShown => {
                        self.config.show_fullscreen_exit_hint = false;
                        self.request_persist();
                    }
                    UiEvent::Key(key) => actions.push(match key {
                        FullscreenKey::PlayPause => Action::PlayPause,
                        FullscreenKey::Next => Action::Next,
                        FullscreenKey::Previous => Action::Previous,
                        FullscreenKey::SeekForward => Action::SeekRelative(SEEK_STEP_SECS),
                        FullscreenKey::SeekBack => Action::SeekRelative(-SEEK_STEP_SECS),
                    }),
                }
            }

            // Newest windowed video frame.
            if let Some(image) = eng.video().take_frame() {
                match &mut self.video_tex {
                    Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
                    None => {
                        self.video_tex =
                            Some(ctx.load_texture("video", image, egui::TextureOptions::LINEAR))
                    }
                }
            }

            // Auto-start: the audio device or a network drive may not be ready yet.
            if let Some((index, until, last)) = self.autostart_retry {
                if Instant::now() > until || eng.state() != PlaybackState::Stopped {
                    if eng.state() != PlaybackState::Stopped || Instant::now() > until {
                        self.autostart_retry = None;
                    }
                } else if last.elapsed() > Duration::from_secs(5) {
                    log::info!("Auto-start: retrying playback");
                    eng.toggle_play_pause(Some(index));
                    self.autostart_retry = Some((index, until, Instant::now()));
                }
                ctx.request_repaint_after(Duration::from_secs(1));
            }

            self.menu_bar(ctx, &eng, &mut actions);
            self.status_bar(ctx, &eng);
            self.transport_panel(ctx, &eng, &mut actions);
            let show_video = eng.is_video_active() || eng.fullscreen().is_some();
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::central_panel(&ctx.style()).inner_margin(if show_video {
                        0.0
                    } else {
                        8.0
                    }),
                )
                .show(ctx, |ui| {
                    if show_video {
                        let full = ui.available_height();
                        let height = if self.config.show_playlist && !eng.tracks().is_empty() {
                            (full * 0.62).max(120.0).min(full)
                        } else {
                            full
                        };
                        self.video_ui(ui, &eng, height, &mut actions);
                        if self.config.show_playlist && ui.available_height() > 40.0 {
                            ui.add_space(4.0);
                            ui.scope(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                egui::Frame::new()
                                    .inner_margin(egui::Margin::symmetric(8, 0))
                                    .show(ui, |ui| self.playlist_ui(ui, &eng, &mut actions));
                            });
                        }
                    } else {
                        self.send_preview_size(&eng, (0, 0));
                        self.playlist_ui(ui, &eng, &mut actions);
                    }
                });
            self.update_title(ctx, &eng);

            if eng.state() == PlaybackState::Playing {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }
        Self::drop_overlay(ctx);

        if !actions.is_empty() {
            for action in actions {
                self.perform(ctx, action);
            }
            self.request_persist();
            ctx.request_repaint();
        }
        if let Some(status) = &self.status {
            ctx.request_repaint_after(STATUS_TIMEOUT.saturating_sub(status.at.elapsed()));
        }
        if let Some(at) = self.persist_at {
            if Instant::now() >= at {
                self.persist();
            } else {
                ctx.request_repaint_after(at - Instant::now());
            }
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            self.persist();
        }
    }
}

impl Drop for AudioPlayerApp {
    fn drop(&mut self) {
        if !self.persisted {
            self.persist();
        }
    }
}

fn format_time(secs: f32) -> String {
    let secs = secs.max(0.0) as u64;
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}
