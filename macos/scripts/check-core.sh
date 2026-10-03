#!/bin/sh
# Xcode build phase: fail early, with the fix, when the Rust core is missing
# or older than its sources. A stale core links, then fails its checksum
# check at launch.
set -eu

MACOS="$(cd "$(dirname "$0")/.." && pwd)"
STAMP="${MACOS}/ShouciCore/ShouciFFI.xcframework/Info.plist"

if [ ! -f "${STAMP}" ]; then
  echo "error: The Rust core is not built. Run macos/scripts/build-core.sh"
  exit 1
fi
changed="$(find "${MACOS}/../crates" -newer "${STAMP}" \
  \( -name '*.rs' -o -name 'Cargo.toml' -o -name 'uniffi.toml' \) -print | head -1)"
if [ -n "${changed}" ]; then
  echo "error: The Rust core changed since it was built (${changed#"${MACOS}/../"}). Run macos/scripts/build-core.sh"
  exit 1
fi
