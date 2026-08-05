#!/usr/bin/env bash
# run-benchmarks.sh — reproducible benchmark run for the ast-sgrep release state.
# Produces the rows published in benchmarks/results/speed.md + head-to-head.md.
#
# Prereqs: hyperfine, rg, ast-grep on PATH; a release-perf build:
#   cargo build --profile release-perf -p ast-sgrep-cli
# Usage:
#   scripts/run-benchmarks.sh <asgrep-binary> <self-corpus-dir> <out-dir>
# The self corpus should be a checkout of the tracked files only:
#   git ls-files | rsync -a --files-from=- . <corpus-dir>
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ASGREP="$(cd "$(dirname "${1:?asgrep binary path}")" && pwd)/$(basename "$1")"
SELF="${2:?self corpus dir}"
OUT="${3:?out dir}"
mkdir -p "$OUT"

echo "== versions =="
"$ASGREP" --version 2>&1 | head -1 || true
rg --version | head -1
ast-grep --version | head -1
hyperfine --version

echo "== cold self-index (p95) =="
hyperfine --warmup 0 --runs 5 --export-json "$OUT/index_self.json" \
  --prepare "rm -rf \"$SELF/.asgrep\"" \
  "$ASGREP index \"$SELF\"" >/dev/null
python3 - "$OUT/index_self.json" <<'EOF'
import json, sys
d = json.load(open(sys.argv[1]))
r = d["results"][0]
t = sorted(r["times"])
p95 = t[int(0.95 * len(t)) - 1]
print(f"  cold index: mean {r['mean']*1000:.1f} ms, median {r['median']*1000:.1f} ms, p95 {p95*1000:.1f} ms")
EOF

echo "== warm literal vs ripgrep (self corpus) =="
hyperfine --warmup 1 --runs 8 --export-json "$OUT/literal.json" \
  "$ASGREP 'literal:auth_refresh' '$SELF' --limit 10" \
  "rg -n 'auth_refresh' '$SELF'" >/dev/null
python3 - "$OUT/literal.json" <<'EOF'
import json, sys
for r in json.load(open(sys.argv[1]))["results"]:
    t = sorted(r["times"])
    p95 = t[int(0.95 * len(t)) - 1]
    print(f"  {r['command'][:64]:64s} mean {r['mean']*1000:7.1f} ms  p95 {p95*1000:7.1f} ms")
EOF

echo "== warm semantic NL query (self corpus) =="
hyperfine --warmup 1 --runs 8 --export-json "$OUT/nl.json" \
  "$ASGREP semantic 'credential renewal' '$SELF' --limit 5" >/dev/null
python3 - "$OUT/nl.json" <<'EOF'
import json, sys
r = json.load(open(sys.argv[1]))["results"][0]
t = sorted(r["times"])
p95 = t[int(0.95 * len(t)) - 1]
print(f"  semantic NL: mean {r['mean']*1000:.1f} ms, median {r['median']*1000:.1f} ms, p95 {p95*1000:.1f} ms")
EOF

echo "== structural pattern vs ast-grep (self corpus) =="
hyperfine --warmup 1 --runs 8 --export-json "$OUT/pattern.json" --ignore-failure \
  "$ASGREP 'pattern:for (\$_) in (\$_)' '$SELF' --limit 10" \
  "ast-grep -p 'for (\$_) in (\$_)' '$SELF'" >/dev/null || true
python3 - "$OUT/pattern.json" <<'EOF'
import json, sys
for r in json.load(open(sys.argv[1]))["results"]:
    t = sorted(r["times"])
    p95 = t[int(0.95 * len(t)) - 1]
    print(f"  {r['command'][:64]:64s} mean {r['mean']*1000:7.1f} ms  p95 {p95*1000:7.1f} ms")
EOF

echo "== index size =="
du -sh "$SELF/.asgrep" | awk '{print "  .asgrep:", $1}'

echo "== error budget: cold self-index vs 285 ms p95 threshold =="
python3 "$HERE/check-error-budget.py" "$OUT/index_self.json" --label cold-index-self \
  --threshold-ms 285 --slo 0.95 --baseline-p95-ms 258.4 2>&1 | tail -3 || true

echo "done — artifacts in $OUT"
