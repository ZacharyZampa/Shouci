#!/usr/bin/env bash
# Build and install the `shouci` CLI, `shouci-tui`, and Shouci on macOS.
#
#   scripts/install.sh              install
#   scripts/install.sh --status     show install state
#   scripts/install.sh --uninstall  stop Shouci and remove leftover agent
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OLD_AGENT_LABEL="com.plecocompanion.agent"
BIN_DIR="${HOME}/.local/bin"
APP_SUPPORT="${HOME}/Library/Application Support/pleco-companion"
OLD_AGENT_PLIST="${HOME}/Library/LaunchAgents/${OLD_AGENT_LABEL}.plist"
CLI_BIN="${BIN_DIR}/shouci"
TUI_BIN="${BIN_DIR}/shouci-tui"
MACAPP="${HOME}/Applications/Shouci.app"
CARGO_BIN="${HOME}/.cargo/bin"
DICT="${APP_SUPPORT}/dictionary.db"

platform_check() {
  local os
  os="$(uname -s)"
  if [ "${os}" != "Darwin" ]; then
    echo "error: this script is macOS-only (Shouci uses AppKit)" >&2
    exit 1
  fi
}

source_db() {
  local cand
  for cand in "${ROOT}/data/dictionary/dictionary.db" \
    "${ROOT}/data/dictionary/sources/dictionary.db"; do
    if [ -f "${cand}" ]; then
      printf '%s' "${cand}"
      return 0
    fi
  done
  return 1
}

remove_legacy_agent() {
  launchctl bootout "gui/$(id -u)/${OLD_AGENT_LABEL}" >/dev/null 2>&1 || true
  pkill -x vocab-agent 2>/dev/null || true
  rm -f "${OLD_AGENT_PLIST}" "${BIN_DIR}/vocab-agent" "${CARGO_BIN}/vocab-agent"
}

status() {
  if [ -d "${MACAPP}" ]; then
    echo "menubar: ${MACAPP} installed"
  else
    echo "menubar: not installed"
  fi
  if pgrep -x Shouci >/dev/null 2>&1; then
    echo "menubar: running (pid $(pgrep -x Shouci | tr '\n' ' '))"
  else
    echo "menubar: not running"
  fi
  if [ -x "${CLI_BIN}" ]; then
    echo "cli: ${CLI_BIN}"
  else
    echo "cli: not installed"
  fi
  if [ -x "${TUI_BIN}" ]; then
    echo "tui: ${TUI_BIN}"
  else
    echo "tui: not installed"
  fi
  if pgrep -x vocab-agent >/dev/null 2>&1 || [ -f "${OLD_AGENT_PLIST}" ]; then
    echo "legacy agent: still present — run --uninstall or reinstall to remove"
  fi
}

do_uninstall() {
  echo "==> removing leftover capture agent"
  remove_legacy_agent
  echo "removing menu-bar app"
  pkill -x Shouci 2>/dev/null || true
  rm -rf "${MACAPP}"
  echo "removing PATH symlinks"
  for bin in shouci shouci-tui; do
    rm -f "${CARGO_BIN}/${bin}"
  done
  echo "note: left ${BIN_DIR} binaries and ${APP_SUPPORT} data in place; delete them manually if desired."
  echo "done."
}

do_install() {
  echo "==> dictionary (CC-CEDICT + frequency + HSK)"
  cargo run -p vocab-dictionary --example ensure -- "${ROOT}/data/dictionary/dictionary.db"

  echo "==> building release binaries"
  cargo build --release -p shouci-cli -p shouci-tui -p shouci-mac

  local dict
  dict="$(source_db)" || {
    echo "error: dictionary ingest did not produce data/dictionary/dictionary.db" >&2
    exit 1
  }

  mkdir -p "${BIN_DIR}" "${APP_SUPPORT}"

  echo "==> installing binaries to ${BIN_DIR}"
  install -m 0755 "${ROOT}/target/release/shouci" "${CLI_BIN}"
  install -m 0755 "${ROOT}/target/release/shouci-tui" "${TUI_BIN}"

  echo "==> linking binaries onto PATH (${CARGO_BIN})"
  mkdir -p "${CARGO_BIN}"
  for bin in shouci shouci-tui; do
    ln -sfn "${BIN_DIR}/${bin}" "${CARGO_BIN}/${bin}"
  done

  echo "==> installing dictionary to ${DICT}"
  install -m 0644 "${dict}" "${DICT}"

  echo "==> packaging menu-bar app to ${MACAPP}"
  "${ROOT}/crates/shouci-mac/package.sh" "${MACAPP}"
  echo "note: if Shouci was running it was replaced on disk; relaunch it from ${MACAPP}"

  echo "==> removing leftover capture agent"
  remove_legacy_agent

  echo
  echo "done."
  echo
  echo "quick:   press ctrl+opt+v anywhere (Shouci)"
  echo "list:    ${CLI_BIN} list"
  echo "tui:     ${TUI_BIN}"
  echo "remove:  ${0} --uninstall"
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
