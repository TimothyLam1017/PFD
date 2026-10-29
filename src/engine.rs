//! Playback engine: one play queue, two backends.
//!
//! * Audio files play through `rodio`.
//! * Video files (Windows) play through Media Foundation on the video thread
//!   (see [`crate::video`]).
//!
//! The engine owns the queue and lives behind an `Arc<Mutex<_>>`. A small watcher
//! thread checks every 150 ms whether the current item has finished (or failed) and
//! starts the next one, so auto-advance and loop-all keep working while the window is
//! minimized or covered by the fullscreen video. For videos it also keeps the *next*
//! video pre-loaded, so consecutive videos switch without a gap.
//!
//! The rodio device sink is created on, and kept alive by, its own thread, so
//! audio-device COM initialization on Windows never clashes with the UI thread.

use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use eframe::egui;
use rodio::mixer::Mixer;
use rodio::{Decoder, DeviceSinkBuilder, Player, Source};
use serde::{Deserialize, Serialize};

use crate::playlist::{remap_index, Track};
use crate::video::{self, FitMode, RestartHint, Video};

pub type SharedEngine = Arc<Mutex<Engine>>;

/// Lock the engine, recovering from a poisoned mutex instead of panicking.
pub fn lock(engine: &SharedEngine) -> MutexGuard<'_, Engine> {
    engine.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatMode {
    /// Stop after the last track.
    Off,
    /// Wrap around to the first track.
    #[default]
    All,
    /// Repeat the current track.
    One,
}

impl RepeatMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Repeat off",
            Self::All => "Repeat all",
            Self::One => "Repeat one",
        }
    }
}

#[derive(Debug)]
pub enum EngineEvent {
    TrackStarted(usize),
    Error(String),
    PlaylistEnded,
}

/// What is producing sound right now.
enum Active {
    None,
    Audio(Player),
    Video,
}

/// The next video, already handed to the video thread for a gapless switch.
struct Pending {
    index: usize,
    generation: u64,
    path: PathBuf,
}

pub struct Engine {
    output: Option<Mixer>,
    active: Active,
    video: Video,
    state: PlaybackState,
    tracks: Vec<Track>,
    current: Option<usize>,
    duration: Option<Duration>,
    volume: f32,
    muted: bool,
    pub repeat: RepeatMode,
    /// Default aspect mode for videos (a playlist entry may override it).
    fit: FitMode,
    /// `Some(monitor)` while fullscreen is on.
    fullscreen: Option<usize>,
    /// Generation number handed to the video thread with every file it opens.
    generation: u64,
    video_generation: u64,
    pending: Option<Pending>,
    video_loop: bool,
    /// Videos that failed in a row (async errors), to stop when nothing is playable.
    video_failures: usize,
    events: Vec<EngineEvent>,
}

impl Engine {
    /// Create the engine and start its watcher thread.
    pub fn spawn(ctx: egui::Context) -> SharedEngine {
        let engine = Arc::new(Mutex::new(Engine {
            output: None,
            active: Active::None,
            video: Video::new(ctx.clone()),
            state: PlaybackState::Stopped,
            tracks: Vec::new(),
            current: None,
            duration: None,
            volume: 0.7,
            muted: false,
            repeat: RepeatMode::default(),
            fit: FitMode::Fit,
            fullscreen: None,
            generation: 0,
            video_generation: 0,
            pending: None,
            video_loop: false,
            video_failures: 0,
            events: Vec::new(),
        }));

        let weak = Arc::downgrade(&engine);
        thread::Builder::new()
            .name("playback-watcher".into())
            .spawn(move || loop {
                thread::sleep(Duration::from_millis(150));
                let Some(engine) = weak.upgrade() else { break };
                let changed = lock(&engine).tick();
                drop(engine);
                if changed {
                    ctx.request_repaint();
                }
            })
            .expect("failed to spawn playback watcher thread");

        engine
    }

    // ---------------------------------------------------------------- queries

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn tracks_mut(&mut self) -> &mut [Track] {
        &mut self.tracks
    }

    pub fn state(&self) -> PlaybackState {
        self.state
    }

    pub fn current(&self) -> Option<usize> {
        self.current
    }

