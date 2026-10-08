#!/usr/bin/env bash
# Builds Shouci.app for release (Apple silicon + Intel, signed to run
# locally) at the path given. `scripts/install.sh` installs it with the
# terminal tools, and `make-dmg.sh` wraps it in a disk image.
#
#   macos/scripts/package.sh app-path
#
# Builds in a staging directory: a failed build never touches the app
# already at app-path.
set -euo pipefail

MACOS="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ $# -ne 1 ]; then
  echo "usage: ${0} app-path   (to install, run scripts/install.sh)" >&2
  exit 2
fi
OUT="${1}"
BUILD="$(mktemp -d)"
trap 'rm -rf "${BUILD}"' EXIT

"${MACOS}/scripts/build-core.sh" --release
xcodebuild -project "${MACOS}/Shouci.xcodeproj" -scheme Shouci -configuration Release \
  -destination 'generic/platform=macOS' \
  -derivedDataPath "${BUILD}" -quiet build

# Copied beside the app already there first, so a failed copy leaves it
# alone.
mkdir -p "$(dirname "${OUT}")"
rm -rf "${OUT}.new"
ditto "${BUILD}/Build/Products/Release/Shouci.app" "${OUT}.new"
rm -rf "${OUT}"
mv "${OUT}.new" "${OUT}"
echo "built: ${OUT}"
