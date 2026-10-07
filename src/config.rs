//! Settings and the auto-saved play queue, stored under the OS config dir:
//!
//! * Windows: `%APPDATA%\audio-player\`
//! * macOS:   `~/Library/Application Support/audio-player/`
//! * Linux:   `~/.config/audio-player/`

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::engine::RepeatMode;
use crate::playlist::{self, Track};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 0..=100
    pub volume: u8,
    pub muted: bool,
    pub repeat: RepeatMode,
    /// The playlist file that "Save" writes to.
    pub last_playlist: Option<PathBuf>,
    /// Whether the queue had unsaved changes relative to `last_playlist`.
    pub queue_dirty: bool,
    pub last_track: Option<usize>,
    /// Last folder used in a file dialog.
    pub last_dir: Option<PathBuf>,
    /// Optional extra font (e.g. a CJK .ttf/.ttc) if the auto-detected one is missing.
    pub font_path: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            volume: 70,
            muted: false,
            repeat: RepeatMode::All,
            last_playlist: None,
            queue_dirty: false,
            last_track: None,
            last_dir: None,
            font_path: None,
        }
    }
}

pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("audio-player"))
}

fn config_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.json"))
}

fn queue_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("queue.json"))
}

impl Config {
    pub fn load() -> Self {
        let Some(path) = config_file() else {
            return Self::default();
        };
        match fs::read_to_string(&path) {
            Ok(text) => {
                serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or_else(|e| {
                    log::warn!("Ignoring invalid config {}: {e}", path.display());
                    Self::default()
                })
            }
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = config_file().ok_or("No config directory on this system")?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&path, json).map_err(|e| format!("Cannot write {}: {e}", path.display()))
    }
}

pub fn load_queue() -> Option<Vec<Track>> {
    let path = queue_file()?;
    if !path.exists() {
        return None;
    }
    playlist::load(&path).map_err(|e| log::warn!("{e}")).ok()
}

pub fn save_queue(tracks: &[Track]) -> Result<(), String> {
    let path = queue_file().ok_or("No config directory on this system")?;
    playlist::save(&path, tracks)
}
