#!/usr/bin/env bash
# Reproducible evaluation pack (bead ast-sgrep-tef-eval-pack-d2dv).
#
# Regenerates the candidate retrieval and token-efficiency measurements for the
# `self` corpus in benchmarks/results/baselines.md. It does not fetch external
# corpora; Rust dependencies must already be available or downloadable by Cargo.
#
#   ./benchmarks/run_eval.sh            # build + evaluate + write raw artifacts
#
# Raw artifacts land in ignored benchmarks/results/raw/. Promote only a clean,
# reviewed fingerprint row to baselines.md; generated payloads are local evidence.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ -n "$(git status --porcelain --untracked-files=normal)" ]]; then
  echo "error: evaluation requires a clean worktree so results identify one commit" >&2
  exit 1
fi

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

echo "== provenance =="
echo "commit=$commit dirty=false"
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
token_work="$(mktemp -d)"
trap 'rm -rf -- "$token_work"' EXIT
token_index="$token_work/index.db"
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
            capture_output=True, text=True, check=True,
        )
        row[fmt] = len(proc.stdout.strip())
    # m38g: budgeted compact picks per-result detail under one ceiling.
    proc = subprocess.run(
        [binary, "--index-path", index_path, "--json", "--format", "compact",
         "--budget-tokens", "300", "--limit", "10", query["query"], "."],
        capture_output=True, text=True, check=True,
    )
    row["compact-budget300"] = len(proc.stdout.strip())
    rows.append(row)
formats = ("native", "agent", "agent-capsule", "compact", "compact-budget300")
totals = {fmt: sum(row[fmt] for row in rows) for fmt in formats}
totals["budget300_vs_agent_capsule_pct"] = round(
    100 - (totals["compact-budget300"] * 100 / totals["agent-capsule"]), 1
)
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
echo "== budget non-inferiority (m38g) =="
python3 - "$bin" "$gold" "$out" "$token_index" <<'BUDGETPY'
import json, subprocess, sys
binary, gold_path, out_dir, index_path = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
gold = json.load(open(gold_path))


def result_ids(query_text, extra):
    proc = subprocess.run(
        [binary, "--index-path", index_path, "--json", "--format", "compact",
         "--limit", "10", *extra, query_text, "."],
        capture_output=True, text=True, check=True,
    )
    body = json.loads(proc.stdout)
    return [row[0] for row in body.get("h", [])]


mismatches = []
for query in gold["queries"]:
    plain = result_ids(query["query"], [])
    budgeted = result_ids(query["query"], ["--budget-tokens", "300"])
    if plain != budgeted:
        mismatches.append({"query": query["name"], "plain": plain, "budgeted": budgeted})

report = {
    "queries": len(gold["queries"]),
    "identical_result_sets": not mismatches,
    "mismatches": mismatches,
    "note": (
        "A token budget selects per-result DETAIL. It must never change which "
        "results are returned, so recall at any cutoff is unchanged by "
        "construction. This check falsifies that claim rather than assuming it."
    ),
}
json.dump(report, open(f"{out_dir}/self-budget-non-inferiority.json", "w"), indent=2)
print(json.dumps({k: report[k] for k in ("queries", "identical_result_sets")}, indent=2))
if mismatches:
    sys.exit("budget changed result sets; the recall claim would be false")
BUDGETPY

echo "== reliability invariants =="
cargo test -p ast-sgrep-core --test snapshot_generation --test store_pragmas \
  2>&1 | tee "$out/reliability-tests.txt" | tail -5

python3 - "$out" "$commit" <<'PY'
import json, sys
out_dir, commit = sys.argv[1], sys.argv[2]
quality = json.load(open(f"{out_dir}/self-quality.json"))
ab = json.load(open(f"{out_dir}/self-ab-no-embed.json"))
tokens = json.load(open(f"{out_dir}/self-token-efficiency.json"))
summary = {
    "commit": commit,
    "dirty_worktree": False,
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
