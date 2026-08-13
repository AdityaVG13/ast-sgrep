#!/usr/bin/env bash
# Missing QUERY on keyword/semantic must teach an exact --json example + triad footer.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
BIN="${ASGREP_BIN:-$ROOT/target/release-perf/asgrep}"
if [[ ! -x "$BIN" ]]; then BIN="$ROOT/target/debug/asgrep"; fi
if [[ ! -x "$BIN" ]]; then echo "skip: no asgrep binary" >&2; exit 0; fi

check_human() {
  local cmd="$1"
  local out ec
  set +e
  out=$("$BIN" "$cmd" 2>&1)
  ec=$?
  set -e
  [[ $ec -eq 1 ]] || { echo "FAIL: $cmd exit=$ec want 1"; echo "$out"; exit 1; }
  echo "$out" | grep -Fq "Example: asgrep $cmd --json" || {
    echo "FAIL: $cmd missing Example line"; echo "$out"; exit 1
  }
  echo "$out" | grep -Fq "Agent surfaces:" || {
    echo "FAIL: $cmd missing triad footer"; echo "$out"; exit 1
  }
  echo "$out" | grep -Fq "Tip: QUERY is required" || {
    echo "FAIL: $cmd missing QUERY tip"; echo "$out"; exit 1
  }
}

check_json() {
  local cmd="$1"
  local out ec
  set +e
  out=$("$BIN" "$cmd" --json 2>&1)
  ec=$?
  set -e
  [[ $ec -eq 1 ]] || { echo "FAIL: $cmd --json exit=$ec want 1"; echo "$out"; exit 1; }
  echo "$out" | grep -Fq "Example: asgrep $cmd --json" || {
    echo "FAIL: $cmd --json missing Example"; echo "$out"; exit 1
  }
  echo "$out" | grep -Eq '"kind": ?"usage"' || {
    echo "FAIL: $cmd --json not usage envelope"; echo "$out"; exit 1
  }
}

check_human keyword
check_human semantic
check_json keyword
check_json semantic

# Unknown flag also gets triad footer (not bare "try --help").
set +e
out=$("$BIN" --not-a-real-flag 2>&1)
ec=$?
set -e
[[ $ec -eq 1 ]] || { echo "FAIL: unknown flag exit=$ec"; exit 1; }
echo "$out" | grep -Fq "Agent surfaces:" || {
  echo "FAIL: unknown flag missing footer"; echo "$out"; exit 1
}

echo "ok R-003 missing_query_teaches"
