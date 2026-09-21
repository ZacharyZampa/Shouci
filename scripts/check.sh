#!/usr/bin/env bash
# The contributor gate: format, clippy, and the full test suite.
# Used by the pre-commit hook and GitHub Actions.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"
export VOCAB_SKIP_DICTIONARY_REFRESH=1
export VOCAB_DICTIONARY="${ROOT}/data/dictionary/dictionary.db"

if [ ! -f data/dictionary/dictionary.db ]; then
  echo "==> dictionary.db missing, fetching"
  cargo run -p vocab-dictionary --example ensure -- data/dictionary/dictionary.db
fi

echo "==> cargo fmt --check"
cargo fmt --all -- --check

echo "==> cargo clippy"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo test --workspace"
cargo test --workspace

echo "ok"
