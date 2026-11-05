#!/usr/bin/env bash
# Build Paddock.app (universal: Apple Silicon + Intel) and a .dmg, on a Mac.
# No Apple Developer account needed: the app is ad-hoc signed only.
# Requires: Xcode Command Line Tools (xcode-select --install) and Rust (https://rustup.rs).
#
# Users opening an ad-hoc signed app for the first time must right-click > Open
# (or run: xattr -dr com.apple.quarantine /Applications/Paddock.app).
set -euo pipefail
cd "$(dirname "$0")/../.."
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
OUT=dist
APP="$OUT/Paddock.app"

rustup target add aarch64-apple-darwin x86_64-apple-darwin
export MACOSX_DEPLOYMENT_TARGET=11.0
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/paddock" \
    target/aarch64-apple-darwin/release/paddock-client \
    target/x86_64-apple-darwin/release/paddock-client
sed "s/__VERSION__/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"

# Icon: build an .icns from the 1024px PNG.
ICONSET=$(mktemp -d)/Paddock.iconset
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
    sips -z $s $s assets/icon-1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    sips -z $((s*2)) $((s*2)) assets/icon-1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Paddock.icns"

# Ad-hoc signature (free). Required for the universal binary to launch on Apple Silicon.
codesign --force --deep --sign - "$APP"

hdiutil create -volname Paddock -srcfolder "$APP" -ov -format UDZO "$OUT/Paddock-$VERSION-macos.dmg" >/dev/null
echo "OK  $APP"
echo "OK  $OUT/Paddock-$VERSION-macos.dmg"
