#!/bin/sh
# Packages the Rust menu-bar app into a bundle. Usage:
#   ./package.sh [output-dir]   # default: ~/Applications/Shouci.app
#
# Builds release, stages into a temp dir (a failed build never touches the
# installed app), ad-hoc signs, then swaps into place.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${1:-$HOME/Applications/Shouci.app}"
STAGE="$(mktemp -d)/Shouci.app"
trap 'rm -rf "$(dirname "$STAGE")"' EXIT

cargo build --release -p shouci-mac 2>&1 | tail -1

mkdir -p "$STAGE/Contents/MacOS" "$STAGE/Contents/Resources"
cp "$HERE/target/release/shouci-mac" "$STAGE/Contents/MacOS/Shouci"
cp "$HERE/crates/shouci-mac/Resources/Info.plist" "$STAGE/Contents/Info.plist"

# Brand icon: render the iconset from the 1254² 文 master PNG, then pack to
# AppIcon.icns inside the bundle. `sips`/`iconutil` are standard on macOS.
ICONSET="$(mktemp -d)/AppIcon.iconset"
SRC="$HERE/crates/shouci-mac/Resources/AppIcon.png"
mkdir -p "$ICONSET"
sips -z 16 16 "$SRC" --out "$ICONSET/icon_16x16.png" >/dev/null
sips -z 32 32 "$SRC" --out "$ICONSET/icon_16x16@2x.png" >/dev/null
sips -z 32 32 "$SRC" --out "$ICONSET/icon_32x32.png" >/dev/null
sips -z 64 64 "$SRC" --out "$ICONSET/icon_32x32@2x.png" >/dev/null
sips -z 128 128 "$SRC" --out "$ICONSET/icon_128x128.png" >/dev/null
sips -z 256 256 "$SRC" --out "$ICONSET/icon_128x128@2x.png" >/dev/null
sips -z 256 256 "$SRC" --out "$ICONSET/icon_256x256.png" >/dev/null
sips -z 512 512 "$SRC" --out "$ICONSET/icon_256x256@2x.png" >/dev/null
sips -z 512 512 "$SRC" --out "$ICONSET/icon_512x512.png" >/dev/null
sips -z 1024 1024 "$SRC" --out "$ICONSET/icon_512x512@2x.png" >/dev/null
iconutil -c icns "$ICONSET" -o "$STAGE/Contents/Resources/AppIcon.icns"
rm -rf "$(dirname "$ICONSET")"
codesign --force --deep -s - "$STAGE"

rm -rf "$OUT"
mv "$STAGE" "$OUT"
trap - EXIT

echo "installed: $OUT"
