#!/usr/bin/env bash
# Builds Shouci.app for release (Apple silicon + Intel, signed to run
# locally) and installs it.
#
#   macos/scripts/package.sh [app-path]   # default ~/Applications/Shouci.app
#
# Builds in a staging directory: a failed build never touches the installed
# app.
set -euo pipefail

MACOS="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-${HOME}/Applications/Shouci.app}"
BUILD="$(mktemp -d)"
trap 'rm -rf "${BUILD}"' EXIT

"${MACOS}/scripts/build-core.sh" --release
xcodebuild -project "${MACOS}/Shouci.xcodeproj" -scheme Shouci -configuration Release \
  -destination 'generic/platform=macOS' \
  -derivedDataPath "${BUILD}" -quiet build

mkdir -p "$(dirname "${OUT}")"
rm -rf "${OUT}"
ditto "${BUILD}/Build/Products/Release/Shouci.app" "${OUT}"
echo "installed: ${OUT}"
