#!/usr/bin/env python3
"""Holdout arm driver: index the phase-3 holdout corpus and score asgrep.

Mirrors benchmarks/sgrep_suite/run_asgrep_arm.py but reads the holdout
bench layout (corpus/holdout/<scope> + holdout-queries.json) and scores
with the frozen RepoQA scorer (file rank + line coverage).

Usage:
  python3 benchmarks/phase3_holdout/run_holdout_arm.py \
    --bin target/release/asgrep \
    --bench /path/to/holdout-v1 \
    --out-dir /path/to/holdout-out
"""
import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

SUITE = Path(__file__).resolve().parent.parent / "sgrep_suite"
sys.path.insert(0, str(SUITE))


def main():
    args = parse()
    bench = Path(args.bench)
    out = Path(args.out_dir)
    out.mkdir(parents=True, exist_ok=True)

    queries = json.loads((bench / "holdout-queries.json").read_text())
    scopes = sorted({q["scope"] for q in queries})
    repos = [bench / "corpus/holdout" / s for s in scopes]
    assert all(r.is_dir() for r in repos), "missing scope dirs"

    times = {}
    for repo in repos:
        t0 = time.time()
        subprocess.run([args.bin, "index", "--json"], cwd=repo,
                       capture_output=True, text=True, check=True)
        times[str(repo)] = round(time.time() - t0, 1)
        assert (repo / ".asgrep/index.db").exists(), f"no index in {repo}"

    import run_suite as rs
    rs.METHODS = ("asgrep",)
    tools = rs.Tools(out, argparse.Namespace(asgrep=args.bin, sgrep=None))
    scorer = rs.Scorer(out)

    seen, rows = set(), []
    for q in queries:  # unmeasured warmup, one per scope
        root = bench / "corpus/holdout" / q["scope"]
        if str(root) not in seen:
            seen.add(str(root))
            rs.run_one(tools, "asgrep", q["query"], root, 100)
    for q in queries:
        root = bench / "corpus/holdout" / q["scope"]
        rec = rs.run_one(tools, "asgrep", q["query"], root, 100)
        rs.record(out, "holdout", "asgrep", q["id"], rec)
        rank, hits = rs.score_repoqa(scorer, q, root, rec)
        rows.append(dict(dataset="repoqa", id=q["id"], status=rec["status"],
                         rank=rank, language=q["language"],
                         hits={str(k): v for k, v in hits.items()},
                         seconds=rec["seconds"]))
        print("holdout", q["id"], rec["status"], "rank", rank, flush=True)

    summary = rs.aggregate(rows)
    by_lang = {l: rs.aggregate([r for r in rows if r.get("language") == l])
               for l in sorted({r.get("language") for r in rows})}
    result = dict(n=len(rows), index_total_seconds=round(sum(times.values()), 1),
                  summary=summary, by_language=by_lang)
    (out / "summary.json").write_text(json.dumps(result, indent=1))
    print(json.dumps(result, indent=1))


def parse():
    p = argparse.ArgumentParser()
    p.add_argument("--bin", required=True)
    p.add_argument("--bench", required=True)
    p.add_argument("--out-dir", required=True)
    return p.parse_args()


if __name__ == "__main__":
    main()
