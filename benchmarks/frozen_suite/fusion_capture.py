"""Capture pre-fusion channel ranks for the frozen suite (loss attribution).

Runs each frozen query once through the given asgrep binary with
ASGREP_FUSION_DUMP pointed at a fresh file, recording per-query byte spans so
dump lines join to queries exactly (a query that never reaches fusion leaves
a zero-length span). Ranks are computed with run_suite's own scorers, so rows
are comparable with arm runs.

Outputs in --out-dir:
  rows.jsonl         per-query rank rows (+ dump span, + query text)
  fusion-dump.jsonl  raw capture, one JSON line per fusion invocation
  attribution.json   miss classification per dataset (capture runs only)

Control/parity: pass --control-rows <prior-arm>/retrieval-rows.jsonl to diff
ranks against a recorded arm, and --no-capture to run the same queries with
the dump env unset (proves the capture does not perturb ranking).
"""

import argparse
import json
import os
import sys
from pathlib import Path

SUITE = Path(__file__).resolve().parent
sys.path.insert(0, str(SUITE))

import run_suite as rs  # noqa: E402

CHANNELS = ("lexical", "definition", "caller", "graph",
            "anchor", "semantic", "pattern", "import")


def dump_lines_between(dump: Path, lo: int, hi: int):
    if hi <= lo:
        return []
    with open(dump, "rb") as f:
        f.seek(lo)
        text = f.read(hi - lo).decode()
    return [json.loads(line) for line in text.splitlines() if line.strip()]


def gold_file_name(dataset, q, man_by_idx):
    if dataset == "repoqa":
        return Path(q["path"]).name
    return man_by_idx.get(q["gold_idx"])


def gold_rows_for(dataset, q, gold_name, results):
    """Dump rows belonging to the gold file. RepoQA joins on the full
    relative path (basename collisions like `utils.py` would otherwise
    misjoin); CoSQA's flat corpus keys by basename via the manifest."""
    if dataset == "repoqa":
        gp = q["path"]
        exact = [r for r in results if r["file"] == gp or r["file"].endswith("/" + gp)]
        if exact:
            return exact
        by_base = [r for r in results if Path(r["file"]).name == gold_name]
        return by_base if len({r["file"] for r in by_base}) == 1 else []
    return [r for r in results if Path(r["file"]).name == gold_name]


def best_channel_ranks(rows):
    """Best (lowest) per-channel rank over the gold file's dump rows."""
    if not rows:
        return None, []
    per_channel = {}
    for r in rows:
        for ch in CHANNELS:
            v = (r["ranks"] or {}).get(ch)
            if v is not None:
                per_channel[ch] = min(per_channel.get(ch, v), v)
    return (min(per_channel.values()) if per_channel else None), [
        {"lines": [r["line_start"], r["line_end"]],
         "ranks": {ch: (r["ranks"] or {}).get(ch) for ch in CHANNELS if (r["ranks"] or {}).get(ch) is not None},
         "kinds": r["kinds"], "score": r["score"]}
        for r in rows[:3]
    ]