    pub fn now_playing(&self) -> Option<&Track> {
        self.current.and_then(|i| self.tracks.get(i))
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn video(&self) -> &Video {
        &self.video
    }

    /// True while the current item is a video (playing or paused).
    pub fn is_video_active(&self) -> bool {
        matches!(self.active, Active::Video)
    }

    pub fn is_video_track(&self, idx: usize) -> bool {
        video::AVAILABLE
            && self
                .tracks
                .get(idx)
                .is_some_and(|t| video::is_video_path(&t.path))
    }

    pub fn fit(&self) -> FitMode {
        self.fit
    }

    /// The aspect mode used for the current video (playlist override or default).
    pub fn effective_fit(&self) -> FitMode {
        self.now_playing().and_then(|t| t.fit).unwrap_or(self.fit)
    }

    pub fn fullscreen(&self) -> Option<usize> {
        self.fullscreen
    }

    pub fn take_events(&mut self) -> Vec<EngineEvent> {
        std::mem::take(&mut self.events)
    }

    /// Current playback position (zero when stopped).
    pub fn position(&self) -> Duration {
        let pos = match (&self.active, self.state) {
            (_, PlaybackState::Stopped) => Duration::ZERO,
            (Active::Audio(player), _) => player.get_pos(),
            (Active::Video, _) => Duration::from_secs_f64(self.video.status().position.max(0.0)),
            (Active::None, _) => Duration::ZERO,
        };
        match self.duration() {
            Some(total) => pos.min(total),
            None => pos,
        }
    }

    /// Total length of the current item: from the decoder if it knows it,
    /// otherwise from the tags / playlist.
    pub fn duration(&self) -> Option<Duration> {
        let decoded = match self.active {
            Active::Video => self.video.status().duration.map(Duration::from_secs_f64),
            _ => self.duration,
        };
        decoded.or_else(|| {
            self.now_playing()
                .and_then(|t| t.duration_secs)
                .map(Duration::from_secs)
        })
    }

    // ----------------------------------------------------------------- volume

    /// `volume` is linear 0.0..=1.0 as shown in the UI.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        self.apply_volume();
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.apply_volume();
    }

    fn apply_volume(&self) {
        if let Active::Audio(player) = &self.active {
            player.set_volume(self.effective_volume());
        }
        self.video.set_volume(self.effective_volume());
    }

