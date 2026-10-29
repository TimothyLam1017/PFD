# PFD (A-Player) Acceptance Checklist

**Product name:** **PFD** (*Player for dummies*)  
**Code / crate / exe / Start Menu:** today `audio-player` — gradually unify branding to **PFD** (曲街) · repo `TimothyLam1017/A-Player`  
**Baseline tip:** `910db58` (2026-10-29 HKT; update when re-baselining)  
**Scope (Timothy):** everyday **music player** + **unattended video screen** (both)  
**UI language:** English only (menus, errors, dialogs)  
**Owner:** 曲質 (QA) · **Dev:** 曲街 · **UX:** 曲介  

**Legend:** `[ ]` not run · `[P]` pass · `[F]` fail (attach log + repro) · `[B]` blocked · `[D]` deferred · `[N/A]` out of scope  

> Pass/fail source of truth. Product prefs → escalate to Timothy via 旺財. Do not invent fail criteria for owner-decided soft policies below.

### Owner policy (2026-10-29 via 旺財)

| Topic | Decision |
|-------|----------|
| Use cases | Music **and** unattended video screen |
| Product name | **PFD** |
| Video on Linux / macOS | **Required**; backend **TBD** (曲街經旺財再問). Today: Windows MF only; Linux/mac stub = **feature gap** |
| Distribution | **Installer** required on **Windows, Linux, and macOS** (not raw binary only; no Win-only short-term) |
| Leave fullscreen | **Double-click video only** (Esc must not exit) — hard requirement |
| HEVC / VP9 / AV1 | Need **install guide** for Microsoft Store Video Extensions; missing codec → clear message + guide, not silent fail |
| Crash / decoder hang / memory restart | **Best-effort recovery** — record caution if recovery works; hang/crash **without** recovery path = `[F]` |
| Checklist push | Timothy approved 2026-10-29 via 旺財 |

---

