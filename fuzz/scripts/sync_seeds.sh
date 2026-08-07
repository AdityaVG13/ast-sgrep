#!/usr/bin/env bash
# Copy committed L1 seeds into cargo-fuzz's gitignored corpus/ dirs.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ ! -d seed_corpus ]]; then
  echo "no seed_corpus/ — nothing to sync" >&2
  exit 0
fi
for target_dir in seed_corpus/*/; do
  [[ -d "$target_dir" ]] || continue
  name="$(basename "$target_dir")"
  dest="corpus/${name}"
  mkdir -p "$dest"
  # -n: do not overwrite evolved corpus entries
  cp -n "${target_dir}"* "$dest/" 2>/dev/null || true
  count="$(find "$dest" -type f | wc -l | tr -d ' ')"
  echo "sync_seeds: $name → $dest ($count files)"
done
