//! Playlist model and the on-disk JSON format.
//!
//! A playlist file is a JSON array. Every entry needs a `path`; everything else
//! is optional and is filled in from the file's tags when missing:
//!
//! ```json
//! [
//!   { "path": "D:/Music/夜曲.mp3", "title": "夜曲", "artist": "周杰倫", "duration_secs": 233 },
//!   { "path": "/home/user/音楽/さくら.mp3" },
//!   "relative/paths/work/too.flac",
//!   { "path": "D:/Videos/promo.mp4", "fit": "fill" }
//! ]
//! ```
//!
//! Relative paths are resolved against the folder that contains the playlist.
//! `fit` (videos only) overrides the aspect mode for that entry: `fit`, `fill` or `stretch`.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};

use crate::metadata::TrackMeta;
use crate::video::{self, FitMode};

/// Audio file extensions (played by rodio / symphonia).
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "wav", "m4a", "aac", "ogg", "oga"];

/// Every extension offered in dialogs and accepted when scanning folders or dropping
/// files: audio everywhere, plus video on Windows.
pub fn supported_extensions() -> Vec<&'static str> {
    let mut all = AUDIO_EXTENSIONS.to_vec();
    if video::AVAILABLE {
        all.extend_from_slice(video::VIDEO_EXTENSIONS);
    } else {
        all.push("mp4"); // audio track of an .mp4 still plays through symphonia
    }
    all
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Track {
    pub path: PathBuf,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_secs"
    )]
    pub duration_secs: Option<u64>,

    /// Videos only: aspect mode for this entry (`fit`, `fill`, `stretch`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_fit"
    )]
    pub fit: Option<FitMode>,

    /// Runtime only: the file is missing or could not be decoded.
    #[serde(skip)]
    pub unavailable: bool,
}

impl Track {
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            ..Default::default()
        }
    }

    /// Title from metadata, falling back to the file name without extension.
    pub fn display_title(&self) -> Cow<'_, str> {
        if let Some(title) = &self.title {
            return Cow::Borrowed(title);
        }
        match self.path.file_stem() {
            Some(stem) => stem.to_string_lossy(),
            None => self.path.to_string_lossy(),
        }
    }

    pub fn display_artist(&self) -> &str {
        self.artist.as_deref().unwrap_or("")
    }

    /// Fill in fields that the playlist did not provide. Values that came from
    /// the playlist file always win over tag values.
    pub fn fill_from(&mut self, meta: &TrackMeta) {
        if self.title.is_none() {
            self.title.clone_from(&meta.title);
        }
        if self.artist.is_none() {
            self.artist.clone_from(&meta.artist);
        }
        if self.album.is_none() {
            self.album.clone_from(&meta.album);
        }
        if self.duration_secs.is_none() {
            self.duration_secs = meta.duration.map(|d| d.as_secs_f64().round() as u64);
        }
    }

    fn normalize(&mut self) {
        for field in [&mut self.title, &mut self.artist, &mut self.album] {
            if field.as_deref().is_some_and(|s| s.trim().is_empty()) {
                *field = None;
            }
        }
    }
}

/// Accept `233`, `233.4` or `null` for `duration_secs`.
fn deserialize_secs<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Num {
        Int(u64),
        Float(f64),
    }
    Ok(match Option::<Num>::deserialize(d)? {
        Some(Num::Int(n)) => Some(n),
        Some(Num::Float(f)) if f.is_finite() && f >= 0.0 => Some(f.round() as u64),
        _ => None,
    })
}

/// Accept `"fit"` / `"fill"` / `"stretch"` in any case; anything else is ignored rather
/// than making the whole playlist unreadable.
fn deserialize_fit<'de, D: Deserializer<'de>>(d: D) -> Result<Option<FitMode>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(
        match value
            .as_ref()
            .and_then(|v| v.as_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("fit") | Some("contain") => Some(FitMode::Fit),
            Some("fill") | Some("cover") | Some("crop") => Some(FitMode::Fill),
            Some("stretch") => Some(FitMode::Stretch),
            _ => None,
        },
    )
}

/// An entry may be a full object or just a path string.
#[derive(Deserialize)]
#[serde(untagged)]
enum Entry {
    Path(PathBuf),
    Track(Track),
}

pub fn is_supported_media(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            supported_extensions()
                .iter()
                .any(|s| s.eq_ignore_ascii_case(ext))
        })
}

pub fn is_playlist_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
}

