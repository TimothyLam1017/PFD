//! Tag reading with `lofty`, done on a background thread so that large
//! playlists never block the UI.

use std::borrow::Cow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use eframe::egui;

#[derive(Clone, Debug, Default)]
pub struct TrackMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<Duration>,
}

/// Read title / artist / album / duration from any format lofty understands
/// (ID3v1/ID3v2, Vorbis comments, MP4 ilst, APE, RIFF INFO, ...).
pub fn read(path: &Path) -> Result<TrackMeta, String> {
    use lofty::prelude::*;

    if !path.is_file() {
        return Err("file not found".to_owned());
    }

    let tagged = lofty::read_from_path(path).map_err(|e| e.to_string())?;

    let duration = tagged.properties().duration();
    let duration = (!duration.is_zero()).then_some(duration);

    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let clean = |s: Option<Cow<'_, str>>| {
        s.map(|s| {
            s.trim_matches(|c: char| c.is_whitespace() || c == '\0')
                .to_owned()
        })
        .filter(|s| !s.is_empty())
    };

    Ok(TrackMeta {
        title: tag.and_then(|t| clean(t.title())),
        artist: tag.and_then(|t| clean(t.artist())),
        album: tag.and_then(|t| clean(t.album())),
        duration,
    })
}

pub type MetaResult = (PathBuf, Result<TrackMeta, String>);

/// A single worker thread that reads tags for requested paths.
pub struct MetaLoader {
    requests: Sender<PathBuf>,
    results: Receiver<MetaResult>,
    pending: HashSet<PathBuf>,
}

impl MetaLoader {
    pub fn new(ctx: egui::Context) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<PathBuf>();
        let (res_tx, res_rx) = mpsc::channel::<MetaResult>();

        thread::Builder::new()
            .name("metadata".into())
            .spawn(move || {
                for path in req_rx {
                    let result = read(&path);
                    if res_tx.send((path, result)).is_err() {
                        break;
                    }
                    // Batch UI refreshes instead of repainting once per file.
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
            })
            .expect("failed to spawn metadata thread");

        Self {
            requests: req_tx,
            results: res_rx,
            pending: HashSet::new(),
        }
    }

    pub fn request(&mut self, path: &Path) {
        if self.pending.insert(path.to_path_buf()) {
            let _ = self.requests.send(path.to_path_buf());
        }
    }

    pub fn poll(&mut self) -> Vec<MetaResult> {
        let results: Vec<MetaResult> = self.results.try_iter().collect();
        for (path, _) in &results {
            self.pending.remove(path);
        }
        results
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }
}
