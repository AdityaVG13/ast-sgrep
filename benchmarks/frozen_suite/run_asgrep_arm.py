"""Run one asgrep arm of the frozen suite end to end.

Reindexes every corpus repo with the given binary (default `.asgrep`
discovery inside each repo -- never `--index-path`, whose parent-walk
discovery can silently search a different index than the one just
written), asserts the stored embed backend from index meta, then runs
the frozen queries through run_suite.retrieve/aggregate.

The caller's environment is passed through untouched: set
ASGREP_EMBED_BACKEND to pin a backend, or leave it unset to measure
the shipped default. The stored backend is read back from meta, never
inferred from the request.
"""
import argparse, json, shutil, sqlite3, subprocess, sys, time
from pathlib import Path

SUITE = Path(__file__).resolve().parent
sys.path.insert(0, str(SUITE))


def stored_backend(db: Path) -> str:
    con = sqlite3.connect(db)
    try:
        row = con.execute(
            "SELECT value FROM meta WHERE key='embed_backend'").fetchone()
        return row[0] if row else "<none>"
    finally:
        con.close()


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", required=True)
    ap.add_argument("--bench-dir", required=True,
                    help="prepared suite dir (corpus/ + query files)")
    ap.add_argument("--out-dir", required=True)
    ap.add_argument("--expect-backend", default=None,
                    help="assert the stored embed_backend equals this")
    args = ap.parse_args()

    bench = Path(args.bench_dir)
    out = Path(args.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    link = out / "corpus"
    if not link.exists():
        link.symlink_to(bench / "corpus")
    for f in ("repoqa-queries.json", "cosqa-queries.json",
              "cosqa-manifest.json"):
        if not (out / f).exists():
            shutil.copy(bench / f, out / f)

    repos = sorted(p for p in (bench / "corpus/repoqa").iterdir()
                   if p.is_dir())
    repos.append(bench / "corpus/cosqa")
    times = {}
    for repo in repos:
        t0 = time.time()
        subprocess.run([args.bin, "index", "--json"], cwd=repo,
                       capture_output=True, text=True, check=True)
        times[str(repo)] = round(time.time() - t0, 1)
        db = repo / ".asgrep/index.db"
        assert db.exists(), f"no .asgrep index written in {repo}"

    backend = stored_backend(repos[0] / ".asgrep/index.db")
    if args.expect_backend:
        assert backend == args.expect_backend, \
            f"stored backend {backend!r} != expected {args.expect_backend!r}"

    import run_suite as rs
    rs.METHODS = ("asgrep",)
    tools = rs.Tools(out, argparse.Namespace(asgrep=args.bin))
    rows = rs.retrieve(out, tools)

    summary = {}
    for ds in ("repoqa", "cosqa"):
        g = [r for r in rows if r["dataset"] == ds]
        summary[ds] = rs.aggregate(g)
        by_lang = {}
        for lang in sorted({r.get("language") for r in g if r.get("language")}):
            by_lang[lang] = rs.aggregate(
                [r for r in g if r.get("language") == lang])
        if by_lang:
            summary[ds]["by_language"] = by_lang
    result = dict(stored_backend=backend,
                  index_total_seconds=sum(times.values()), summary=summary)
    (out / "summary.json").write_text(json.dumps(result, indent=1))
    print(json.dumps(result, indent=1))


if __name__ == "__main__":
    main()
