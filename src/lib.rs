#![warn(clippy::all, rust_2018_idioms)]

mod app;
mod config;
mod engine;
mod fonts;
mod metadata;
mod playlist;

pub use app::AudioPlayerApp;
