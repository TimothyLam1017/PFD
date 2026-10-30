# Packaging / installers

Scripts and CI that produce **Windows**, **Linux**, and **macOS** install packages for
**PFD**. These stack on the `feat-libmpv` video backend: Linux/macOS
dynload system **libmpv** (not bundled); Windows keeps Media Foundation.

Release binaries target **≤ ~50 MB** best-effort by not shipping libmpv/ffmpeg.

## Outputs

| Platform | Artifact (typical) | Runtime video dependency |
|----------|--------------------|--------------------------|
| Windows  | `PFD-setup-x.y.z.exe` (Inno Setup) | Media Foundation (+ Store codec packs for HEVC/VP9/AV1) |
| Linux    | `pfd_x.y.z_amd64.deb` | `libmpv2` (Debian/Ubuntu) / `mpv-libs` (Fedora) |
| macOS    | `PFD-x.y.z-macos.dmg` (contains `PFD.app`) | Homebrew `mpv` (`brew install mpv`) |

CI uploads artifacts named `pfd-windows` / `pfd-linux` / `pfd-macos`, plus the raw
`cargo build --release` binary (`pfd` / staged `PFD.exe` on Windows).

## Build locally

Requires Rust **1.87+**. From the repo root:

### Windows (Inno Setup)

1. `cargo build --release`
2. Install [Inno Setup 6](https://jrsoftware.org/isinfo.php) (includes `ISCC.exe`).
3. `packaging/windows/build.ps1`  
   or: `ISCC packaging/windows/pfd.iss`

The script stages `target/release/pfd.exe` as `dist/windows/PFD.exe` and runs the
`.iss` script. Unsigned installer is fine for private builds.

### Linux (`.deb`)

1. Install build deps: `sudo apt install libasound2-dev libmpv2` (Ubuntu/Debian).
2. `cargo build --release`
3. `packaging/linux/build-deb.sh`  
   Produces `dist/linux/pfd_<version>_amd64.deb`.

Install: `sudo apt install ./dist/linux/pfd_*.deb`  
Runtime dep: package `Depends` on `libmpv2` (or install `libmpv2` yourself).

Optional AppImage (not produced by default CI): see comments in `build-deb.sh`.

### macOS (`.app` + `.dmg`)

1. `brew install mpv` (provides libmpv for video).
2. `cargo build --release`
3. `packaging/macos/build-app.sh`  
   Produces `dist/macos/PFD.app` and `dist/macos/PFD-<version>-macos.dmg`.

No Apple code signing / notarization in these scripts (Gatekeeper may warn on unsigned
downloads).

## GitHub Actions

Workflow: [`.github/workflows/release.yml`](../.github/workflows/release.yml)

- **Triggers:** `workflow_dispatch`, or push of a tag matching `v*` (e.g. `v0.1.0`).
- **Matrix:** `windows-latest`, `ubuntu-latest`, `macos-latest`.
- **Artifacts:** per-OS zip/folder with installer (or `.deb` / `.dmg`) + release binary.
- On tag push, also attaches assets to a GitHub Release (if the run has permission).

Keep CI green on Windows via the existing [`.github/workflows/ci.yml`](../.github/workflows/ci.yml).

## HEVC / extra codecs (Windows)

Microsoft Store **does not** ship every codec with Windows. For HEVC / VP9 / AV1 on
Windows, install the matching free/paid **Video Extensions** from the Microsoft Store
(many PCs already have them). See the main README “Setting up a dedicated video screen”
and the Video formats bullet. Linux/macOS codecs come from the system ffmpeg/libmpv
build (`brew install mpv` / distro `libmpv2`).

## Gaps / known limits

* Installers are **unsigned** (Windows SmartScreen / macOS Gatekeeper may warn).
* No AppImage / Flatpak / Homebrew cask yet — `.deb` + `.dmg` + Inno are the first cut.
* Linux/mac CI installs `libmpv-dev` / brew `mpv` only to **link/build**; runtime still
  expects the system library (dynload).
* Binary size ≤50 MB is best-effort; strip + LTO are already on in `Cargo.toml`.
