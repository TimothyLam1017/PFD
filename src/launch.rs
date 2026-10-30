//! Command-line options.
//!
//! ```text
//! PFD.exe [files | folders | playlist.json ...]
//!     --autostart        started by Windows: fullscreen loop of the last playlist
//!     --resume           continue the saved queue (used after a watchdog restart)
//!     --fullscreen       start fullscreen
//!     --monitor N        fullscreen monitor (0 = main)
//!     --start-at N       queue position to start from (0-based)
//!     --fit fit|fill|stretch
//!     --repeat off|all|one
//! ```

use std::ffi::OsString;
use std::path::PathBuf;

use crate::engine::RepeatMode;
use crate::video::FitMode;

#[derive(Debug, Default, PartialEq)]
pub struct LaunchOptions {
    pub autostart: bool,
    pub resume: bool,
    pub fullscreen: bool,
    pub monitor: Option<usize>,
    pub start_at: Option<usize>,
    pub fit: Option<FitMode>,
    pub repeat: Option<RepeatMode>,
    pub files: Vec<PathBuf>,
}

impl LaunchOptions {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Self {
        let mut opts = Self::default();
        let mut it = args.into_iter();
        while let Some(arg) = it.next() {
            let mut value = || {
                it.next()
                    .and_then(|v| v.into_string().ok())
                    .unwrap_or_default()
            };
            match arg.to_str() {
                Some("--autostart") => opts.autostart = true,
                Some("--resume") => opts.resume = true,
                Some("--fullscreen") => opts.fullscreen = true,
                Some("--monitor") => opts.monitor = value().parse().ok(),
                Some("--start-at") => opts.start_at = value().parse().ok(),
                Some("--fit") => {
                    opts.fit = match value().to_ascii_lowercase().as_str() {
                        "fit" => Some(FitMode::Fit),
                        "fill" => Some(FitMode::Fill),
                        "stretch" => Some(FitMode::Stretch),
                        _ => None,
                    }
                }
                Some("--repeat") => {
                    opts.repeat = match value().to_ascii_lowercase().as_str() {
                        "off" => Some(RepeatMode::Off),
                        "all" => Some(RepeatMode::All),
                        "one" => Some(RepeatMode::One),
                        _ => None,
                    }
                }
                Some(flag) if flag.starts_with("--") => log::warn!("Unknown option {flag}"),
                _ => opts.files.push(PathBuf::from(arg)),
            }
        }
        opts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> LaunchOptions {
        LaunchOptions::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn options() {
        assert_eq!(parse(&[]), LaunchOptions::default());
        let o = parse(&["--autostart"]);
        assert!(o.autostart && o.files.is_empty());
        let o = parse(&[
            "--resume",
            "--start-at",
            "3",
            "--fullscreen",
            "--monitor",
            "1",
            "--fit",
            "FILL",
            "--repeat",
            "one",
            "D:/v/夜曲.mp4",
        ]);
        assert!(o.resume && o.fullscreen);
        assert_eq!(o.start_at, Some(3));
        assert_eq!(o.monitor, Some(1));
        assert_eq!(o.fit, Some(FitMode::Fill));
        assert_eq!(o.repeat, Some(RepeatMode::One));
        assert_eq!(o.files, vec![PathBuf::from("D:/v/夜曲.mp4")]);
        // Bad values are ignored, unknown flags too.
        let o = parse(&["--monitor", "x", "--bogus", "a.mp3"]);
        assert_eq!(o.monitor, None);
        assert_eq!(o.files, vec![PathBuf::from("a.mp3")]);
    }
}
