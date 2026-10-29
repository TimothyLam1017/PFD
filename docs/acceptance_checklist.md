# PFD (A-Player) Acceptance Checklist

**Product name:** **PFD** (*Player for dummies*)  
**Code / crate / exe / Start Menu:** today `audio-player` — gradually unify branding to **PFD** (曲街) · repo https://github.com/TimothyLam1017/PFD  
**Baseline tip:** `899eefa` (2026-10-29 HKT; main after PR #5 feat-libmpv-installers merge; includes PR #4 ENUM fix + PR #6 libmpv)  
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
| Video on Linux / macOS | **Required**; backend **libmpv** (settled). **Wired** at tip `899eefa` (`video-mpv` default, dynload). **Windows stays Media Foundation** |
| Binary / package size | **≤50 MB** target (release binary / ship package) |
| Distribution | **Installer** required on **Windows, Linux, and macOS** (not raw binary only; no Win-only short-term) |
| Leave fullscreen | **Double-click video only** (Esc must not exit) — hard requirement |
| HEVC / VP9 / AV1 | Need **install guide** for Microsoft Store Video Extensions; missing codec → clear message + guide, not silent fail |
| Crash / decoder hang / memory restart | **Best-effort recovery** — record caution if recovery works; hang/crash **without** recovery path = `[F]` |
| Checklist push | Timothy approved 2026-10-29 via 旺財 |

---

## 0. Build / install / first launch

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| E01 | `cargo build --release` on Windows | Produces `target/release/audio-player.exe`; size ≤50 MB | [P] | tip Actions run#17 @899eefa **success**: cargo test + cargo build --release (Windows). ENUM_DISPLAY_DEVICE_FLAGS fixed in PR #4 (0u32); PR #5 tip. https://github.com/TimothyLam1017/PFD/actions/runs/40884226812 |
| E02 | `cargo build --release` on Linux | Binary builds; ALSA deps documented | [P] | Prior box [P] ~12.5 MiB (pre-libmpv). Tip `video-mpv` is dynload (no link-time libmpv). Tip re-verify on box blocked (apt mirror/pkg-config). release.yml Linux job installs libasound2-dev+libmpv. README ALSA+libmpv docs OK |
| E03 | `cargo build --release` on macOS | Binary builds | [B] | no macOS runner on this box |
| E04 | **Installer** (Windows) | Installs app, Start Menu / shortcut, uninstall clean | [P] | tip `899eefa`: `packaging/windows/audio-player.iss` + `build.ps1` (Inno); `release.yml` Win matrix uploads `audio-player-setup-*.exe`. Install E2E / SmartScreen **not** smoked on box |
| E05 | Installer (Linux / macOS) | Installs cleanly (pkg/AppImage/dmg or agreed form); uninstall/docs OK | [P] | tip: `packaging/linux/build-deb.sh` (.deb Depends libmpv2) + `packaging/macos/build-app.sh` (.app+.dmg); `release.yml` ubuntu+macos artifact jobs. Install E2E **not** smoked on box |
| E06 | First launch UI language | All visible chrome **English** | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| E07 | CLI open files / folders / playlist.json | Queue replaced; playback starts | [B] | CLI/playlist open not smoked without fixtures+GUI |
| E08 | Config dir created | `%APPDATA%\audio-player\` (Win) / platform `dirs::config_dir` | [B] | code: dirs::config_dir()/audio-player; first-launch not smoked on box |

---

## 1. Music player (all platforms)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| M01 | Play / Pause / Stop | Works; UI state matches | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| M02 | Previous / Next | Prev restarts item if >3 s played; else previous item | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| M03 | Seek bar | Click/drag seeks; time labels update | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| M04 | Volume / mute / wheel on slider | 0–100 % perceptual; mute toggles | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| M05 | Repeat off / all / one | Behaviour matches mode | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| M06 | Formats: mp3, flac, wav, m4a/aac, ogg | Each plays without crash | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| M07 | Missing / unreadable file | Red ⚠; auto-skipped; queue continues | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| M08 | Playlist JSON load/save | BOM OK; relative paths; non-ASCII preserved | [B] | unit `parses_example_playlist` pass on box; interactive load/save not smoked |
| M09 | Folder add / DnD / reorder / delete | Queue updates; persisted | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| M10 | Tags fill title/artist/duration in background | Empty fields fill; playlist values win | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| M11 | CJK titles | Render (system CJK font or `font_path`) | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| M12 | Restore session | Volume, repeat, queue, last playlist restored | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| M13 | Shortcuts (window) | Space, arrows, Ctrl+arrows, M, Ctrl+N/O/S per README | [B] | box: no interactive GUI/music smoke this pass (not invented) |

---

## 2. Video — Windows (Media Foundation)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| V01 | H.264 mp4 in window | Picture above playlist; audio sync OK | [B] | Win-only — Timothy Win11 smoke |
| V02 | Hide playlist under video | Video fills player window | [B] | Win-only — Timothy Win11 smoke |
| V03 | Fit / Fill / Stretch | Applies in window + fullscreen; per-entry `fit` overrides | [B] | Win-only — Timothy Win11 smoke |
| V04 | Non-square pixels | Display aspect corrected (DVD-style) | [B] | Win-only — Timothy Win11 smoke |
| V05 | Gapless next video | No long black gap between items / loop to first | [B] | Win-only — Timothy Win11 smoke |
| V06 | Mixed audio+video playlist | Advances correctly across types | [B] | Win-only — Timothy Win11 smoke |
| V07 | Unsupported / no-picture file | Skip after ~20 s no picture; clear handling | [B] | Win-only — Timothy Win11 smoke |
| V08 | HEVC / VP9 / AV1 | With Store extensions: play; without: **clear error + install guide** | [B] | Win-only — Timothy Win11 smoke; HEVC guide required |
| V09 | `video_probe` example | Builds; writes `video-probe.log` with format/size/FPS notes | [B] | Win-only — Timothy Win11 smoke; tip `examples/video_probe.rs` not fetched to box (MCP size); Win example |

---

## 3. Video — Linux / macOS (**required by Timothy**)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| L01 | Play common video (e.g. H.264 mp4) on Linux | Plays in-app (not “unsupported skip-only”) | [B] | tip `899eefa`: libmpv **wired** (`src/video/mpv.rs`, AVAILABLE=true w/ video-mpv). Play smoke needs GUI+libmpv2 — not run on box |
| L02 | Play common video on macOS | Same | [B] | tip: libmpv wired (dynload Homebrew mpv). No macOS runner/GUI smoke this pass |
| L03 | Fullscreen / fit modes on Linux/mac | Parity with Win behaviour where feasible | [B] | code paths in mpv.rs; blocked on L01/L02 interactive smoke |
| L04 | Until backend ships | README + UI state “video not available” is honest; checklist stays `[B]`/`[ ]` not fake `[P]` | [P] | Backend **shipped** @899eefa; README documents system libmpv + clear missing-lib error; L01/L02 stay [B] until play smoke |

---

## 4. Fullscreen unattended screen (Windows primary)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| F01 | Enter fullscreen (F / menu / double-click) | Picture only: no chrome, timeline, subtitles, cursor | [B] | Win-only — Timothy Win11 smoke |
| F02 | Topmost + re-assert ~2 s | Stays above normal apps | [B] | Win-only — Timothy Win11 smoke |
| F03 | High priority + no sleep/dim/screensaver | Display stays on during show | [B] | Win-only — Timothy Win11 smoke |
| F04 | **Exit only by double-click on video** | Esc / stray keys do **not** exit | [B] | Win-only — Timothy Win11 smoke hard req |
| F05 | Silent keys in fullscreen | Space, ←/→, Ctrl+←/→, P/N, media keys work | [B] | Win-only — Timothy Win11 smoke |
| F06 | Monitor picker | Fullscreen on chosen monitor | [B] | Win-only — Timothy Win11 smoke |
| F07 | Lobby shortcut | `audio-player.exe --fullscreen --repeat all playlist.json` | [B] | Win-only — Timothy Win11 smoke |

---

## 5. Start with Windows / resume

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| A01 | Options → Start with Windows | HKCU Run written; no admin | [B] | Win-only — Timothy Win11 smoke |
| A02 | Autostart after sign-in | Opens fullscreen; loops last playlist | [B] | Win-only — Timothy Win11 smoke |
| A03 | Continue from last item | Toggle respected | [B] | Win-only — Timothy Win11 smoke |
| A04 | Wait for devices (~90 s) | Survives slow disk/audio/display ready | [B] | Win-only — Timothy Win11 smoke |
| A05 | Queue saved on change | Power cut does not empty show list | [B] | Win-only — Timothy Win11 smoke |
| A06 | OS checklist (manual) | Autologon / DND / Active hours / codec extensions documented | [B] | Win-only — Timothy Win11 smoke OS settings documented in README |

---

## 6. Stability / recovery (best-effort policy)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| S01 | Decoder hang ~20–25 s | Watchdog restarts process; continues next item; fullscreen restored if was FS | [B] | Win-only — Timothy Win11 smoke |
| S02 | `RegisterApplicationRestart` | After ≥60 s run, OS relaunch continues queue | [B] | Win-only — Timothy Win11 smoke |
| S03 | `memory_limit_mb` (default 1500) | Restart at next item; force within ~60 s if no boundary | [B] | Win-only — Timothy Win11 smoke |
| S04 | Display mode change | Fullscreen window recreated to new mode | [B] | Win-only — Timothy Win11 smoke |
| S05 | Recovery **fails** (no restart / stuck black / wrong index) | `[F]` + repro | [B] | Win-only — Timothy Win11 smoke |
| S06 | Minimized / covered window | Auto-advance still works (watcher thread) | [B] | Win-only — Timothy Win11 smoke |

---

## 7. Diagnostics & repro (required for every `[F]`)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| D01 | Log location documented | Fixed path for release builds (no console) — **recommend** `%APPDATA%\audio-player\logs\` | [F] | only env_logger→stderr; no fixed release log file under config dir (gap vs recommend logs/) |
| D02 | Fail packet | SHA, OS build, playlist, CLI flags, codec, monitor count, log + `video-probe.log` | [B] | not exercised |
| D03 | Repro template used | See below | [B] | template present; unused this pass |

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
| R01 | CI | At least `cargo test` + `cargo check` on PR/main | [P] | tip @899eefa Actions run#17 **green** (Win cargo test + release). ci.yml still **Windows-only** (no Linux job on PR/push). release.yml has Win/Linux/mac packaging matrix on tag/`workflow_dispatch`. Prior: run#14 (PR#6) success; run#12 (PR#4 ENUM) success (superseded tip) |
| R02 | Packaging / installer artifact | CI or release job produces installable build | [P] | `packaging/` + `.github/workflows/release.yml` matrix uploads Win setup / Linux .deb / macOS .dmg (+ raw bins). Not triggered by every PR; needs tag or dispatch. Install E2E open |
| R03 | `restart_process` spawn failure | Must **not** exit hard leaving screen dark if child `spawn` fails — log + safe fallback (stay alive or retry) | [P] | try_restart_process keeps alive on spawn fail; unit tests pass on Linux box |
| R04 | Single instance | Second launch focuses / hands off to existing PFD; no duplicate fullscreen stacks | [B] | Win mutex code present; tip Win CI **compiles** (run#17 green). Second instance exits (no focus handoff) still open; Linux no-op |
| R05 | Triple-restart behaviour | Document + verify: watchdog self-restart, OS `RegisterApplicationRestart`, memory-limit restart — each continues queue/FS correctly; no restart storm | [B] | README+code paths for watchdog/OS restart/memory; E2E not verified on box |
| R06 | P1 later (not gate) | Module split / audio device hot-plug | [D] | 曲街：可後加 |

---

## 9. Hard fails (any mode)


| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| H01 | Esc exits fullscreen | Must **not** — `[F]` if it does | [B] | Win-only — Timothy Win11 smoke |
| H02 | UI not English (primary chrome) | `[F]` | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| H03 | Crash with no recovery and no actionable log | `[F]` | [B] | needs crash+log packet |
| H04 | Installer missing on agreed ship platform | `[F]` or `[B]` until shipped | [P] | packaging scripts + release.yml artifacts present for Win/Linux/mac @899eefa (E04/E05); unsigned / E2E install still open |

---

## Known gaps / limits (not auto-fail unless marked)

| ID | Item | Notes |
|----|------|-------|
| K01 | Linux/mac video backend | **libmpv wired** at `899eefa` (`video-mpv` default; `mpv.rs`/`mpv_ffi.rs`; stub only w/ `--no-default-features`). L01/L02 play smoke still open |
| K02 | Installer | Scripts + release.yml for Win Inno / Linux .deb / macOS .dmg present @899eefa; unsigned; E2E install smoke open |
| K03 | Store HEVC/VP9/AV1 extensions | External; guide mandatory |
| K04 | Unit tests | tip adds restart spawn decision + restart_args tests (pass on Linux); playlist/launch/aspect remain; engine/autostart/watchdog still light |
| K05 | CI Windows-only; tip run failed | tip Actions run#17 @899eefa **success**. ci.yml still Win-only on PR/push; Linux/mac packaging via release.yml only — see R01 |
| K07 | restart spawn / single-instance / restart storm | R03 unit+static **[P]** on Linux; R04 Win **compiles** on tip CI (run#17); focus handoff + R05 E2E still open（曲街） |
| K06 | UAC / Ctrl+Alt+Del / critical alerts | Can cover fullscreen — OS, not fail |

---

## Suggested smoke order (Win dedicated screen)

1. E01 → E04 → E06–E08  
2. M01–M08 (quick music)  
3. V01–V05 → F01–F04 (**double-click only**) → F06  
4. A01–A03  
5. S01 or S03 (one recovery path) + D01  
6. V08 + install guide copy  

Linux/mac: E02/E03 + M*; L01/L02 backend wired — need interactive play smoke + system libmpv.

---

## Change log

| Date (HKT) | Change |
|------------|--------|
| 2026-10-29 | Draft v1 by 曲質 from README @ `910db58` + Timothy decisions via 旺財 (both use cases, PFD, Linux/mac video, installer, double-click FS exit, HEVC guide, EN UI, best-effort crash recovery). |
| 2026-10-29 | v1.1：全名 *Player for dummies*；Linux/mac backend TBD；對齊曲街 P0（CI／打包、restart spawn、單實例、三重重啟）；P1 後加。**Not pushed.** |
| 2026-10-29 | v1.2：Timothy 批准 push；E05＝Win／Linux／mac 皆要 installer。曲街 P0 工程在 `p0-reliability-ci` @ `5f2fffdd`（未 merge）供 R01／R03–R05 對照。 |
| 2026-10-29 | v1.3：Baseline → `234097eb` (PR #1 merged); repo URL → TimothyLam1017/PFD；Linux/mac video policy → **libmpv** (Win MF stays)；size ≤50MB；R01 CI present (Win test+release; tip Actions failed E0277)；R03 [P] Linux unit；E02 [P] Linux release 13 101 856 B；E04/E05/D01/L01–L03 gaps updated；K01/K05/K07 refreshed. Local checklist only — **do not push** without Timothy. |
| 2026-10-29 | v1.3.1：E01 re-verify after PR #3 merge @ `9d0ebc74`; Actions run#10 **failure** E0425 `ENUM_DISPLAY_DEVICE_FLAGS` @ win.rs:280 (SendSyncHandle cleared E0277). E01 stays [F]; R01/K05 notes refreshed. Older UX tip run#8 @1f6a566c also failure. Local checklist only — **do not push**. |
| 2026-10-29 | v1.4：Baseline → `899eefa` (PR #5 feat-libmpv-installers; includes PR #4 ENUM + PR #6 libmpv). E01 **[P]** Actions run#17 green (test+release). E04/E05/R02 **[P]** packaging/ + release.yml Win/Linux/mac artifacts (install E2E open). L01–L03 still [B] smoke; L04/K01 libmpv wired. R01 green Win; ci.yml still no Linux job. Superseded: run#12 @b32d6d06 success; run#14 @795fca28 success. **Timothy via 旺財 approved push.** |
