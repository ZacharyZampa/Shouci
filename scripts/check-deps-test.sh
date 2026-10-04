#!/usr/bin/env bash
# The boundary check must refuse every way a Cargo.toml can declare a
# forbidden dependency, and pass a clean workspace. Run by check.sh.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "${SCRATCH}"' EXIT
status=0

# A workspace (members on one line) whose shouci-cli manifest ends with $2.
workspace() {
  local dir="${SCRATCH}/$1" crate
  printf '[workspace]\nmembers = ["crates/shouci-cli", "crates/shouci-core"]\nresolver = "2"\n' \
    >"$(mkdir -p "${dir}" && echo "${dir}/Cargo.toml")"
  for crate in shouci-cli shouci-core; do
    mkdir -p "${dir}/crates/${crate}/src"
    touch "${dir}/crates/${crate}/src/lib.rs"
    printf '[package]\nname = "%s"\nversion = "0.0.0"\nedition = "2021"\n' "${crate}" \
      >"${dir}/crates/${crate}/Cargo.toml"
  done
  printf '\n[dependencies]\nshouci-core = { path = "../shouci-core" }\n%b' "$2" \
    >>"${dir}/crates/shouci-cli/Cargo.toml"
  echo "${dir}"
}

# $1 is pass or fail: what checking the workspace named $2 must do.
expect() {
  local dir result
  dir="$(workspace "$2" "$3")"
  if "${ROOT}/scripts/check-deps.sh" "${dir}" 2>/dev/null; then result=pass; else result=fail; fi
  if [ "${result}" != "$1" ]; then
    echo "check-deps: a workspace with ${2} should ${1}, but it did not" >&2
    status=1
  fi
}

expect pass "only shouci-core" ''
expect fail "an inline dependency" 'rusqlite = "0.31"\n'
expect fail "a dependency table" '\n[dependencies.rusqlite]\nversion = "0.31"\n'
expect fail "a target dependency" "\n[target.'cfg(unix)'.dependencies]\nrusqlite = \"0.31\"\n"
expect fail "a renamed dependency" 'db = { package = "vocab-db", version = "0.1" }\n'

exit "${status}"
