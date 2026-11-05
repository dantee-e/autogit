#!/usr/bin/env bash
# Cross-compile the Windows .exe from Linux.
# Requires: rustup target add x86_64-pc-windows-gnu ; apt install gcc-mingw-w64-x86-64 zip
set -euo pipefail
cd "$(dirname "$0")/../.."
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
cargo build --release --target x86_64-pc-windows-gnu
mkdir -p dist
STAGE=$(mktemp -d)
cp target/x86_64-pc-windows-gnu/release/paddock-client.exe "$STAGE/Paddock.exe"
(cd "$STAGE" && zip -q "paddock.zip" Paddock.exe)
mv "$STAGE/paddock.zip" "dist/paddock-$VERSION-windows-x64.zip"
rm -rf "$STAGE"
echo "OK  dist/paddock-$VERSION-windows-x64.zip"
