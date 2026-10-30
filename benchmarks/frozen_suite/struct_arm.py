"""Run the frozen structural corpus through asgrep; rank = first returned
hit whose file is in gold_files (file-level, repoqa convention)."""
import argparse, json, sys
from pathlib import Path

SUITE = Path("/Users/aditya/Developer/ast-sgrep/benchmarks/frozen_suite")
sys.path.insert(0, str(SUITE))
import run_suite as rs

BENCH = Path("~/.hermes/cache/scratch/asgrep-frozen-20261028").expanduser()
OUT = BENCH / "out-struct"
OUT.mkdir(exist_ok=True)
tools = rs.Tools(OUT, argparse.Namespace(asgrep="/Users/aditya/Developer/ast-sgrep/target/release/asgrep"))
qs = json.loads((BENCH / "struct-queries.json").read_text())
rows_path = OUT / "retrieval-rows.jsonl"

def parse_hits(rec):
    out = rec["results"]
    hits = out["hits"] if isinstance(out, dict) else out
    return hits

with open(rows_path, "w") as rf:
    for q in qs:
        root = BENCH / "corpus/repoqa" / q["scope"]
        rec = rs.run_one(tools, "asgrep", q["query"], root, 50)
        rank = None
        if rec["status"] == "complete":
            for i, h in enumerate(parse_hits(rec)):
                f = h[0] if isinstance(h, (list, tuple)) else (h.get("file") or h.get("path"))
                if f in q["gold"] or any(f.endswith("/" + g) for g in q["gold"]):
                    rank = i + 1
                    break
        rf.write(json.dumps(dict(id=q["id"], type=q["type"], scope=q["scope"],
                                 symbol=q["symbol"], status=rec["status"], rank=rank,
                                 seconds=rec["seconds"])) + "\n")
        rf.flush()

rows = [json.loads(l) for l in open(rows_path)]
for t in ("callers", "defs", "imports"):
    sub = [r for r in rows if r["type"] == t]
    mrr = sum(1.0 / r["rank"] for r in sub if r["rank"] and r["rank"] <= 10) / len(sub)
    hit1 = sum(1 for r in sub if r["rank"] == 1) / len(sub)
    hit10 = sum(1 for r in sub if r["rank"] and r["rank"] <= 10) / len(sub)
    miss = sum(1 for r in sub if not r["rank"]) / len(sub)
    print(f"{t:8s} n={len(sub):3d} MRR@10={mrr:.4f} hit@1={hit1:.4f} hit@10={hit10:.4f} miss@50={miss:.4f}")
tot = len(rows)
print(f"ALL      n={tot} MRR@10={sum(1.0/r['rank'] for r in rows if r['rank'] and r['rank']<=10)/tot:.4f}")
