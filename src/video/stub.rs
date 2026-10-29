//! Stand-in when `video-mpv` is disabled: every video reports that libmpv is required.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;

use super::{lock, Cmd, Shared};

pub(super) fn run(rx: Receiver<Cmd>, shared: Arc<Shared>, ctx: egui::Context) {
    loop {
        shared.beat();
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Cmd::Open { generation, .. }) => {
                let mut s = lock(&shared.status);
                if s.generation == generation {
                    s.error = Some(
                        "Video playback on this platform requires the `video-mpv` feature and system libmpv"
                            .to_owned(),
                    );
                }
                drop(s);
                ctx.request_repaint();
            }
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}
