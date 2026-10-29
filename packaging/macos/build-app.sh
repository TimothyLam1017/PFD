#!/usr/bin/env bash
# Build audio-player.app + a simple .dmg.
# Prerequisites: Rust 1.87+, brew install mpv (for runtime libmpv), hdiutil.
# Usage (from repo root):  bash packaging/macos/build-app.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -1)"
VERSION="${VERSION:-0.1.0}"
DIST="$ROOT/dist/macos"
APP="$DIST/audio-player.app"
DMG="$DIST/audio-player-${VERSION}-macos.dmg"

echo "==> cargo build --release"
cargo build --release

BIN="$ROOT/target/release/audio-player"
test -x "$BIN"

rm -rf "$APP" "$DMG"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

install -m 755 "$BIN" "$APP/Contents/MacOS/audio-player"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>A-Player</string>
  <key>CFBundleDisplayName</key><string>A-Player</string>
  <key>CFBundleIdentifier</key><string>com.audio-player.app</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleExecutable</key><string>audio-player</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>Media</string>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSItemContentTypes</key>
      <array>
        <string>public.audio</string>
        <string>public.movie</string>
      </array>
    </dict>
  </array>
</dict>
</plist>
PLIST

# Note for users who open the .app without brew mpv installed:
cat > "$APP/Contents/Resources/README-libmpv.txt" <<NOTE
Video on macOS requires system libmpv from Homebrew:
  brew install mpv
libmpv is not bundled inside this .app (keeps the binary small).
NOTE

echo "==> hdiutil create $DMG"
STAGE="$DIST/dmg-stage"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -sf /Applications "$STAGE/Applications"
hdiutil create -volname "A-Player ${VERSION}" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
rm -rf "$STAGE"

echo "Done: $APP"
echo "Done: $DMG"
ls -lh "$APP/Contents/MacOS/audio-player" "$DMG"
