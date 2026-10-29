# A-Player (`audio-player`)

A lightweight native music **and video** player written in Rust, based on the
[eframe template](https://github.com/emilk/eframe_template) layout
(`src/main.rs` + `src/lib.rs` + `src/app.rs`).

It works as an everyday music player, and on Windows also as an unattended video
screen. In that mode it loops a playlist in fullscreen with nothing on screen but the
picture, stays on top of other windows, and can start by itself when Windows starts.

* **GUI:** `eframe`/`egui` 0.31 with the glow (OpenGL) backend, so no webview.
* **Audio:** `rodio` 0.22 with `symphonia-all` (mp3, flac, wav, m4a/aac, ogg/vorbis …).
* **Video (Windows):** Windows Media Foundation through the `windows` crate. Decoding
  runs on the GPU, and there are no extra DLLs to ship.
* **Tags:** `lofty` (ID3v1/v2, Vorbis comments, MP4 atoms, APE …).
* **Other crates:** `serde`/`serde_json`, `rfd` (file dialogs), `dirs` (config path).

The Windows build is a single ~8 MB `audio-player.exe`.

## Build & run

Requires Rust **1.87+** (`rustup update`).

```sh
cargo run --release
# open files / folders / a playlist directly (replaces the queue and starts playing):
cargo run --release -- "D:/Music/夜曲.mp3" "D:/Videos" my-playlist.json
```

The release binary is `target/release/audio-player(.exe)`. On Windows it opens
without a console window.

**Linux** also needs the ALSA headers: `sudo apt install libasound2-dev` (Debian/Ubuntu)
or `alsa-lib-devel` (Fedora). Video playback is Windows-only; elsewhere, video files are
reported as unsupported and skipped.

## Features

### Playback
* Play / Pause / Stop / Previous / Next. Previous restarts the item if more than 3 s
  have played.
* Seek bar with current/total time; click or drag to seek.
* Volume 0–100 % (perceptual curve), mute, and mouse-wheel over the slider.
* Repeat **off / all / one**. Auto-advance keeps working while the window is minimized
  or covered by fullscreen video, because a watcher thread runs the queue.
* Audio and video can be mixed in one playlist.
* Unreadable or missing files are flagged in red (⚠) and skipped automatically.

### Video (Windows)
* **Formats:** mp4, m4v, mov, mkv, wmv, avi, webm, ts, mpg, 3gp.
  * H.264 works out of the box.
  * HEVC, VP9 and AV1 need the matching (free or paid) **Video Extensions** from the
    Microsoft Store; many PCs already have them.
* **In the player window:** the video shows above the playlist. To let the video fill
  the window, untick **Video → Show playlist under the video**.
* **Fullscreen:** double-click the video, press **F**, or use **Video → Fullscreen**.
  * Only the picture is shown: no title, timeline, playback bar, subtitles or mouse cursor.
  * The window stays on top of all others and re-asserts itself every 2 s.
  * The player runs at high priority, and the PC is kept from sleeping, dimming the
    screen or starting the screensaver.
  * Because the window counts as a fullscreen app, Windows' Do Not Disturb holds back
    notification banners.
  * **Double-click the video to leave fullscreen**; the player window comes back.
    Esc does nothing, so stray key presses can't end the show.
  * The first time you enter fullscreen, a brief bottom hint says “Double-click to exit fullscreen”; it is not shown again.
* **Picture shape** (Video menu), applied in the window and in fullscreen:
  * **Fit:** the whole picture at its true shape, with black bars.
  * **Fill:** covers the screen and crops the edges.
  * **Stretch:** fills the screen and distorts the picture.
  * Videos with non-square pixels (e.g. widescreen DVD rips) are corrected automatically.
  * A playlist entry can override the shape with `"fit": "fill"`.
* **Gapless looping:** the next video is loaded in the background, so videos follow
  each other, including back to the first one, without a black gap.
* **Fullscreen monitor:** choose it in the Video menu if you have more than one screen.
* **Keys in fullscreen** (silent): Space play/pause, ← / → seek 5 s,
  Ctrl+← / Ctrl+→ or P / N previous/next, and the media keys.

### Start with Windows
**Options → Start with Windows.** After you sign in, the player opens straight into
fullscreen and loops the last playlist. It starts from the first item, or from the last
item played if you tick **…continue from the last item played**.

* It uses your account's start-up list (`HKCU\…\Run`), so no admin rights are needed.
* It waits up to a minute and a half for the screen, sound device and drives to be ready.
* The queue is saved every time it changes, so a power cut doesn't lose it.

### Unattended-screen safety nets
* **Frozen video decoder:** a watchdog notices within ~25 s, restarts the player and
  continues with the next item, in fullscreen again.
* **Crash:** the player registers with Windows to be relaunched automatically (Windows
  does this for programs that ran at least 60 s), continuing where it was.
* **Memory growth:** if the player ever uses more than `memory_limit_mb` (default 1500),
  it restarts cleanly at the next item boundary.
* **Display change:** if the screen resolution changes, the fullscreen window is
  re-created to match.
* **File with no picture:** a file that shows no picture for 20 s is skipped.

### Playlist & other
* Open/save `.json` playlists, add files or whole folders, drag & drop, reorder,
  remove, and copy a file's path.
* Missing title/artist/duration are filled in from tags in the background; values
  written in the playlist always take priority.
* Chinese/Japanese/Korean text uses a CJK font already installed on the system.
* The queue, volume, repeat mode, picture shape and last playlist are restored on the
  next start.

## Setting up a dedicated video screen (Windows)

1. Put the videos in a folder and build the playlist (or just add the folder), then
   **File → Save playlist**.
2. **Video → Fullscreen monitor**: pick the screen. **Video → Picture shape**: pick one.
3. **Options → Start with Windows**.
4. Recommended Windows settings:
   * **Automatic sign-in:** start-up programs only run after someone signs in. Use
     Microsoft's free [Autologon](https://learn.microsoft.com/sysinternals/downloads/autologon)
     tool so the PC signs in by itself after a reboot or power cut.
   * **Settings → System → Notifications → Do not disturb:** turn it on, or keep the
     automatic rule "When using an app in full-screen mode", which is on by default in
     Windows 11.
   * **Settings → Windows Update → Advanced options → Active hours:** cover the hours the
     screen is in use, so Windows doesn't restart during a show.
   * **Microsoft Store:** install *HEVC*, *VP9* and *AV1 Video Extensions* if your
     videos use those formats.

Windows always keeps a few things on top of any app, so these can still appear:
Ctrl+Alt+Del, admin (UAC) prompts, and critical system alerts.

## Playlist format

A JSON array (UTF-8, with or without a BOM). Only `path` is required:

```json
[
  { "path": "D:/Music/夜曲.mp3", "title": "夜曲", "artist": "周杰倫", "duration_secs": 233 },
  { "path": "/home/user/音楽/さくら.mp3" },
  { "path": "D:/Videos/promo.mp4", "fit": "fill" },
  "a plain string path also works.ogg"
]
```

* Relative paths are resolved against the playlist file's folder.
* `fit` (videos): `fit`, `fill` or `stretch`; unknown values are ignored.
* Saved playlists are pretty-printed and keep non-ASCII text readable.
* See `assets/example-playlist.json`.

## Keyboard shortcuts (player window)

| Key | Action |
| --- | --- |
| Space | Play / pause |
| ← / → | Seek −5 s / +5 s |
| Ctrl+← / Ctrl+→ | Previous / next |
| Ctrl+↑ / Ctrl+↓ | Volume up / down |
| M | Mute |
| F | Fullscreen video |
| ↑ / ↓ | Move the selection |
| Enter / double-click | Play the selected item |
| Alt+↑ / Alt+↓ | Move the selected item up / down |
| Delete | Remove the selected item |
| Ctrl+N / Ctrl+O / Ctrl+S / Ctrl+Shift+S | New / open / save / save-as playlist |

## Command line

```text
audio-player.exe [files | folders | playlist.json ...]
    --autostart          what "Start with Windows" runs: fullscreen loop of the last playlist
    --resume             continue the saved queue (used by the automatic restarts)
    --fullscreen         start in fullscreen
    --monitor N          fullscreen monitor (0 = main)
    --start-at N         queue position to start from (0 = first)
    --fit fit|fill|stretch
    --repeat off|all|one
```

Example shortcut for a lobby screen:
`audio-player.exe --fullscreen --repeat all "D:\Signage\show.json"`

## Config

Stored in `dirs::config_dir()/audio-player/`
(Windows `%APPDATA%\audio-player\`, macOS `~/Library/Application Support/audio-player/`,
Linux `~/.config/audio-player/`):

* `config.json`
  * Volume, mute, repeat, last playlist and folder.
  * `fit` and `fullscreen_monitor`.
  * `show_playlist` and `autostart_resume`.
  * `memory_limit_mb` (0 = never restart for memory).
  * `font_path`: a `.ttf`/`.ttc` to use if CJK text shows as boxes.
* `queue.json`: the current queue, saved on every change.

## Diagnostics

`examples/video_probe.rs` is a standalone test program for Windows video playback. It
plays files fullscreen and writes `video-probe.log` (formats, picture sizes, CPU use,
frame rate, notification state). Build it with
`cargo build --release --example video_probe`.

## Project layout

```
src/
  main.rs         native entry point (window options)
  lib.rs          module list, re-exports AudioPlayerApp
  app.rs          egui UI: menus, video area, transport, playlist, shortcuts, start-up
  engine.rs       play queue + audio (rodio) / video backends, repeat, auto-advance, gapless
  video/mod.rs    video API, aspect-ratio maths, watchdog / restarts
  video/win.rs    Windows video thread: Media Foundation + Direct3D 11, fullscreen window
  video/stub.rs   non-Windows stand-in
  autostart.rs    "Start with Windows" (HKCU Run key)
  launch.rs       command-line options
  playlist.rs     Track model, JSON load/save, folder scanning
  metadata.rs     lofty tag reading on a background thread
  config.rs       settings + queue persistence (dirs)
  fonts.rs        system CJK fallback font loading
examples/
  video_probe.rs  standalone video diagnostics
```
