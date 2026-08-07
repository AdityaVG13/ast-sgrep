#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked

if ! command -v cargo-fuzz >/dev/null 2>&1; then
  echo "local release gate requires cargo-fuzz: cargo install cargo-fuzz --locked" >&2
  exit 1
fi
(
  cd fuzz
  bash scripts/sync_seeds.sh
  cargo +nightly fuzz run query_grammar -- -max_total_time=30 -timeout=5 -dict=dictionaries/query_grammar.dict
  cargo +nightly fuzz run rank -- -max_total_time=30 -timeout=5
)
