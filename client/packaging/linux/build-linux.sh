#!/usr/bin/env bash
# Build Linux packages: portable .tar.gz, .deb (cargo-deb) and AppImage (appimagetool).
# Usage: packaging/linux/build-linux.sh        (run from the client/ folder or anywhere)
set -euo pipefail
cd "$(dirname "$0")/../.."
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
ARCH=$(uname -m)
OUT=dist
mkdir -p "$OUT"

cargo build --release
BIN=target/release/paddock-client

# 1) Portable tarball
STAGE=$(mktemp -d)
mkdir -p "$STAGE/paddock-$VERSION"
cp "$BIN" "$STAGE/paddock-$VERSION/paddock"
cp assets/icon-256.png "$STAGE/paddock-$VERSION/paddock.png"
cp packaging/linux/paddock.desktop "$STAGE/paddock-$VERSION/"
tar -C "$STAGE" -czf "$OUT/paddock-$VERSION-linux-$ARCH.tar.gz" "paddock-$VERSION"
rm -rf "$STAGE"
echo "OK  $OUT/paddock-$VERSION-linux-$ARCH.tar.gz"

# 2) .deb
if command -v cargo-deb >/dev/null; then
    cargo deb --no-build --output "$OUT/"
    echo "OK  .deb em $OUT/"
else
    echo "--  cargo-deb não instalado (cargo install cargo-deb): pulando .deb"
fi

# 3) AppImage
APPIMAGETOOL=${APPIMAGETOOL:-$(command -v appimagetool || true)}
if [[ -n "$APPIMAGETOOL" ]]; then
    APPDIR=$(mktemp -d)/Paddock.AppDir
    mkdir -p "$APPDIR/usr/bin"
    cp "$BIN" "$APPDIR/usr/bin/paddock"
    cp packaging/linux/paddock.desktop "$APPDIR/paddock.desktop"
    cp assets/icon-256.png "$APPDIR/paddock.png"
    cat > "$APPDIR/AppRun" <<'RUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/paddock" "$@"
RUN
    chmod +x "$APPDIR/AppRun"
    ARCH=$ARCH "$APPIMAGETOOL" --no-appstream "$APPDIR" "$OUT/Paddock-$VERSION-$ARCH.AppImage"
    echo "OK  $OUT/Paddock-$VERSION-$ARCH.AppImage"
else
    echo "--  appimagetool não encontrado (https://github.com/AppImage/appimagetool/releases): pulando AppImage"
fi
