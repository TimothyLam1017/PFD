#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    // Best-effort single-instance guard (Windows named mutex). Prevents a second
    // overlapping instance after a watchdog restart race; no-op on other OSes.
    if let Err(e) = pfd::acquire_single_instance() {
        log::error!("{e}");
        eprintln!("{e}");
        std::process::exit(1);
    }

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PFD")
            .with_app_id("pfd")
            .with_inner_size([760.0, 560.0])
            .with_min_inner_size([480.0, 340.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "PFD",
        native_options,
        Box::new(|cc| Ok(Box::new(pfd::AudioPlayerApp::new(cc)))),
    )
}
