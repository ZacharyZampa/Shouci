#!/usr/bin/env bash
# Builds the Rust core (crates/shouci-ffi) for the Mac app: an XCFramework
# with the static library, plus the Swift bindings. Run it before building
# the app and after any change to shouci-ffi or shouci-core.
#
#   macos/scripts/build-core.sh            # debug, this Mac's architecture
#   macos/scripts/build-core.sh --release  # release, Apple silicon + Intel
set -euo pipefail

MACOS="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT="$(cd "${MACOS}/.." && pwd)"
PACKAGE="${MACOS}/ShouciCore"
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
# The app's minimum macOS; the bundled SQLite (C) otherwise targets this
# Mac's version. A separate target directory keeps the workspace's own
# builds from recompiling every time this one runs.
export MACOSX_DEPLOYMENT_TARGET=14.0
export CARGO_TARGET_DIR="${ROOT}/target/macos"

cd "${ROOT}"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

if [ "${1:-}" = "--release" ]; then
  installed="$(rustup target list --installed)"
  for target in "${TARGETS[@]}"; do
    if ! grep -qx "${target}" <<<"${installed}"; then
      echo "==> adding Rust target ${target}"
      rustup target add "${target}"
    fi
    cargo build -p shouci-ffi --release --target "${target}"
  done
  lipo -create -output "${work}/libshouci_ffi.a" \
    "${CARGO_TARGET_DIR}/aarch64-apple-darwin/release/libshouci_ffi.a" \
    "${CARGO_TARGET_DIR}/x86_64-apple-darwin/release/libshouci_ffi.a"
  interface="${CARGO_TARGET_DIR}/aarch64-apple-darwin/release/libshouci_ffi.dylib"
  profile=release
else
  cargo build -p shouci-ffi
  cp "${CARGO_TARGET_DIR}/debug/libshouci_ffi.a" "${work}/libshouci_ffi.a"
  interface="${CARGO_TARGET_DIR}/debug/libshouci_ffi.dylib"
  profile=debug
fi

# The generator builds in its own target directory: building it in the main
# one would rebuild the library above with the generator's features.
CARGO_TARGET_DIR="${ROOT}/target/bindgen" \
  cargo run -q -p shouci-ffi --features bindgen --bin uniffi-bindgen -- \
  generate --library "${interface}" --language swift --out-dir "${work}/generated"

mkdir -p "${work}/include"
cp "${work}/generated/ShouciFFI.h" "${work}/include/ShouciFFI.h"
cp "${work}/generated/ShouciFFI.modulemap" "${work}/include/module.modulemap"

rm -rf "${PACKAGE}/ShouciFFI.xcframework"
xcodebuild -create-xcframework -library "${work}/libshouci_ffi.a" \
  -headers "${work}/include" -output "${PACKAGE}/ShouciFFI.xcframework" >/dev/null
cp "${work}/generated/ShouciCore.swift" "${PACKAGE}/Sources/ShouciCore/ShouciCore.swift"

echo "core: ${profile} build and Swift bindings in ${PACKAGE}"
