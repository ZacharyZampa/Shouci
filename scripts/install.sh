#!/usr/bin/env bash
# Build and install Shouci for Mac from this checkout.
#
#   scripts/install.sh              install, or update after `git pull`
#   scripts/install.sh --status     show what is installed and where
#   scripts/install.sh --uninstall  remove the app (your words stay)
#
# The CLI and TUI are moving onto the new core and are not built here yet.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MACAPP="${HOME}/Applications/Shouci.app"
# Where the app keeps its data; overridable the same way the app's is.
DATA_DIR="${SHOUCI_HOME:-${HOME}/Library/Application Support/Shouci}"
DICTIONARIES="${SHOUCI_DICTIONARIES:-${DATA_DIR}/dictionaries}"
LOGIN_AGENT="${HOME}/Library/LaunchAgents/com.zacharyzampa.shouci.plist"
OLD_AGENT_LABEL="com.plecocompanion.agent"
OLD_AGENT_PLIST="${HOME}/Library/LaunchAgents/${OLD_AGENT_LABEL}.plist"
OLD_DATA="${HOME}/Library/Application Support/pleco-companion"

platform_check() {
  if [ "$(uname -s)" != "Darwin" ]; then
    echo "error: Shouci for Mac needs macOS 14 or later" >&2
    exit 1
  fi
}

prerequisites() {
  if ! command -v cargo >/dev/null 2>&1; then
    echo "error: Rust is not installed. Install it from https://rustup.rs, then run this again." >&2
    exit 1
  fi
  if ! xcodebuild -version >/dev/null 2>&1; then
    echo "error: Xcode is needed (free from the App Store; the Command Line Tools alone are not enough)." >&2
    echo "       After installing it, run:" >&2
    echo "         sudo xcode-select -s /Applications/Xcode.app/Contents/Developer" >&2
    exit 1
  fi
}

quit_shouci() {
  if pgrep -x Shouci >/dev/null 2>&1; then
    echo "==> quitting the running Shouci"
    pkill -x Shouci || true
    for _ in $(seq 1 20); do
      pgrep -x Shouci >/dev/null 2>&1 || break
      sleep 0.25
    done
  fi
}

remove_legacy_agent() {
  launchctl bootout "gui/$(id -u)/${OLD_AGENT_LABEL}" >/dev/null 2>&1 || true
  pkill -x vocab-agent 2>/dev/null || true
  rm -f "${OLD_AGENT_PLIST}" "${HOME}/.local/bin/vocab-agent" "${HOME}/.cargo/bin/vocab-agent"
}

status() {
  if [ -d "${MACAPP}" ]; then
    local version
    version="$(defaults read "${MACAPP}/Contents/Info" CFBundleShortVersionString 2>/dev/null || echo "?")"
    echo "app:          ${MACAPP} (version ${version})"
  else
    echo "app:          not installed"
  fi
  if pgrep -x Shouci >/dev/null 2>&1; then
    echo "running:      yes (pid $(pgrep -x Shouci | tr '\n' ' '))"
  else
    echo "running:      no"
  fi
  if [ -f "${DATA_DIR}/user.db" ]; then
    echo "your words:   ${DATA_DIR}/user.db"
  else
    echo "your words:   none yet (created on first launch)"
  fi
  if [ -f "${DICTIONARIES}/cc-cedict.db" ]; then
    echo "dictionary:   ${DICTIONARIES}/cc-cedict.db"
  else
    echo "dictionary:   not built yet (downloaded on install or first launch)"
  fi
  if [ -f "${LOGIN_AGENT}" ]; then
    echo "at login:     opens Shouci"
  fi
  if [ -f "${OLD_DATA}/user.db" ]; then
    echo "older words:  ${OLD_DATA}/user.db (copied into the new library on first launch; left in place)"
  fi
}

do_uninstall() {
  quit_shouci
  echo "==> removing ${MACAPP}"
  rm -rf "${MACAPP}"
  rm -f "${LOGIN_AGENT}"
  remove_legacy_agent
  echo
  echo "Shouci is removed. Your words and dictionary are still in:"
  echo "  ${DATA_DIR}"
  echo "Delete that folder to remove them too."
}

do_install() {
  prerequisites

  echo "==> dictionary (CC-CEDICT with word frequency and HSK levels)"
  mkdir -p "${DICTIONARIES}"
  (cd "${ROOT}" && cargo run -q --release -p vocab-dictionary --example ensure -- \
    "${DICTIONARIES}/cc-cedict.db")

  echo "==> building Shouci (Apple silicon and Intel; the first build takes a few minutes)"
  local stage
  stage="$(mktemp -d)"
  trap 'rm -rf "${stage}"' RETURN
  "${ROOT}/macos/scripts/package.sh" "${stage}/Shouci.app"

  quit_shouci
  echo "==> installing to ${MACAPP}"
  mkdir -p "$(dirname "${MACAPP}")"
  rm -rf "${MACAPP}"
  ditto "${stage}/Shouci.app" "${MACAPP}"
  remove_legacy_agent

  echo "==> starting Shouci"
  open "${MACAPP}"

  echo
  echo "Done. Look for 文 in the menu bar."
  echo
  echo "  Quick search:  Control-Option-V from any app (change it in Settings)"
  echo "  Your library:  Command-O in quick search, or open Shouci from Spotlight"
  echo "  Update later:  git pull && ${0}"
  echo "  Remove:        ${0} --uninstall"
}

platform_check
case "${1:-}" in
  --status) status ;;
  --uninstall) do_uninstall ;;
  "" | --install) do_install ;;
  *)
    echo "usage: ${0} [--install|--status|--uninstall]" >&2
    exit 2
    ;;
esac
