#!/usr/bin/env bash
# Build a simple amd64 .deb for audio-player (PFD).
# Prerequisites: cargo release binary already built (or this script builds it),
#                dpkg-deb, and runtime dep libmpv2 on the target machine.
# Usage (from repo root):  bash packaging/linux/build-deb.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -1)"
VERSION="${VERSION:-0.1.0}"
ARCH="amd64"
PKG_NAME="audio-player"
DEB_DIR="$ROOT/dist/linux/${PKG_NAME}_${VERSION}_${ARCH}"
OUT_DEB="$ROOT/dist/linux/${PKG_NAME}_${VERSION}_${ARCH}.deb"

echo "==> cargo build --release"
cargo build --release

BIN="$ROOT/target/release/audio-player"
test -x "$BIN"

rm -rf "$DEB_DIR"
mkdir -p "$DEB_DIR/DEBIAN" \
         "$DEB_DIR/usr/bin" \
         "$DEB_DIR/usr/share/applications" \
         "$DEB_DIR/usr/share/doc/$PKG_NAME"

install -m 755 "$BIN" "$DEB_DIR/usr/bin/audio-player"

cat > "$DEB_DIR/DEBIAN/control" <<CTRL
Package: ${PKG_NAME}
Version: ${VERSION}
Section: sound
Priority: optional
Architecture: ${ARCH}
Depends: libmpv2 | libmpv1, libasound2 | libasound2t64
Maintainer: audio-player contributors <noreply@example.com>
Description: Lightweight native music and video player (PFD / A-Player)
 eframe/egui music + video player. Video on Linux dynloads system libmpv
 (install libmpv2). Windows builds use Media Foundation instead.
CTRL

cat > "$DEB_DIR/usr/share/applications/audio-player.desktop" <<DESK
[Desktop Entry]
Type=Application
Name=A-Player
Comment=Lightweight music and video player
Exec=audio-player %F
Terminal=false
Categories=AudioVideo;Player;Audio;Video;
MimeType=audio/mpeg;audio/flac;audio/x-wav;audio/mp4;video/mp4;video/x-matroska;video/webm;
DESK

cat > "$DEB_DIR/usr/share/doc/$PKG_NAME/README.Debian" <<DOC
Runtime video requires system libmpv (Debian/Ubuntu: apt install libmpv2).
libmpv is NOT bundled; the binary dynloads it at runtime.
DOC

# Optional: produce AppImage later with linuxdeploy + appimagetool from the staged tree.
# CI currently ships the .deb only to keep the first cut simple.

SIZE_KB="$(du -sk "$DEB_DIR" | awk '{print $1}')"
echo "Installed-Size: $SIZE_KB" >> "$DEB_DIR/DEBIAN/control"

mkdir -p "$(dirname "$OUT_DEB")"
dpkg-deb --build --root-owner-group "$DEB_DIR" "$OUT_DEB"
echo "Done: $OUT_DEB"
ls -lh "$OUT_DEB"
