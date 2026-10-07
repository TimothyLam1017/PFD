//! Playback engine built on `rodio`.
//!
//! The engine owns the play queue and lives behind an `Arc<Mutex<_>>`. A small
//! watcher thread checks every 150 ms whether the current track has finished and
//! starts the next one. That keeps auto-advance working even while the window is
//! minimized (eframe does not call `update` for minimized windows on every platform).
//!
//! The rodio device sink (`MixerDeviceSink`) is created on, and kept alive by, its own thread, so
//! audio-device COM initialization on Windows never clashes with the UI thread.

use std::fs::File;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use eframe::egui;
use rodio::mixer::Mixer;
use rodio::{Decoder, DeviceSinkBuilder, Player, Source};
use serde::{Deserialize, Serialize};

use crate::playlist::{remap_index, Track};

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

pub struct Engine {
    output: Option<Mixer>,
    player: Option<Player>,
    state: PlaybackState,
    tracks: Vec<Track>,
    current: Option<usize>,
    duration: Option<Duration>,
    volume: f32,
    muted: bool,
    pub repeat: RepeatMode,
    events: Vec<EngineEvent>,
}

impl Engine {
    /// Create the engine and start its watcher thread.
    pub fn spawn(ctx: egui::Context) -> SharedEngine {
        let engine = Arc::new(Mutex::new(Engine {
            output: None,
            player: None,
            state: PlaybackState::Stopped,
            tracks: Vec::new(),
            current: None,
            duration: None,
            volume: 0.7,
            muted: false,
            repeat: RepeatMode::default(),
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

    pub fn take_events(&mut self) -> Vec<EngineEvent> {
        std::mem::take(&mut self.events)
    }

    /// Current playback position (zero when stopped).
    pub fn position(&self) -> Duration {
        let pos = match (&self.player, self.state) {
            (Some(player), PlaybackState::Playing | PlaybackState::Paused) => player.get_pos(),
            _ => Duration::ZERO,
        };
        match self.duration() {
            Some(total) => pos.min(total),
            None => pos,
        }
    }

    /// Total length of the current track: from the decoder if it knows it,
    /// otherwise from the tags / playlist.
    pub fn duration(&self) -> Option<Duration> {
        self.duration.or_else(|| {
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
        if let Some(player) = &self.player {
            player.set_volume(self.effective_volume());
        }
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

    // --------------------------------------------------------------- playback

    fn ensure_output(&mut self) -> Result<Mixer, String> {
        if let Some(mixer) = &self.output {
            return Ok(mixer.clone());
        }
        let mixer = open_output()?;
        self.output = Some(mixer.clone());
        Ok(mixer)
    }

    /// Start playing track `idx`. Returns an error message if it can't be played.
    fn start(&mut self, idx: usize) -> Result<(), String> {
        let track = self.tracks.get(idx).ok_or("No such track")?;
        let path = track.path.clone();
        let fallback = track.duration_secs.map(Duration::from_secs);
        let name = track.display_title().into_owned();

        self.stop_player();
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

        self.player = Some(player);
        self.state = PlaybackState::Playing;
        self.current = Some(idx);
        self.duration = duration;

        let track = &mut self.tracks[idx];
        track.unavailable = false;
        if track.duration_secs.is_none() {
            track.duration_secs = duration.map(|d| d.as_secs_f64().round() as u64);
        }
        self.events.push(EngineEvent::TrackStarted(idx));
        Ok(())
    }

    /// Play exactly this track; report an error if it fails.
    pub fn play_index(&mut self, idx: usize) {
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
        if let Err(err) = self.ensure_output() {
            self.stop();
            self.events.push(EngineEvent::Error(err));
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
                    self.play_skipping(idx, true);
                }
            }
        }
    }

    pub fn pause(&mut self) {
        if let (Some(player), PlaybackState::Playing) = (&self.player, self.state) {
            player.pause();
            self.state = PlaybackState::Paused;
        }
    }

    pub fn resume(&mut self) {
        if let (Some(player), PlaybackState::Paused) = (&self.player, self.state) {
            player.play();
            self.state = PlaybackState::Playing;
        }
    }

    fn stop_player(&mut self) {
        if let Some(player) = self.player.take() {
            player.stop();
        }
    }

    pub fn stop(&mut self) {
        self.stop_player();
        self.state = PlaybackState::Stopped;
    }

    pub fn next(&mut self) {
        if let Some(idx) = self.next_index(false) {
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

    pub fn seek(&mut self, pos: Duration) -> Result<(), String> {
        let Some(player) = &self.player else {
            return Ok(());
        };
        if self.state == PlaybackState::Stopped {
            return Ok(());
        }
        let pos = match self.duration() {
            // Seeking exactly to the end can error on some decoders.
            Some(total) => pos.min(total.saturating_sub(Duration::from_millis(250))),
            None => pos,
        };
        player
            .try_seek(pos)
            .map_err(|e| format!("Seek failed: {e}"))
    }

    pub fn seek_relative(&mut self, delta_secs: f32) -> Result<(), String> {
        let target = (self.position().as_secs_f32() + delta_secs).max(0.0);
        self.seek(Duration::from_secs_f32(target))
    }

    /// Called periodically by the watcher thread. Returns true if anything changed.
    fn tick(&mut self) -> bool {
        if self.state != PlaybackState::Playing {
            return false;
        }
        let finished = self.player.as_ref().is_none_or(Player::empty);
        if !finished {
            return false;
        }
        match self.next_index(true) {
            Some(idx) => self.play_skipping(idx, true),
            None => {
                self.stop();
                self.events.push(EngineEvent::PlaylistEnded);
            }
        }
        true
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
