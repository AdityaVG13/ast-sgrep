#!/usr/bin/env bash
# Minimize evolved corpora (run after long campaigns; requires cargo-fuzz + nightly).
set -euo pipefail
cd "$(dirname "$0")/.."
bash scripts/sync_seeds.sh
for target in $(cargo +nightly fuzz list 2>/dev/null || true); do
  echo "cmin: $target"
  cargo +nightly fuzz cmin "$target" || true
done