def classify_miss(rank, span_lines, gold_rows, best_rank, miss_cut):
    if not span_lines:
        return "no_fusion_call"
    if gold_rows is None:
        return "upstream_admission"
    if best_rank is not None and best_rank <= 3:
        return "fusion_loss"
    return "channel_scoring_loss"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", required=True)
    ap.add_argument("--bench-dir", required=True)
    ap.add_argument("--out-dir", required=True)
    ap.add_argument("--datasets", default="cosqa,repoqa")
    ap.add_argument("--limit", type=int, default=None)
    ap.add_argument("--control-rows", default=None)
    ap.add_argument("--no-capture", action="store_true")
    ap.add_argument("--analyze-only", action="store_true",
                    help="skip queries; re-analyze existing out-dir rows+dump")
    ap.add_argument("--miss-cut", type=int, default=10)
    args = ap.parse_args()

    bench, out = Path(args.bench_dir), Path(args.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    dump = out / "fusion-dump.jsonl"
    rows_path = out / "rows.jsonl"

    man = rs.load(bench / "cosqa-manifest.json")
    man_by_idx = {v["snippet_idx"]: k for k, v in man.items()}

    if not args.analyze_only:
        dump.unlink(missing_ok=True)
        rows_path.unlink(missing_ok=True)
        tools = rs.Tools(out, argparse.Namespace(asgrep=args.bin))
        scorer = rs.Scorer(bench)
        capture = not args.no_capture
        if capture:
            os.environ["ASGREP_FUSION_DUMP"] = str(dump)
        else:
            os.environ.pop("ASGREP_FUSION_DUMP", None)

        jobs = []
        for ds in args.datasets.split(","):
            qs = rs.load(bench / f"{ds}-queries.json")
            if args.limit:
                qs = qs[:args.limit]
            for q in qs:
                root = (bench / "corpus/repoqa" / q["scope"]) if ds == "repoqa" else (bench / "corpus/cosqa")
                n = rs.REPOQA_N if ds == "repoqa" else rs.COSQA_N
                jobs.append((ds, q, root, n))

        with open(rows_path, "w") as rf:
            for ds, q, root, n in jobs:
                before = dump.stat().st_size if dump.exists() else 0
                rec = rs.run_one(tools, "asgrep", q["query"], root, n)
                after = dump.stat().st_size if dump.exists() else 0
                if ds == "repoqa":
                    rank, _hits = rs.score_repoqa(scorer, q, root, rec)
                else:
                    rank = rs.score_cosqa(man, q, rec)
                row = dict(dataset=ds, id=q["id"], status=rec["status"], rank=rank,
                           dump_span=[before, after], query=q["query"],
                           gold_file=gold_file_name(ds, q, man_by_idx))
                rf.write(json.dumps(row) + "\n")
                rf.flush()
                print(ds, q["id"], rec["status"], "rank", rank, flush=True)
        os.environ.pop("ASGREP_FUSION_DUMP", None)

    # ---- parity vs a prior arm -------------------------------------------
    if args.control_rows:
        control = {}
        with open(args.control_rows) as f:
            for line in f:
                r = json.loads(line)
                control[(r["dataset"], r["id"])] = r["rank"]
        mismatch = []
        with open(rows_path) as f:
            for line in f:
                r = json.loads(line)
                key = (r["dataset"], r["id"])
                if key in control and control[key] != r["rank"]:
                    mismatch.append((key, control[key], r["rank"]))
        print(f"parity vs {args.control_rows}: {len(mismatch)} rank mismatches "
              f"of {sum(1 for l in open(rows_path))} rows")
        for m in mismatch[:10]:
            print("  MISMATCH", m)
        if mismatch:
            sys.exit(3)

    # ---- attribution ------------------------------------------------------
    rows = [json.loads(l) for l in open(rows_path) if l.strip()]
    qmap = {}
    for f in ("repoqa-queries.json", "cosqa-queries.json"):
        for q in rs.load(bench / f):
            qmap[q["id"]] = q
    report = {}
    for ds in sorted({r["dataset"] for r in rows}):
        ds_rows = [r for r in rows if r["dataset"] == ds]
        classes = {}
        examples = []
        for r in ds_rows:
            rank = r["rank"]
            missed = rank is None or (rank is not None and rank > args.miss_cut)
            if not missed:
                continue
            span_lines = dump_lines_between(dump, *r["dump_span"]) if not args.no_capture else []
            best, gold_rows = (None, None)
            if span_lines and r.get("gold_file"):
                merged = [row for line in span_lines for row in line.get("results", [])]
                gold_rows = gold_rows_for(ds, qmap.get(r["id"], {}), r["gold_file"], merged)
                best, gold_rows = best_channel_ranks(gold_rows)
            cls = classify_miss(rank, span_lines, gold_rows, best, args.miss_cut)
            classes[cls] = classes.get(cls, 0) + 1
            if len(examples) < 12:
                examples.append(dict(id=r["id"], rank=rank, cls=cls,
                                     best_channel_rank=best,
                                     gold=gold_rows if gold_rows else None,
                                     input_hits=span_lines[0]["input_hits"] if span_lines else 0))
        total = len(ds_rows)
        misses = sum(classes.values())
        report[ds] = dict(rows=total, miss_cut=args.miss_cut, misses=misses,
                          classes=classes, examples=examples)
        print(f"[{ds}] {misses}/{total} queries miss rank<={args.miss_cut}: {classes}")
    (out / "attribution.json").write_text(json.dumps(report, indent=1))
    print("attribution written:", out / "attribution.json")


if __name__ == "__main__":
    main()
