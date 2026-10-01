#!/usr/bin/env bash
# Architecture boundaries, checked on every run of check.sh:
#
#   - connectors (vocab-pleco, vocab-anki) depend on vocab-core only, so a
#     new format never sees storage;
#   - frontends (shouci-cli, shouci-tui, shouci-mac, shouci-ffi) depend on
#     shouci-core only, so every UI is a projection of the same core.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
status=0

# Prints the dependency names in a crate's [dependencies] table.
deps() {
  awk '/^\[dependencies\]/{on=1; next} /^\[/{on=0} on && /^[a-zA-Z0-9_-]+ *=/{print $1}' "$1"
}

# Workspace members only; a crate outside the workspace is not built.
MEMBERS="$(awk '/^members = \[/{on=1; next} on && /\]/{on=0} on{print}' "${ROOT}/Cargo.toml")"

check() {
  local crate="$1" allowed="$2"
  local manifest="${ROOT}/crates/${crate}/Cargo.toml"
  [ -f "${manifest}" ] || return 0
  grep -q "\"crates/${crate}\"" <<<"${MEMBERS}" || return 0
  local dep
  for dep in $(deps "${manifest}"); do
    case "${dep}" in
      vocab-*|shouci-*|rusqlite)
        if ! grep -qw -- "${dep}" <<<"${allowed}"; then
          echo "boundary: ${crate} must not depend on ${dep} (allowed: ${allowed})" >&2
          status=1
        fi
        ;;
    esac
  done
}

for connector in vocab-pleco vocab-anki; do
  check "${connector}" "vocab-core"
done
for frontend in shouci-cli shouci-tui shouci-mac shouci-ffi; do
  check "${frontend}" "shouci-core"
done

exit "${status}"
