# audio-player

A lightweight native music player written in Rust, based on the
[eframe template](https://github.com/emilk/eframe_template) layout
(`src/main.rs` + `src/lib.rs` + `src/app.rs`).

* **GUI:** `eframe`/`egui` 0.31 with the glow (OpenGL) backend, so no webview.
* **Audio:** `rodio` 0.22 with the `symphonia-all` feature (mp3, flac, wav, m4a/aac, ogg/vorbis, plus alac/aiff/mkv).
* **Tags:** `lofty` (ID3v1/v2, Vorbis comments, MP4 atoms, APE …).
* **Other crates:** `serde`/`serde_json` (playlists and config), `rfd` (native file dialogs), `dirs` (config path).

## Build & run

Requires Rust **1.87+** (`rustup update`).

```sh
cargo run --release
# or open files / folders / a playlist directly (replaces the queue and starts playing):
cargo run --release -- "D:/Music/夜曲.mp3" my-playlist.json
```

The release binary is `target/release/audio-player(.exe)`. It is a single
executable, and on Windows it opens without a console window.

**Linux** also needs the ALSA headers: `sudo apt install libasound2-dev` (Debian/Ubuntu) or `alsa-lib-devel` (Fedora).

## Features

* Play / Pause / Stop / Previous / Next. Previous restarts the track if more than 3 s have played.
* Seek bar with current/total time; click or drag to seek.
* Volume 0–100 % (perceptual curve), mute, and mouse-wheel over the slider.
* Auto-advance to the next track, with repeat **off / all / one**. It keeps working
  while the window is minimized, because a watcher thread runs the playback queue instead of the UI loop.
* Unreadable or missing files are flagged in red (⚠) and skipped automatically.
* Playlist: open/save `.json`, add files, add a folder (recursive), drag & drop
  files/folders/playlists onto the window, reorder, remove, and copy the path.
* Missing title/artist/duration are filled from tags in a background thread;
  values written in the playlist always take priority.
* Chinese/Japanese/Korean text: a CJK font installed on the system is
  loaded automatically (Microsoft JhengHei/YaHei, Yu Gothic, Malgun Gothic on Windows;
  PingFang/Hiragino on macOS; Noto CJK/WenQuanYi on Linux).
* The queue, volume, repeat mode and last playlist are restored on the next start.

## Playlist format

A JSON array (UTF-8, with or without a BOM). Only `path` is required:

```json
[
  { "path": "D:/Music/夜曲.mp3", "title": "夜曲", "artist": "周杰倫", "duration_secs": 233 },
  { "path": "/home/user/音楽/さくら.mp3" },
  { "path": "albums/track01.flac", "album": "Optional album" },
  "a plain string path also works.ogg"
]
```

* Relative paths are resolved against the playlist file's folder.
* Saved playlists are pretty-printed and keep non-ASCII text readable (not `\u` escaped).
* See `assets/example-playlist.json`.

## Keyboard shortcuts

| Key | Action |
| --- | --- |
| Space | Play / pause |
| ← / → | Seek −5 s / +5 s |
| Ctrl+← / Ctrl+→ | Previous / next track |
| Ctrl+↑ / Ctrl+↓ | Volume up / down |
| M | Mute |
| ↑ / ↓ | Move the selection |
| Enter | Play the selected track |
| Double-click | Play a track |
| Alt+↑ / Alt+↓ | Move the selected track up / down |
| Delete | Remove the selected track |
| Ctrl+N / Ctrl+O / Ctrl+S / Ctrl+Shift+S | New / open / save / save-as playlist |

(On macOS, Cmd replaces Ctrl.)

## Config

Stored in `dirs::config_dir()/audio-player/`:

| OS | Folder |
| --- | --- |
| Windows | `%APPDATA%\audio-player\` |
| macOS | `~/Library/Application Support/audio-player/` |
| Linux | `~/.config/audio-player/` |

* `config.json` holds volume, mute, repeat, last playlist, last folder, and `font_path`.
* `queue.json` is the current queue, saved automatically on exit.

If CJK text shows as boxes, set `"font_path"` in `config.json` to any `.ttf`/`.ttc` font that covers it.

## Project layout

```
src/
  main.rs      native entry point (window options)
  lib.rs       module list, re-exports AudioPlayerApp
  app.rs       egui UI: menus, transport, playlist table, shortcuts, drag & drop
  engine.rs    rodio playback engine, watcher thread, repeat / auto-advance
  playlist.rs  Track model, JSON load/save, folder scanning
  metadata.rs  lofty tag reading on a background thread
  config.rs    settings + queue persistence (dirs)
  fonts.rs     system CJK fallback font loading
```
