#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("audio-player")
            .with_app_id("audio-player")
            .with_inner_size([760.0, 560.0])
            .with_min_inner_size([480.0, 340.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "audio-player",
        native_options,
        Box::new(|cc| Ok(Box::new(audio_player::AudioPlayerApp::new(cc)))),
    )
}