/// Load a playlist. Accepts UTF-8 with or without a BOM (Windows Notepad adds one).
pub fn load(path: &Path) -> Result<Vec<Track>, String> {
    let bytes = fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let text =
        String::from_utf8(bytes).map_err(|_| format!("{} is not valid UTF-8", path.display()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);

    let entries: Vec<Entry> = serde_json::from_str(text)
        .map_err(|e| format!("Invalid playlist {}: {e}", path.display()))?;

    let base = path.parent();
    Ok(entries
        .into_iter()
        .map(|entry| {
            let mut track = match entry {
                Entry::Path(p) => Track::from_path(p),
                Entry::Track(t) => t,
            };
            track.normalize();
            if is_relative_path(&track.path) {
                if let Some(base) = base {
                    track.path = base.join(&track.path);
                }
            }
            track
        })
        .collect())
}

/// A path that should be resolved against the playlist's folder. Paths that are
/// absolute on *any* OS (`/home/...`, `D:/...`, `\\server\share`) are left untouched,
/// so a playlist shared between Windows and Linux is never rewritten on save.
fn is_relative_path(path: &Path) -> bool {
    if path.is_absolute() || path.has_root() {
        return false;
    }
    let s = path.to_string_lossy();
    let b = s.as_bytes();
    let windows_drive = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    let unc = s.starts_with("\\\\");
    !(windows_drive || unc)
}

/// Save a playlist as pretty-printed UTF-8 JSON (non-ASCII is written as-is, not escaped).
/// Writes to a temporary file first so a crash never leaves a half-written playlist.
pub fn save(path: &Path, tracks: &[Track]) -> Result<(), String> {
    let mut json = serde_json::to_string_pretty(tracks).map_err(|e| e.to_string())?;
    json.push('\n');

    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| format!("Cannot write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("Cannot write {}: {e}", path.display())
    })
}

/// Recursively collect supported audio files under `dir`, sorted by path.
pub fn scan_dir(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() && depth < 32 {
                walk(&path, depth + 1, out);
            } else if file_type.is_file() && is_supported_media(&path) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, 0, &mut out);
    out.sort();
    out
}

/// Where index `idx` ends up after the element at `from` is moved to `to`.
pub fn remap_index(idx: usize, from: usize, to: usize) -> usize {
    if idx == from {
        to
    } else if from < idx && idx <= to {
        idx - 1
    } else if to <= idx && idx < from {
        idx + 1
    } else {
        idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_example_playlist() {
        let dir = std::env::temp_dir().join(format!("audio-player-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("list.json");
        fs::write(
            &file,
            "\u{feff}[\n  { \"path\": \"D:/Music/夜曲.mp3\", \"title\": \"夜曲\", \"artist\": \"周杰倫\", \"duration_secs\": 233 },\n  { \"path\": \"/home/user/音楽/さくら.mp3\" },\n  { \"path\": \"/x.flac\", \"title\": \"\", \"duration_secs\": 61.6 },\n  \"rel/song.ogg\"\n]",
        )
        .unwrap();

        let tracks = load(&file).unwrap();
        assert_eq!(tracks.len(), 4);
        assert_eq!(tracks[0].title.as_deref(), Some("夜曲"));
        assert_eq!(tracks[0].artist.as_deref(), Some("周杰倫"));
        assert_eq!(tracks[0].duration_secs, Some(233));
        assert_eq!(tracks[1].display_title(), "さくら");
        assert!(tracks[1].artist.is_none());
        assert_eq!(tracks[2].title, None);
        assert_eq!(tracks[2].duration_secs, Some(62));
        assert_eq!(tracks[3].path, dir.join("rel/song.ogg"));
        assert_eq!(tracks[0].path, PathBuf::from("D:/Music/夜曲.mp3"));
        assert_eq!(tracks[1].path, PathBuf::from("/home/user/音楽/さくら.mp3"));

        // Round trip keeps non-ASCII text unescaped.
        let out = dir.join("out.json");
        save(&out, &tracks[..2]).unwrap();
        let text = fs::read_to_string(&out).unwrap();
        assert!(text.contains("周杰倫"));
        assert!(!text.contains("unavailable"));
        assert_eq!(load(&out).unwrap().len(), 2);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn per_entry_fit() {
        let dir = std::env::temp_dir().join(format!("audio-player-fit-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("videos.json");
        fs::write(
            &file,
            r#"[ { "path": "/v/a.mp4", "fit": "FILL" },
                 { "path": "/v/b.mkv", "fit": "zoom-ish" },
                 { "path": "/v/c.mov", "fit": "stretch" },
                 { "path": "/v/d.mp4" } ]"#,
        )
        .unwrap();
        let tracks = load(&file).unwrap();
        assert_eq!(
            tracks.len(),
            4,
            "an unknown fit value must not break the playlist"
        );
        assert_eq!(tracks[0].fit, Some(FitMode::Fill));
        assert_eq!(tracks[1].fit, None);
        assert_eq!(tracks[2].fit, Some(FitMode::Stretch));
        assert_eq!(tracks[3].fit, None);

        let out = dir.join("out.json");
        save(&out, &tracks).unwrap();
        let text = fs::read_to_string(&out).unwrap();
        assert!(text.contains(r#""fit": "fill""#));
        assert_eq!(text.matches("\"fit\"").count(), 2);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn remap() {
        // [a b c d], move 0 -> 2 gives [b c a d]
        assert_eq!(remap_index(0, 0, 2), 2);
        assert_eq!(remap_index(1, 0, 2), 0);
        assert_eq!(remap_index(2, 0, 2), 1);
        assert_eq!(remap_index(3, 0, 2), 3);
        // move 3 -> 1 gives [a d b c]
        assert_eq!(remap_index(1, 3, 1), 2);
        assert_eq!(remap_index(3, 3, 1), 1);
        assert_eq!(remap_index(0, 3, 1), 0);
    }
}