## 0. Build / install / first launch

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| E01 | `cargo build --release` on Windows | Produces `target/release/audio-player.exe` (~8 MB class) | [ ] | Rust 1.87+ |
| E02 | `cargo build --release` on Linux | Binary builds; ALSA deps documented | [ ] | `libasound2-dev` |
| E03 | `cargo build --release` on macOS | Binary builds | [ ] | |
| E04 | **Installer** (Windows) | Installs app, Start Menu / shortcut, uninstall clean | [ ] | **Gap if missing** — product requires installer |
| E05 | Installer (Linux / macOS) | Installs cleanly (pkg/AppImage/dmg or agreed form); uninstall/docs OK | [ ] | Timothy：Win／Linux／mac **都要** installer（唔接受 short-term 只 Win） |
| E06 | First launch UI language | All visible chrome **English** | [ ] | |
| E07 | CLI open files / folders / playlist.json | Queue replaced; playback starts | [ ] | |
| E08 | Config dir created | `%APPDATA%\audio-player\` (Win) / platform `dirs::config_dir` | [ ] | `config.json`, `queue.json` |

---

## 1. Music player (all platforms)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| M01 | Play / Pause / Stop | Works; UI state matches | [ ] | |
| M02 | Previous / Next | Prev restarts item if >3 s played; else previous item | [ ] | |
| M03 | Seek bar | Click/drag seeks; time labels update | [ ] | |
| M04 | Volume / mute / wheel on slider | 0–100 % perceptual; mute toggles | [ ] | |
| M05 | Repeat off / all / one | Behaviour matches mode | [ ] | |
| M06 | Formats: mp3, flac, wav, m4a/aac, ogg | Each plays without crash | [ ] | |
| M07 | Missing / unreadable file | Red ⚠; auto-skipped; queue continues | [ ] | |
| M08 | Playlist JSON load/save | BOM OK; relative paths; non-ASCII preserved | [ ] | Unit tests exist — still smoke on host |
| M09 | Folder add / DnD / reorder / delete | Queue updates; persisted | [ ] | |
| M10 | Tags fill title/artist/duration in background | Empty fields fill; playlist values win | [ ] | |
| M11 | CJK titles | Render (system CJK font or `font_path`) | [ ] | |
| M12 | Restore session | Volume, repeat, queue, last playlist restored | [ ] | |
| M13 | Shortcuts (window) | Space, arrows, Ctrl+arrows, M, Ctrl+N/O/S per README | [ ] | |

---

## 2. Video — Windows (Media Foundation)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| V01 | H.264 mp4 in window | Picture above playlist; audio sync OK | [ ] | Baseline codec |
| V02 | Hide playlist under video | Video fills player window | [ ] | |
| V03 | Fit / Fill / Stretch | Applies in window + fullscreen; per-entry `fit` overrides | [ ] | |
| V04 | Non-square pixels | Display aspect corrected (DVD-style) | [ ] | |
| V05 | Gapless next video | No long black gap between items / loop to first | [ ] | Record gap if >~0.5 s as caution |
| V06 | Mixed audio+video playlist | Advances correctly across types | [ ] | |
| V07 | Unsupported / no-picture file | Skip after ~20 s no picture; clear handling | [ ] | |
| V08 | HEVC / VP9 / AV1 | With Store extensions: play; without: **clear error + install guide** | [ ] | Guide required (Timothy) |
| V09 | `video_probe` example | Builds; writes `video-probe.log` with format/size/FPS notes | [ ] | Diagnostics |

---

## 3. Video — Linux / macOS (**required by Timothy**)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| L01 | Play common video (e.g. H.264 mp4) on Linux | Plays in-app (not “unsupported skip-only”) | [ ] | **Current stub = gap**; backend TBD |
| L02 | Play common video on macOS | Same | [ ] | **Current stub = gap** |
| L03 | Fullscreen / fit modes on Linux/mac | Parity with Win behaviour where feasible | [ ] | May trail Win; document deltas |
| L04 | Until backend ships | README + UI state “video not available” is honest; checklist stays `[B]`/`[ ]` not fake `[P]` | [ ] | |

---

## 4. Fullscreen unattended screen (Windows primary)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| F01 | Enter fullscreen (F / menu / double-click) | Picture only: no chrome, timeline, subtitles, cursor | [ ] | |
| F02 | Topmost + re-assert ~2 s | Stays above normal apps | [ ] | UAC/CAD may still cover |
| F03 | High priority + no sleep/dim/screensaver | Display stays on during show | [ ] | |
| F04 | **Exit only by double-click on video** | Esc / stray keys do **not** exit | [ ] | **Hard** (Timothy) |
| F05 | Silent keys in fullscreen | Space, ←/→, Ctrl+←/→, P/N, media keys work | [ ] | |
| F06 | Monitor picker | Fullscreen on chosen monitor | [ ] | Multi-monitor |
| F07 | Lobby shortcut | `audio-player.exe --fullscreen --repeat all playlist.json` | [ ] | |

---

## 5. Start with Windows / resume

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| A01 | Options → Start with Windows | HKCU Run written; no admin | [ ] | |
| A02 | Autostart after sign-in | Opens fullscreen; loops last playlist | [ ] | Needs Autologon for power-cut E2E (OS) |
| A03 | Continue from last item | Toggle respected | [ ] | |
| A04 | Wait for devices (~90 s) | Survives slow disk/audio/display ready | [ ] | |
| A05 | Queue saved on change | Power cut does not empty show list | [ ] | |
| A06 | OS checklist (manual) | Autologon / DND / Active hours / codec extensions documented | [ ] | Not app fail if user skips OS setup |

---

## 6. Stability / recovery (best-effort policy)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| S01 | Decoder hang ~20–25 s | Watchdog restarts process; continues next item; fullscreen restored if was FS | [ ] | Best-effort: recovery OK = pass/caution |
| S02 | `RegisterApplicationRestart` | After ≥60 s run, OS relaunch continues queue | [ ] | Best-effort |
| S03 | `memory_limit_mb` (default 1500) | Restart at next item; force within ~60 s if no boundary | [ ] | 0 = disabled |
| S04 | Display mode change | Fullscreen window recreated to new mode | [ ] | |
| S05 | Recovery **fails** (no restart / stuck black / wrong index) | `[F]` + repro | [ ] | Exception to soft policy |
| S06 | Minimized / covered window | Auto-advance still works (watcher thread) | [ ] | |

---

## 7. Diagnostics & repro (required for every `[F]`)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| D01 | Log location documented | Fixed path for release builds (no console) — **recommend** `%APPDATA%\audio-player\logs\` | [ ] | **Gap if only stdout** |
| D02 | Fail packet | SHA, OS build, playlist, CLI flags, codec, monitor count, log + `video-probe.log` | [ ] | |
| D03 | Repro template used | See below | [ ] | |

### Repro template

```
Product: PFD (A-Player)
Commit:
OS / build:
Mode: music | windowed video | fullscreen | autostart
CLI:
Playlist / sample files (codec):
Monitors:
Steps:
1.
2.
Expected:
Actual:
Log / video-probe path:
Recovery observed? (watchdog / OS restart / none):
```

---

## 8. Engineering P0 reliability (align 曲街)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| R01 | CI | At least `cargo test` + `cargo check` on PR/main | [ ] | No `.github` at `910db58` — open work |
| R02 | Packaging / installer artifact | CI or release job produces installable build | [ ] | Ties to E04 |
| R03 | `restart_process` spawn failure | Must **not** exit hard leaving screen dark if child `spawn` fails — log + safe fallback (stay alive or retry) | [ ] | 曲街 P0 |
| R04 | Single instance | Second launch focuses / hands off to existing PFD; no duplicate fullscreen stacks | [ ] | 曲街 P0 |
| R05 | Triple-restart behaviour | Document + verify: watchdog self-restart, OS `RegisterApplicationRestart`, memory-limit restart — each continues queue/FS correctly; no restart storm | [ ] | 曲街 P0; best-effort policy still applies |
| R06 | P1 later (not gate) | Module split / audio device hot-plug | [D] | 曲街：可後加 |

---

## 9. Hard fails (any mode)


| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| H01 | Esc exits fullscreen | Must **not** — `[F]` if it does | [ ] | F04 |
| H02 | UI not English (primary chrome) | `[F]` | [ ] | |
| H03 | Crash with no recovery and no actionable log | `[F]` | [ ] | |
| H04 | Installer missing on agreed ship platform | `[F]` or `[B]` until shipped | [ ] | E04 |

---

## Known gaps / limits (not auto-fail unless marked)

| ID | Item | Notes |
|----|------|-------|
| K01 | Linux/mac video backend | Required; unimplemented at `910db58` → track as open work |
| K02 | Installer | Required; not in repo tip → open work |
| K03 | Store HEVC/VP9/AV1 extensions | External; guide mandatory |
| K04 | Unit tests | playlist / launch / aspect only — engine/autostart/watchdog largely untested |
| K05 | No CI (`.github`) | See R01 |
| K07 | restart spawn / single-instance / restart storm | See R03–R05（曲街 P0） |
| K06 | UAC / Ctrl+Alt+Del / critical alerts | Can cover fullscreen — OS, not fail |

---

## Suggested smoke order (Win dedicated screen)

1. E01 → E04 → E06–E08  
2. M01–M08 (quick music)  
3. V01–V05 → F01–F04 (**double-click only**) → F06  
4. A01–A03  
5. S01 or S03 (one recovery path) + D01  
6. V08 + install guide copy  

Linux/mac: E02/E03 + M* until L01/L02 unblocked.

---

## Change log

| Date (HKT) | Change |
|------------|--------|
| 2026-10-29 | Draft v1 by 曲質 from README @ `910db58` + Timothy decisions via 旺財 (both use cases, PFD, Linux/mac video, installer, double-click FS exit, HEVC guide, EN UI, best-effort crash recovery). |
| 2026-10-29 | v1.1：全名 *Player for dummies*；Linux/mac backend TBD；對齊曲街 P0（CI／打包、restart spawn、單實例、三重重啟）；P1 後加。**Not pushed.** |
| 2026-10-29 | v1.2：Timothy 批准 push；E05＝Win／Linux／mac 皆要 installer。曲街 P0 工程在 `p0-reliability-ci` @ `5f2fffdd`（未 merge）供 R01／R03–R05 對照。 |
