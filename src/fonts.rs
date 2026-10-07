//! egui's built-in fonts only cover Latin/Greek/Cyrillic plus icons, so Chinese,
//! Japanese and Korean titles would render as boxes. We load a CJK font that is
//! already installed on the system and register it as a fallback.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

fn candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();

    if cfg!(target_os = "windows") {
        let fonts = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("Fonts");
        // Microsoft JhengHei (Traditional), YaHei (Simplified) — both include kana.
        for name in [
            "msjh.ttc",
            "msyh.ttc",
            "YuGothM.ttc",
            "meiryo.ttc",
            "simsun.ttc",
            "mingliub.ttc",
        ] {
            out.push(fonts.join(name));
        }
    } else if cfg!(target_os = "macos") {
        for p in [
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        ] {
            out.push(p.into());
        }
    } else {
        for p in [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf",
            "/usr/share/fonts/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        ] {
            out.push(p.into());
        }
    }
    out
}

/// Korean is not covered by most Chinese/Japanese fonts; add one if present.
fn korean_candidates() -> Vec<PathBuf> {
    if cfg!(target_os = "windows") {
        let fonts = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("Fonts");
        vec![fonts.join("malgun.ttf")]
    } else if cfg!(target_os = "macos") {
        vec!["/System/Library/Fonts/AppleSDGothicNeo.ttc".into()]
    } else {
        // Noto Sans CJK already covers Hangul.
        Vec::new()
    }
}

fn add_fallback(fonts: &mut FontDefinitions, name: &str, path: &Path) -> bool {
    match std::fs::read(path) {
        Ok(bytes) => {
            // Index 0 of a .ttc collection is the regular face.
            fonts
                .font_data
                .insert(name.to_owned(), Arc::new(FontData::from_owned(bytes)));
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push(name.to_owned());
            }
            log::info!("Loaded fallback font {}", path.display());
            true
        }
        Err(_) => false,
    }
}

/// Install system CJK fallback fonts. `user_font` (from config) is tried first.
pub fn install(ctx: &egui::Context, user_font: Option<&Path>) {
    let mut fonts = FontDefinitions::default();
    let mut loaded = false;

    if let Some(path) = user_font {
        loaded |= add_fallback(&mut fonts, "user-font", path);
    }
    if !loaded {
        loaded = candidates()
            .iter()
            .any(|p| add_fallback(&mut fonts, "system-cjk", p));
    }
    if let Some(p) = korean_candidates().iter().find(|p| p.exists()) {
        add_fallback(&mut fonts, "system-korean", p);
    }
    if !loaded {
        log::warn!("No CJK font found; set \"font_path\" in config.json to a .ttf/.ttc file");
    }
    ctx.set_fonts(fonts);
}
