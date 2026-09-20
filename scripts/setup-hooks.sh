#!/usr/bin/env bash
# Point this clone at scripts/githooks (pre-commit → scripts/check.sh).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"
git config core.hooksPath scripts/githooks
chmod +x scripts/githooks/pre-commit scripts/check.sh
echo "git hooks: core.hooksPath=scripts/githooks"
echo "pre-commit runs ./scripts/check.sh (fmt, clippy, cargo test --workspace)"
