#!/usr/bin/env bash
# Build and install Shouci for Mac from this checkout: the app, and the
# `shouci` command and `shouci-tui` for the terminal.
#
#   scripts/install.sh              install, or update after `git pull`
#   scripts/install.sh --status     show what is installed and where
#   scripts/install.sh --uninstall  remove them (your words stay)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_ID="com.zacharyzampa.shouci"
# An update replaces the copy already installed, in either Applications
# folder (one dragged in from the disk image is in /Applications); a new
# install goes to ~/Applications, which needs no administrator.
if [ -d "/Applications/Shouci.app" ] && [ ! -d "${HOME}/Applications/Shouci.app" ]; then
  MACAPP="/Applications/Shouci.app"
else
  MACAPP="${HOME}/Applications/Shouci.app"
fi
# Where the app keeps its data; overridable the same way the app's is.
DATA_DIR="${SHOUCI_HOME:-${HOME}/Library/Application Support/Shouci}"
DICTIONARIES="${SHOUCI_DICTIONARIES:-${DATA_DIR}/dictionaries}"
LOGIN_AGENT="${HOME}/Library/LaunchAgents/com.zacharyzampa.shouci.plist"
# The terminal tools live in BIN_DIR, linked from ~/.cargo/bin, which rustup
# puts on PATH.
BIN_DIR="${HOME}/.local/bin"
CARGO_BIN="${HOME}/.cargo/bin"
TOOLS=(shouci shouci-tui)
OLD_AGENT_LABEL="com.plecocompanion.agent"
OLD_AGENT_PLIST="${HOME}/Library/LaunchAgents/${OLD_AGENT_LABEL}.plist"

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

is_running() {
  # By bundle id: a development build, also named Shouci, has another.
  [ "$(osascript -e "application id \"${APP_ID}\" is running" 2>/dev/null)" = "true" ]
}

# Asks Shouci to quit, as its Quit command does, so a note still being typed
# is saved; stops it only when it hasn't quit within 15 seconds.
quit_shouci() {
  is_running || return 0
  echo "==> quitting the running Shouci"
  osascript -e "tell application id \"${APP_ID}\" to quit" >/dev/null 2>&1 || true
  for _ in $(seq 1 60); do
    is_running || return 0
    sleep 0.25
  done
  echo "    it didn't quit in time; stopping it"
  pkill -f "^${MACAPP}/Contents/MacOS/Shouci$" || true
  sleep 1
  if is_running; then
    echo "error: Shouci is still running; quit it, then run this again" >&2
    exit 1
  fi
}

install_tools() {
  echo "==> installing shouci and shouci-tui to ${BIN_DIR}"
  mkdir -p "${BIN_DIR}" "${CARGO_BIN}"
  local tool
  for tool in "${TOOLS[@]}"; do
    install -m 0755 "${ROOT}/target/release/${tool}" "${BIN_DIR}/${tool}"
    ln -sfn "${BIN_DIR}/${tool}" "${CARGO_BIN}/${tool}"
  done
}

remove_tools() {
  local tool
  for tool in "${TOOLS[@]}"; do
    # Only the link this script made; a `cargo install` copy stays.
    if [ "$(readlink "${CARGO_BIN}/${tool}" 2>/dev/null)" = "${BIN_DIR}/${tool}" ]; then
      rm -f "${CARGO_BIN}/${tool}"
    fi
    rm -f "${BIN_DIR}/${tool}"
  done
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
  if is_running; then
    echo "running:      yes"
  else
    echo "running:      no"
  fi
  local tool
  for tool in "${TOOLS[@]}"; do
    if [ -x "${BIN_DIR}/${tool}" ]; then
      printf '%-13s %s\n' "${tool}:" "${BIN_DIR}/${tool}"
    else
      printf '%-13s %s\n' "${tool}:" "not installed"
    fi
  done
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
}

do_uninstall() {
  quit_shouci
  echo "==> removing ${MACAPP}, shouci, and shouci-tui"
  rm -rf "${MACAPP}"
  rm -f "${LOGIN_AGENT}"
  remove_tools
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
  # Global, and removed on EXIT: a RETURN trap never runs when `set -e`
  # stops the script mid-function.
  STAGE="$(mktemp -d)"
  trap 'rm -rf "${STAGE}"' EXIT
  "${ROOT}/macos/scripts/package.sh" "${STAGE}/Shouci.app"

  echo "==> building shouci and shouci-tui"
  (cd "${ROOT}" && cargo build -q --release -p shouci-cli -p shouci-tui)

  quit_shouci
  echo "==> installing to ${MACAPP}"
  # Copied beside the installed app first, so a failed copy leaves it alone.
  mkdir -p "$(dirname "${MACAPP}")"
  rm -rf "${MACAPP}.new"
  ditto "${STAGE}/Shouci.app" "${MACAPP}.new"
  rm -rf "${MACAPP}"
  mv "${MACAPP}.new" "${MACAPP}"
  install_tools
  remove_legacy_agent

  echo "==> starting Shouci"
  open "${MACAPP}"

  echo
  echo "Done. Look for 文 in the menu bar."
  echo
  echo "  Quick search:  Control-Option-V from any app (change it in Settings)"
  echo "  Your library:  Command-O in quick search, or open Shouci from Spotlight"
  echo "  Terminal:      shouci --help, or shouci-tui"
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
