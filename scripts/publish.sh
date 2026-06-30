#!/usr/bin/env bash
set -euo pipefail

# Publish all ast-sgrep crates to crates.io in dependency order.
# Requires: cargo login <token>

crates=(
  ast-sgrep-lang
  ast-sgrep-embed
  ast-sgrep-core
  ast-sgrep-cli
  ast-sgrep-lsp
)

for crate in "${crates[@]}"; do
  echo "Publishing ${crate}..."
  cargo publish -p "${crate}"
  echo "Waiting for crates.io index..."
  sleep 30
done

echo "All crates published."