    /// Loudness is perceived roughly logarithmically, so square the slider value
    /// to make 50% sound like "half as loud" instead of barely quieter.
    fn effective_volume(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.volume * self.volume
        }
    }

    // ------------------------------------------------------------ video modes

    pub fn set_fit(&mut self, fit: FitMode) {
        self.fit = fit;
        if self.is_video_active() {
            self.video.set_fit(self.effective_fit());
        }
    }

    /// `Some(monitor)` enters fullscreen there, `None` leaves it.
    pub fn set_fullscreen(&mut self, monitor: Option<usize>) {
        self.fullscreen = monitor;
        self.video.set_fullscreen(monitor);
        self.update_restart_hint();
    }

    /// The user left fullscreen from the video window itself (double-click).
    pub fn fullscreen_closed(&mut self) {
        self.fullscreen = None;
        self.update_restart_hint();
    }

    /// Memory limit exceeded: restart cleanly now, at the start of item `idx`, so the
    /// new process continues with exactly this item (fullscreen if it was fullscreen).
    fn restart_if_requested(&self, idx: usize) {
        if self.video.restart_requested() {
            if let Err(e) = crate::config::save_queue(&self.tracks) {
                log::error!("Saving queue before restart failed: {e}");
            }
            if let Err(e) = self.video.restart_now(&RestartHint {
                next_index: Some(idx),
                fullscreen_monitor: self.fullscreen,
            }) {
                // Spawn failed: keep this process running (see video::try_restart_process).
                log::error!("Clean restart failed; continuing in this process: {e}");
            }
        }
    }

    pub fn set_memory_limit_mb(&self, mb: u64) {
        self.video.set_memory_limit_mb(mb);
    }

    fn update_restart_hint(&self) {
        self.video.set_restart_hint(RestartHint {
            next_index: self.next_index(true).or(self.current),
            fullscreen_monitor: self.fullscreen,
        });
    }

    // --------------------------------------------------------------- playback

    fn ensure_output(&mut self) -> Result<Mixer, String> {
        if let Some(mixer) = &self.output {
            return Ok(mixer.clone());
        }
        let mixer = open_output()?;
        self.output = Some(mixer.clone());
        Ok(mixer)
    }

    fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    /// Start playing track `idx`. Returns an error message if it can't be played.
    /// (Video errors arrive later, asynchronously, and are handled in `tick`.)
    fn start(&mut self, idx: usize) -> Result<(), String> {
        let track = self.tracks.get(idx).ok_or("No such track")?;
        let path = track.path.clone();
        let fallback = track.duration_secs.map(Duration::from_secs);
        let name = track.display_title().into_owned();
        let fit = track.fit.unwrap_or(self.fit);

        self.stop_player();

        if self.is_video_track(idx) {
            if !path.is_file() {
                return Err(format!("Cannot open \"{name}\": file not found"));
            }
            let generation = self.next_generation();
            self.video_loop = self.wants_loop();
            self.video.set_loop(self.video_loop);
            self.video.set_volume(self.effective_volume());
            self.video.open(&path, generation, fit);
            self.video_generation = generation;
            self.active = Active::Video;
            self.duration = None;
        } else {
            let mixer = self.ensure_output()?;
            let file = File::open(&path).map_err(|e| format!("Cannot open \"{name}\": {e}"))?;
            // `TryFrom<File>` passes the byte length and seekability to symphonia,
            // which it needs for accurate durations and seeking.
            let source =
                Decoder::try_from(file).map_err(|e| format!("Cannot decode \"{name}\": {e}"))?;
            let duration = source.total_duration().or(fallback);
            let player = Player::connect_new(&mixer);
            player.set_volume(self.effective_volume());
            player.append(source);
            self.active = Active::Audio(player);
            self.duration = duration;
            let track = &mut self.tracks[idx];
            if track.duration_secs.is_none() {
                track.duration_secs = duration.map(|d| d.as_secs_f64().round() as u64);
            }
        }

        self.state = PlaybackState::Playing;
        self.current = Some(idx);
        self.tracks[idx].unavailable = false;
        log::info!("Playing [{idx}] {}", path.display());
        self.events.push(EngineEvent::TrackStarted(idx));
        self.update_restart_hint();
        self.restart_if_requested(idx);
        Ok(())
    }

    /// Play exactly this track; report an error if it fails.
    pub fn play_index(&mut self, idx: usize) {
        self.video_failures = 0;
        if let Err(err) = self.start(idx) {
            self.mark_failed(idx);
            self.stop();
            self.events.push(EngineEvent::Error(err));
        }
    }

    /// Play `start`, skipping over unplayable tracks in the given direction.
    fn play_skipping(&mut self, start: usize, forward: bool) {
        let n = self.tracks.len();
        if n == 0 {
            self.stop();
            return;
        }
        let mut idx = start.min(n - 1);
        let mut last_err = None;
        for _ in 0..n {
            match self.start(idx) {
                Ok(()) => {
                    if let Some(err) = last_err {
                        self.events.push(EngineEvent::Error(err));
                    }
                    return;
                }
                Err(err) => {
                    log::warn!("{err}");
                    self.mark_failed(idx);
                    last_err = Some(err);
                    idx = if forward {
                        (idx + 1) % n
                    } else {
                        (idx + n - 1) % n
                    };
                }
            }
        }
        self.stop();
        if let Some(err) = last_err {
            self.events.push(EngineEvent::Error(format!(
                "No playable tracks. Last error: {err}"
            )));
        }
    }

    fn mark_failed(&mut self, idx: usize) {
        if let Some(t) = self.tracks.get_mut(idx) {
            t.unavailable = true;
        }
    }

    /// Play / pause toggle. When stopped, starts `preferred` (e.g. the selected row),
    /// else the last track, else the first.
    pub fn toggle_play_pause(&mut self, preferred: Option<usize>) {
        match self.state {
            PlaybackState::Playing => self.pause(),
            PlaybackState::Paused => self.resume(),
            PlaybackState::Stopped => {
                if !self.tracks.is_empty() {
                    let idx = preferred.or(self.current).unwrap_or(0);
                    self.video_failures = 0;
                    self.play_skipping(idx, true);
                }
            }
        }
    }

    pub fn pause(&mut self) {
        if self.state != PlaybackState::Playing {
            return;
        }
        match &self.active {
            Active::Audio(player) => player.pause(),
            Active::Video => self.video.pause(),
            Active::None => return,
        }
        self.state = PlaybackState::Paused;
    }

    pub fn resume(&mut self) {
        if self.state != PlaybackState::Paused {
            return;
        }
        match &self.active {
            Active::Audio(player) => player.play(),
            Active::Video => self.video.play(),
            Active::None => return,
        }
        self.state = PlaybackState::Playing;
    }

    fn stop_player(&mut self) {
        match std::mem::replace(&mut self.active, Active::None) {
            Active::Audio(player) => player.stop(),
            Active::Video => self.video.stop(),
            Active::None => {}
        }
        self.pending = None;
    }

    pub fn stop(&mut self) {
        self.stop_player();
        self.state = PlaybackState::Stopped;
    }

    pub fn next(&mut self) {
        if let Some(idx) = self.next_index(false) {
            self.video_failures = 0;
            self.play_skipping(idx, true);
        }
    }

    /// Go to the previous track, or restart the current one if more than
    /// 3 seconds have been played.
    pub fn previous(&mut self) {
        if self.state != PlaybackState::Stopped && self.position() > Duration::from_secs(3) {
            if let Err(err) = self.seek(Duration::ZERO) {
                self.events.push(EngineEvent::Error(err));
            }
            return;
        }
        let n = self.tracks.len();
        if n == 0 {
            return;
        }
        let idx = match self.current {
            Some(0) | None => n - 1,
            Some(c) => (c - 1).min(n - 1),
        };
        self.video_failures = 0;
        self.play_skipping(idx, false);
    }

    fn next_index(&self, auto: bool) -> Option<usize> {
        let n = self.tracks.len();
        if n == 0 {
            return None;
        }
        let Some(cur) = self.current else {
            return Some(0);
        };
        if auto && self.repeat == RepeatMode::One {
            return Some(cur.min(n - 1));
        }
        if cur + 1 < n {
            Some(cur + 1)
        } else if !auto || self.repeat == RepeatMode::All {
            Some(0)
        } else {
            None
        }
    }

    /// The current video should loop on its own (repeat one, or a one-item loop-all).
    fn wants_loop(&self) -> bool {
        self.repeat == RepeatMode::One || (self.repeat == RepeatMode::All && self.tracks.len() == 1)
    }

    pub fn seek(&mut self, pos: Duration) -> Result<(), String> {
        if self.state == PlaybackState::Stopped {
            return Ok(());
        }
        let pos = match self.duration() {
            // Seeking exactly to the end can error on some decoders.
            Some(total) => pos.min(total.saturating_sub(Duration::from_millis(250))),
            None => pos,
        };
        match &self.active {
            Active::Audio(player) => player
                .try_seek(pos)
                .map_err(|e| format!("Seek failed: {e}")),
            Active::Video => {
                self.video.seek(pos.as_secs_f64());
                Ok(())
            }
            Active::None => Ok(()),
        }
    }

    pub fn seek_relative(&mut self, delta_secs: f32) -> Result<(), String> {
        let target = (self.position().as_secs_f32() + delta_secs).max(0.0);
        self.seek(Duration::from_secs_f32(target))
    }

    /// Continue after the current item finished on its own.
    fn advance(&mut self) {
        match self.next_index(true) {
            Some(idx) => self.play_skipping(idx, true),
            None => {
                self.stop();
                self.events.push(EngineEvent::PlaylistEnded);
            }
        }
    }

    /// Called periodically by the watcher thread. Returns true if anything changed.
    fn tick(&mut self) -> bool {
        if self.is_video_active() {
            return self.tick_video();
        }
        if self.state != PlaybackState::Playing {
            return false;
        }
        let finished = match &self.active {
            Active::Audio(player) => player.empty(),
            _ => true,
        };
        if !finished {
            return false;
        }
        self.advance();
        true
    }

    fn tick_video(&mut self) -> bool {
        let status = self.video.status();
        let mut changed = false;

        // Did the video thread switch to the pre-loaded file (gapless)?
        if let Some(p) = &self.pending {
            if status.generation == p.generation {
                let p = self.pending.take().unwrap();
                // The queue may have been edited since; find the file again if needed.
                let idx = if self.tracks.get(p.index).is_some_and(|t| t.path == p.path) {
                    Some(p.index)
                } else {
                    self.tracks.iter().position(|t| t.path == p.path)
                };
                log::info!("Gapless switch to [{idx:?}] {}", p.path.display());
                self.video_generation = p.generation;
                self.current = idx;
                self.state = PlaybackState::Playing;
                if let Some(i) = idx {
                    self.tracks[i].unavailable = false;
                    self.events.push(EngineEvent::TrackStarted(i));
                    self.restart_if_requested(i);
                }
                self.update_restart_hint();
                changed = true;
            }
        }

        if status.generation == self.video_generation {
            if let Some(err) = status.error {
                if let Some(cur) = self.current {
                    self.mark_failed(cur);
                }
                self.events.push(EngineEvent::Error(err));
                self.video_failures += 1;
                if self.video_failures >= self.tracks.len().max(1) {
                    self.stop();
                    self.events.push(EngineEvent::Error(
                        "No playable items in the playlist".into(),
                    ));
                    return true;
                }
                // Skip forward even in repeat-one, or a broken file would retry forever.
                let next = self
                    .current
                    .map_or(0, |c| (c + 1) % self.tracks.len().max(1));
                self.play_skipping(next, true);
                return true;
            }
            if status.position > 0.5 {
                self.video_failures = 0;
            }
            if let (Some(cur), Some(d)) = (self.current, status.duration) {
                if let Some(t) = self.tracks.get_mut(cur) {
                    if t.duration_secs.is_none() {
                        t.duration_secs = Some(d.round() as u64);
                        changed = true;
                    }
                }
            }
            if status.ended && self.state == PlaybackState::Playing {
                self.advance();
                return true;
            }
        }

        self.sync_video_followup();
        changed
    }

    /// Keep the video thread's loop flag and pre-loaded "next video" in line with the
    /// queue and repeat mode.
    fn sync_video_followup(&mut self) {
        if !self.is_video_active() || self.state == PlaybackState::Stopped {
            if self.pending.take().is_some() {
                self.video.preload(None);
            }
            return;
        }
        let looping = self.wants_loop();
        if looping != self.video_loop {
            self.video_loop = looping;
            self.video.set_loop(looping);
        }
        let desired = if looping {
            None
        } else {
            self.next_index(true)
                .filter(|&i| self.is_video_track(i) && !self.tracks[i].unavailable)
                .map(|i| {
                    (
                        i,
                        self.tracks[i].path.clone(),
                        self.tracks[i].fit.unwrap_or(self.fit),
                    )
                })
        };
        match desired {
            Some((index, path, _))
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.index == index && p.path == path) => {}
            Some((index, path, fit)) => {
                let generation = self.next_generation();
                self.video.preload(Some((&path, generation, fit)));
                self.pending = Some(Pending {
                    index,
                    generation,
                    path,
                });
            }
            None => {
                if self.pending.take().is_some() {
                    self.video.preload(None);
                }
            }
        }
    }

    // ------------------------------------------------------------ queue edits

    pub fn set_tracks(&mut self, tracks: Vec<Track>) {
        self.stop();
        self.tracks = tracks;
        self.current = None;
        self.duration = None;
    }

    pub fn add_tracks(&mut self, tracks: impl IntoIterator<Item = Track>) {
        self.tracks.extend(tracks);
    }

    /// Restore which track was current (without playing it).
    pub fn set_current(&mut self, idx: Option<usize>) {
        if self.state == PlaybackState::Stopped {
            self.current = idx.filter(|&i| i < self.tracks.len());
        }
    }

    pub fn remove(&mut self, idx: usize) {
        if idx >= self.tracks.len() {
            return;
        }
        self.tracks.remove(idx);
        match self.current {
            Some(c) if c == idx => {
                self.stop();
                self.current = None;
                self.duration = None;
            }
            Some(c) if c > idx => self.current = Some(c - 1),
            _ => {}
        }
    }

    pub fn move_track(&mut self, from: usize, to: usize) {
        let n = self.tracks.len();
        if from >= n || to >= n || from == to {
            return;
        }
        let track = self.tracks.remove(from);
        self.tracks.insert(to, track);
        self.current = self.current.map(|c| remap_index(c, from, to));
    }

    pub fn clear(&mut self) {
        self.set_tracks(Vec::new());
    }
}

/// Open the default audio device on a dedicated thread that keeps the device
/// sink alive for the rest of the process, and hand back its mixer.
fn open_output() -> Result<Mixer, String> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("audio-output".into())
        .spawn(move || match DeviceSinkBuilder::open_default_sink() {
            Ok(mut device) => {
                device.log_on_drop(false);
                let _ = tx.send(Ok(device.mixer().clone()));
                let _keep_alive = device;
                loop {
                    thread::park();
                }
            }
            Err(e) => {
                let _ = tx.send(Err(format!("No audio output device: {e}")));
            }
        })
        .map_err(|e| format!("Cannot start audio thread: {e}"))?;
    rx.recv()
        .map_err(|_| "Audio thread exited unexpectedly".to_owned())?
}
