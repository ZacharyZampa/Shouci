#!/usr/bin/env bash
# Builds Shouci.app for release (Apple silicon + Intel, signed to run
# locally) and wraps it in a disk image with a shortcut to Applications.
#
#   macos/scripts/make-dmg.sh [output-dir]   # default <repo>/dist
#
# Writes Shouci-<version>.dmg and its .sha256. The app is not notarized, so a
# downloaded copy needs Open Anyway the first time (see the README).
set -euo pipefail

MACOS="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-${MACOS}/../dist}"
STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE}"' EXIT

"${MACOS}/scripts/package.sh" "${STAGE}/Shouci.app"

VERSION="$(defaults read "${STAGE}/Shouci.app/Contents/Info" CFBundleShortVersionString)"
mkdir -p "${OUT}"
OUT="$(cd "${OUT}" && pwd)"
DMG="${OUT}/Shouci-${VERSION}.dmg"

# What the disk image shows: the app and a link to drop it on.
mkdir "${STAGE}/disk"
mv "${STAGE}/Shouci.app" "${STAGE}/disk/Shouci.app"
ln -s /Applications "${STAGE}/disk/Applications"

rm -f "${DMG}"
hdiutil create -quiet -volname "Shouci" -srcfolder "${STAGE}/disk" \
  -format UDZO -fs HFS+ "${DMG}"
(cd "${OUT}" && shasum -a 256 "$(basename "${DMG}")" > "$(basename "${DMG}").sha256")

echo "built: ${DMG}"
cat "${DMG}.sha256"
