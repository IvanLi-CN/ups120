#!/usr/bin/env bash
set -euo pipefail

# Format only staged Rust files without requiring a Cargo workspace at repo root.
files=$(git diff --cached --name-only --diff-filter=ACM | grep -E '\\.rs$' || true)
if [ -z "${files}" ]; then
  exit 0
fi

echo "$files" | xargs -I{} rustfmt --edition 2021 {}

