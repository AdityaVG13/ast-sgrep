#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
BIN="${ASGREP_BIN:-$ROOT/target/release-perf/asgrep}"
if [[ ! -x "$BIN" ]]; then BIN="$ROOT/target/debug/asgrep"; fi
if [[ ! -x "$BIN" ]]; then echo "skip"; exit 0; fi
out=$("$BIN" search --json --format jason foo . 2>&1 || true)
echo "$out" | grep -q "did you mean 'compact'" || { echo "FAIL: missing did-you-mean"; echo "$out"; exit 1; }
echo "$out" | grep -q 'asgrep --json --format compact' || { echo "FAIL: missing exact command"; exit 1; }
echo "ok R-002 format_typo"
