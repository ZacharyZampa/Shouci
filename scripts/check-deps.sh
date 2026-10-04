#!/usr/bin/env bash
# Architecture boundaries, checked on every run of check.sh:
#
#   - connectors (vocab-pleco, vocab-anki) depend on vocab-core only, so a
#     new format never sees storage;
#   - frontends (shouci-cli, shouci-tui, shouci-ffi) depend on
#     shouci-core only, so every UI is a projection of the same core.
#
# Dependencies come from `cargo metadata`, so every way a Cargo.toml can
# declare one counts: inline, as a `[dependencies.name]` table, for one
# target only, or renamed. An argument names another workspace to check
# (scripts/check-deps-test.sh uses it).
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
status=0

# One line per workspace member: its name, then its normal dependencies. A
# crate outside the workspace is not built, so it isn't listed.
GRAPH="$(cargo metadata --format-version 1 --no-deps --offline --manifest-path "${ROOT}/Cargo.toml" |
  jq -r '.packages[] | [.name, (.dependencies[] | select(.kind == null) | .name)] | join(" ")')"

check() {
  local crate="$1" allowed="$2" dep
  for dep in $(awk -v crate="${crate}" '$1 == crate { $1 = ""; print }' <<<"${GRAPH}"); do
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
for frontend in shouci-cli shouci-tui shouci-ffi; do
  check "${frontend}" "shouci-core"
done

exit "${status}"
