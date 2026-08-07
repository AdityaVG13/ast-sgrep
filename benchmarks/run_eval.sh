#!/usr/bin/env bash
# Reproducible evaluation pack (bead ast-sgrep-tef-eval-pack-d2dv).
#
# Regenerates every retrieval and token-efficiency number recorded for the
# `self` corpus in benchmarks/results/baselines.md, from a clean checkout, with
# no network access and no external corpora.
#
#   ./benchmarks/run_eval.sh            # build + evaluate + write raw artifacts
#
# Raw artifacts land in benchmarks/results/raw/ and are checked in, so a reader
# can diff a new run against the recorded one instead of trusting a summary.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

gold="benchmarks/gold/self.json"
out="benchmarks/results/raw"
mkdir -p "$out"

bin="target/release/asgrep"
if [[ "${ASGREP_EVAL_DEBUG:-0}" == "1" ]]; then
  bin="target/debug/asgrep"
  cargo build -p ast-sgrep-cli
else
  cargo build --release -p ast-sgrep-cli
fi

commit="$(git rev-parse HEAD)"
dirty="false"
if ! git diff --quiet || ! git diff --cached --quiet; then dirty="true"; fi

echo "== provenance =="
echo "commit=$commit dirty=$dirty"
"$bin" version --json > "$out/version.json"

# 1. Retrieval quality, and the hybrid vs no-embed A/B in one run.
echo "== retrieval quality (self corpus) =="
"$bin" eval --gold "$gold" . --json > "$out/self-quality.json"
"$bin" eval --gold "$gold" . --json --ab no-embed > "$out/self-ab-no-embed.json"

# 2. Token efficiency: bytes a model would actually receive, per format,
#    over the same gold queries. Compact is the agent-facing envelope.
echo "== token efficiency (self corpus) =="
# A dedicated, freshly built index keeps the measurement deterministic: the
# native/agent envelopes embed the snapshot generation, so reusing a repo index
# whose generation keeps incrementing would change the byte totals run to run.
token_index="$(mktemp -d)/index.db"
"$bin" --index-path "$token_index" index . >/dev/null
python3 - "$bin" "$gold" "$out" "$token_index" <<'PY'
import json, subprocess, sys
binary, gold_path, out_dir, index_path = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
gold = json.load(open(gold_path))
rows = []
for query in gold["queries"]:
    row = {"name": query["name"], "query": query["query"]}
    for fmt in ("native", "agent", "agent-capsule", "compact"):
        proc = subprocess.run(
            [binary, "--index-path", index_path, "--json", "--format", fmt,
             "--limit", "10", query["query"], "."],
            capture_output=True, text=True,
        )
        row[fmt] = len(proc.stdout.strip())
    rows.append(row)
totals = {
    fmt: sum(row[fmt] for row in rows)
    for fmt in ("native", "agent", "agent-capsule", "compact")
}
totals["compact_vs_agent_capsule_pct"] = round(
    100 - (totals["compact"] * 100 / totals["agent-capsule"]), 1
)
totals["compact_vs_native_pct"] = round(
    100 - (totals["compact"] * 100 / totals["native"]), 1
)
json.dump({"per_query": rows, "totals": totals}, open(f"{out_dir}/self-token-efficiency.json", "w"), indent=2)
print(json.dumps(totals, indent=2))
PY

# 3. Reliability invariants, as executable gates rather than prose claims.
echo "== reliability invariants =="
cargo test -p ast-sgrep-core --test snapshot_generation --test generation_swap \
  --test store_pragmas 2>&1 | tee "$out/reliability-tests.txt" | tail -5

python3 - "$out" "$commit" "$dirty" <<'PY'
import json, sys
out_dir, commit, dirty = sys.argv[1], sys.argv[2], sys.argv[3]
quality = json.load(open(f"{out_dir}/self-quality.json"))
ab = json.load(open(f"{out_dir}/self-ab-no-embed.json"))
tokens = json.load(open(f"{out_dir}/self-token-efficiency.json"))
summary = {
    "commit": commit,
    "dirty_worktree": dirty == "true",
    "corpus": "self",
    "gold_queries": quality["aggregate"]["n_queries"],
    "retrieval": quality["aggregate"],
    "hybrid_minus_no_embed": ab["diff"]["aggregate"],
    "token_efficiency": tokens["totals"],
}
json.dump(summary, open(f"{out_dir}/summary.json", "w"), indent=2)
print(json.dumps(summary, indent=2))
PY

echo "== done: artifacts in $out =="
