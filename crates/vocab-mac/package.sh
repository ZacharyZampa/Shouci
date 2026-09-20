#!/bin/sh
# Packages the Rust menu-bar app into a bundle. Usage:
#   ./package.sh [output-dir]   # default: ~/Applications/VocabBar.app
#
# Builds release, stages into a temp dir (a failed build never touches the
# installed app), ad-hoc signs, then swaps into place.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${1:-$HOME/Applications/VocabBar.app}"
STAGE="$(mktemp -d)/VocabBar.app"
trap 'rm -rf "$(dirname "$STAGE")"' EXIT

cargo build --release -p vocab-mac 2>&1 | tail -1

mkdir -p "$STAGE/Contents/MacOS" "$STAGE/Contents/Resources"
cp "$HERE/target/release/vocab-mac" "$STAGE/Contents/MacOS/VocabBar"
cp "$HERE/crates/vocab-mac/Resources/Info.plist" "$STAGE/Contents/Info.plist"
codesign --force --deep -s - "$STAGE"

rm -rf "$OUT"
mv "$STAGE" "$OUT"
trap - EXIT

echo "installed: $OUT"
