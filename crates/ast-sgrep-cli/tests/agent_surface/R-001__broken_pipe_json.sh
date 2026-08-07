#!/usr/bin/env bash
# Agents pipe JSON through head; CLI must not panic on broken pipe.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
BIN="${ASGREP_BIN:-$ROOT/target/release-perf/asgrep}"
if [[ ! -x "$BIN" ]]; then BIN="$ROOT/target/debug/asgrep"; fi
if [[ ! -x "$BIN" ]]; then
  echo "skip: no asgrep binary" >&2
  exit 0
fi
set +e
err=$(mktemp)
"$BIN" --json --format compact "fn" "$ROOT" 2>"$err" | head -c 20 >/dev/null
ec=$?
set -e
if grep -qi 'panicked\|Broken pipe' "$err"; then
  echo "FAIL: panic/broken-pipe noise on stderr:" >&2
  cat "$err" >&2
  exit 1
fi
# exit 0 or 141 (SIGPIPE) both ok depending on shell; panic is not
if [[ $ec -ne 0 && $ec -ne 141 && $ec -ne 1 ]]; then
  # 1 would be usage; search should succeed
  echo "WARN: unexpected exit $ec" >&2
fi
echo "ok R-001 broken_pipe"
