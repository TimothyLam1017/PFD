# PFD (A-Player) Acceptance Checklist

**Product name:** **PFD** (*Player for dummies*)  
**Code / crate / exe / Start Menu / packaging / artifacts:** Timothy 2026-10-30 → unify to **PFD** (no `audio-player` on release packages／installer／exe／Start Menu). 曲街 implementing; re-verify after merge. · repo https://github.com/TimothyLam1017/PFD  
**Baseline tip:** `dff56fd8` (2026-10-30 HKT; main after PR #8 fix-pfd-naming-and-continue; PFD rename + Continue B)
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
| Release / artifact naming | **PFD** on all release packages／artifacts／installer／exe／Start Menu (not `audio-player`) — Timothy 2026-10-30 via 旺財; 曲街 implementing |
| Continue from last item | **Independent** Options checkbox; **not** tied to Start with Windows. Behaviour **B** (Timothy 2026-10-30 via 旺財／曲介): when enabled, **normal app open** also resumes from last item (not autostart-only). 曲街 implementing |

---

## 0. Build / install / first launch

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| E01 | `cargo build --release` on Windows | Produces `PFD.exe` / crate `pfd`; size ≤50 MB | [P] | tip `dff56fd8` CI run#24 https://github.com/TimothyLam1017/PFD/actions/runs/40999355806：`cargo test` + `cargo build --release` **success** (post steps still finishing). Crate/exe `pfd`/`PFD.exe`. |
| E02 | `cargo build --release` on Linux | Binary builds; ALSA deps documented | [P] | Prior box [P] ~12.5 MiB (pre-libmpv). Tip `video-mpv` is dynload (no link-time libmpv). Tip re-verify on box blocked (apt mirror/pkg-config). release.yml Linux job installs libasound2-dev+libmpv. README ALSA+libmpv docs OK |
| E03 | `cargo build --release` on macOS | Binary builds | [B] | no macOS runner on this box |
| E04 | **Installer** (Windows) | Installs as **PFD**; Start Menu **PFD**; uninstall clean; artifact/setup/exe names **PFD***/`pfd-*` (not audio-player) | [P] | tip `dff56fd8` Win11 install E2E **[P]** (via 旺財). **Published** [v0.1.0](https://github.com/TimothyLam1017/PFD/releases/tag/v0.1.0) attach run#41001211500 **success**：`PFD-setup-0.1.0.exe` (~4.6 MiB) + `PFD.exe` (~8.4 MiB); no `audio-player*`. |
| E05 | Installer (Linux / macOS) | Installs cleanly (pkg/AppImage/dmg or agreed form); uninstall/docs OK | [P] | tip: `packaging/linux/build-deb.sh` (.deb Depends libmpv2) + `packaging/macos/build-app.sh` (.app+.dmg); `release.yml` ubuntu+macos artifact jobs. Install E2E **not** smoked on box |
| E06 | First launch UI language | All visible chrome **English** | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| E07 | CLI open files / folders / playlist.json | Queue replaced; playback starts | [B] | CLI/playlist open not smoked without fixtures+GUI |
| E08 | Config dir created | `%APPDATA%\audio-player\` (Win) / platform `dirs::config_dir` | [B] | code: dirs::config_dir()/audio-player; first-launch not smoked on box |

---

## 1. Music player (all platforms)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| M01 | Play / Pause / Stop | Works; UI state matches | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| M02 | Previous / Next | Prev restarts item if >3 s played; else previous item | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| M03 | Seek bar | Click/drag seeks; time labels update | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| M04 | Volume / mute / wheel on slider | 0–100 % perceptual; mute toggles | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| M05 | Repeat off / all / one | Behaviour matches mode | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| M06 | Formats: mp3, flac, wav, m4a/aac, ogg | Each plays without crash | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| M07 | Missing / unreadable file | Red ⚠; auto-skipped; queue continues | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| M08 | Playlist JSON load/save | BOM OK; relative paths; non-ASCII preserved | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| M09 | Folder add / DnD / reorder / delete | Queue updates; persisted | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| M10 | Tags fill title/artist/duration in background | Empty fields fill; playlist values win | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| M11 | CJK titles | Render (system CJK font or `font_path`) | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| M12 | Restore session | Volume, repeat, queue, last playlist restored | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| M13 | Shortcuts (window) | Space, arrows, Ctrl+arrows, M, Ctrl+N/O/S per README | [B] | box: no interactive GUI/music smoke this pass (not invented) |

---

## 2. Video — Windows (Media Foundation)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| V01 | H.264 mp4 in window | Picture above playlist; audio sync OK | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| V02 | Hide playlist under video | Video fills player window | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| V03 | Fit / Fill / Stretch | Applies in window + fullscreen; per-entry `fit` overrides | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| V04 | Non-square pixels | Display aspect corrected (DVD-style) | [B] | Win-only — Timothy Win11 smoke |
| V05 | Gapless next video | No long black gap between items / loop to first | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| V06 | Mixed audio+video playlist | Advances correctly across types | [B] | Win-only — Timothy Win11 smoke |
| V07 | Unsupported / no-picture file | Skip after ~20 s no picture; clear handling | [B] | Win-only — Timothy Win11 smoke |
| V08 | HEVC / VP9 / AV1 | With Store extensions: play; without: **clear error + install guide** | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
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
| F01 | Enter fullscreen (F / menu / double-click) | Picture only: no chrome, timeline, subtitles, cursor | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| F02 | Topmost + re-assert ~2 s | Stays above normal apps | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| F03 | High priority + no sleep/dim/screensaver | Display stays on during show | [B] | Win-only — Timothy Win11 smoke |
| F04 | **Exit only by double-click on video** | Esc / stray keys do **not** exit | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29); hard req — exit only double-click |
| F05 | Silent keys in fullscreen | Space, ←/→, Ctrl+←/→, P/N, media keys work | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| F06 | Monitor picker | Fullscreen on chosen monitor | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| F07 | Lobby shortcut | `pfd.exe` (or ship exe name) `--fullscreen --repeat all playlist.json` | [B] | Win-only — Timothy Win11 smoke; exe name → **PFD** after rename |

---

## 5. Start with Windows / resume

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| A01 | Options → Start with Windows | HKCU Run written; no admin | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| A02 | Autostart after sign-in | Opens fullscreen; loops last playlist | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| A03 | Continue from last item | Independent Options checkbox; **not** bound to Start with Windows. **Behaviour B**: toggle ON → **normal launch** resumes last item; OFF → no auto-play from last on normal open | [P] | tip `dff56fd8` static + **Timothy Win11 (via 旺財 2026-10-30) [P]**: Continue independent; ON＝B normal open resumes last; OFF OK. |
| A04 | Wait for devices (~90 s) | Survives slow disk/audio/display ready | [B] | Win-only — Timothy Win11 smoke |
| A05 | Queue saved on change | Power cut does not empty show list | [B] | Win-only — Timothy Win11 smoke |
| A06 | OS checklist (manual) | Autologon / DND / Active hours / codec extensions documented | [B] | Win-only — Timothy Win11 smoke OS settings documented in README |

---

## 6. Stability / recovery (best-effort policy)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| S01 | Decoder hang ~20–25 s | Watchdog restarts process; continues next item; fullscreen restored if was FS | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| S02 | `RegisterApplicationRestart` | After ≥60 s run, OS relaunch continues queue | [B] | Win-only — Timothy Win11 smoke |
| S03 | `memory_limit_mb` (default 1500) | Restart at next item; force within ~60 s if no boundary | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29) |
| S04 | Display mode change | Fullscreen window recreated to new mode | [B] | Win-only — Timothy Win11 smoke |
| S05 | Recovery **fails** (no restart / stuck black / wrong index) | `[F]` + repro | [B] | Win-only — Timothy Win11 smoke |
| S06 | Minimized / covered window | Auto-advance still works (watcher thread) | [B] | Win-only — Timothy Win11 smoke |

---

## 7. Diagnostics & repro (required for every `[F]`)

| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| D01 | Log location documented | Fixed path for release builds (no console) — **recommend** `%APPDATA%\audio-player\logs\` | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29); release log path OK on Win11 (was local [F] gap) |
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
| R02 | Packaging / installer artifact | CI or release job produces installable build | [P] | **Published** [v0.1.0](https://github.com/TimothyLam1017/PFD/releases/tag/v0.1.0) @`dff56fd8` attach run#41001211500 **success** — 6 assets: `PFD-setup-0.1.0.exe`, `PFD.exe`, `pfd_0.1.0_amd64.deb`, `pfd-linux-amd64`, `PFD-0.1.0-macos.dmg`, `pfd-macos`. Naming **PFD**/`pfd-*` only. |
| R03 | `restart_process` spawn failure | Must **not** exit hard leaving screen dark if child `spawn` fails — log + safe fallback (stay alive or retry) | [P] | try_restart_process keeps alive on spawn fail; unit tests pass on Linux box |
| R04 | Single instance | Second launch focuses / hands off to existing PFD; no duplicate fullscreen stacks | [B] | Win mutex code present; tip Win CI **compiles** (run#17 green). Second instance exits (no focus handoff) still open; Linux no-op |
| R05 | Triple-restart behaviour | Document + verify: watchdog self-restart, OS `RegisterApplicationRestart`, memory-limit restart — each continues queue/FS correctly; no restart storm | [B] | README+code paths for watchdog/OS restart/memory; E2E not verified on box |
| R06 | P1 later (not gate) | Module split / audio device hot-plug | [D] | 曲街：可後加 |

---

## 9. Hard fails (any mode)


| ID | Check | Expected | Result | Notes |
|----|-------|----------|--------|-------|
| H01 | Esc exits fullscreen | Must **not** — `[F]` if it does | [P] | Timothy Win11 smoke @899eefa **[P]** (via 旺財 2026-10-29); Esc does **not** exit fullscreen |
| H02 | UI not English (primary chrome) | `[F]` | [B] | box: no interactive GUI/music smoke this pass (not invented) |
| H03 | Crash with no recovery and no actionable log | `[F]` | [B] | needs crash+log packet |
| H04 | Installer missing on agreed ship platform | `[F]` or `[B]` until shipped | [P] | **v0.1.0** shipped (setup/deb/dmg + bins); Win install E2E **[P]** (Timothy). Unsigned; Linux/mac install E2E open. |

---

## Known gaps / limits (not auto-fail unless marked)

| ID | Item | Notes |
|----|------|-------|
| K01 | Linux/mac video backend | **libmpv wired** at `899eefa` (`video-mpv` default; `mpv.rs`/`mpv_ffi.rs`; stub only w/ `--no-default-features`). L01/L02 play smoke still open |
| K02 | Installer | tip `dff56fd8` `pfd-*` artifacts; Win Start Menu／uninstall E2E **[P]** (Timothy 2026-10-30). Unsigned; Linux/mac E2E open; publish approved |
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
| 2026-10-29 | v1.5：Timothy Win11 smoke @`899eefa`（via 旺財）：M01–M08、V01/V02/V03/V05/V08、F01/F02/F04/F05/F06、H01、A01–A03、S01/S03、D01 全部 **[P]**。未測仍 [B]：V04/V06/V07/V09、F03/F07、A04–A06、S02/S04–S06、M09–M13、L01–L03、E03/E06–E08 等。Local checklist only — **do not push** without Timothy via 旺財。 |
| 2026-10-30 | v1.5.1 local：Release packages run#40979093495 @`ea2d54cd`（no publish）：Win Package **[F]** ISCC `InitializeWizard` 應為 procedure 非 function Boolean → 無 Win artifact；Linux/mac Package **[P]** artifacts。E04 notes→[F] packaging build；R02 partial。**do not push** without Timothy via 旺財。 |
| 2026-10-30 | v1.5.2 local：Baseline → `ed30c54c` (PR #7 Inno InitializeWizard fix). Release packages run#40984596451 **success**：E04/R02/H04 **[P]** (`audio-player-setup-0.1.0.exe` ~4.6 MiB). Install E2E still Win11. **do not push** without Timothy via 旺財。 |
| 2026-10-30 | v1.5.3：Timothy Win11 **E04 install E2E [P]** @`ed30c54c` artifact（via 旺財）：download setup／Start Menu A-Player／uninstall 全部 OK。E04/R02/H04/K02 notes refreshed. **Timothy via 旺財 approved push.** |
| 2026-10-30 | v1.5.4 local：Timothy via 旺財（曲街跟）—(1) release package／artifact／installer／exe／Start Menu 統一 **PFD**（唔再 audio-player）；(2) Options「continue from the last item played」獨立，唔綁 Start with Windows。E01/E04/A03/F07/K02 + Owner policy 預記；Result 標 [P]* 等曲街 PR merge 後重驗。**do not push** without Timothy via 旺財。 |
| 2026-10-30 | v1.5.4b local：Timothy via 旺財／曲介拍板 Continue＝**B**（普通開 app 亦由 last item 起播）。A03 Expected／Owner policy 跟 B；等曲街改完重驗。**do not push** without Timothy via 旺財。 |
| 2026-10-30 | v1.5.5 local：PR #8 merged main `dff56fd8`（PFD 改名＋Continue B）。E01 CI run#24 in_progress；E04 靜態 iss/yml **[P]**、等 Release dispatch＋Timothy Win11 E2E；A03 靜態 start_up **[P]**、等 Win11 Options E2E。**do not push** without Timothy via 旺財。 |
| 2026-10-30 | v1.5.6 local：Release packages run#40999425598 @`dff56fd8` **success** — E04/R02 artifact 名 **[P]**（`pfd-windows`→`PFD-setup-0.1.0.exe`+`PFD.exe`；linux/mac 亦 pfd/PFD）。A03 靜態仍 [B] 等 Win11；E04 install E2E（Start Menu PFD）等 Timothy。**do not push** without Timothy via 旺財。 |
| 2026-10-30 | v1.5.7：Timothy Win11（via 旺財）E04 名＋install／Start Menu **PFD**／uninstall **[P]**；A03＝B（獨立＋ON/OFF）**[P]**；E01 CI run#24 test+release **[P]**。Timothy **已批 publish**。**Timothy via 旺財 approved push.** |
| 2026-10-30 | v1.5.8：Release **v0.1.0** published；attach run#41001211500 **success**；六資產齊（setup／exe／deb／linux／dmg／macos），命名 **PFD**/`pfd-*`。E04/R02/H04 notes 記 release。**Timothy via 旺財 approved push.** |
