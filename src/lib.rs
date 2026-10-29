#![warn(clippy::all, rust_2018_idioms)]

mod app;
mod autostart;
mod config;
mod engine;
mod fonts;
mod launch;
mod metadata;
mod playlist;
mod video;

pub use app::AudioPlayerApp;
pub use video::acquire_single_instance;
