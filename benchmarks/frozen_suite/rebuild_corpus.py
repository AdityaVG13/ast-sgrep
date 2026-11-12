#!/usr/bin/env python3
"""Rebuild the RepoQA-600 corpus from public GitHub repos (shallow, HEAD).

The frozen 2026-11-02 corpus bytes are gone (external dir deleted); no
(repo, sha) pin survives. This rebuilds an EQUIVALENT corpus: same 60
repos, same scope layout, fresh HEAD. Absolute numbers will NOT match the
frozen baselines — label every downstream row `rebuilt-HEAD`. Paired arms
(control vs switch on the same rebuilt bytes) remain valid causal evidence.

Usage:
  python3 rebuild_corpus.py --root /tmp/astsgrep-corpus-20261109 [--scopes a,b]

Provenance: writes root/corpus-provenance.json with per-scope HEAD shas.
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

SUITE = Path(__file__).resolve().parent
FROZEN = Path("/Users/aditya/Developer/ast-sgrep/benchmarks/results/internal/frozen-20261102")


def scope_repo(scope: str) -> tuple[str, str]:
    body = scope.split("-", 1)[1]  # strip lang prefix
    owner, repo = body.split("__", 1)
    return owner, repo


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", required=True)
    ap.add_argument("--scopes", default=None)
    args = ap.parse_args()

    rq = json.loads((FROZEN / "repoqa-queries.json").read_text())
    scopes = sorted({q["scope"] for q in rq})
    if args.scopes:
        want = set(args.scopes.split(","))
        scopes = [s for s in scopes if s in want]

    dest = Path(args.root) / "corpus" / "repoqa"
    dest.mkdir(parents=True, exist_ok=True)
    prov: dict[str, dict] = {}
    failures = []
    for scope in scopes:
        owner, repo = scope_repo(scope)
        target = dest / scope
        if target.is_dir() and any(target.iterdir()):
            print(f"SKIP {scope} (exists)", flush=True)
        else:
            target.mkdir(parents=True, exist_ok=True)
            url = f"https://github.com/{owner}/{repo}.git"
            print(f"CLONE {scope} <- {url}", flush=True)
            p = subprocess.run(
                ["git", "clone", "--depth", "1", "--quiet", url, str(target)],
                capture_output=True, text=True, timeout=1200)
            if p.returncode != 0:
                failures.append(scope)
                print(f"FAIL {scope}: {p.stderr[-300:]}", flush=True)
                continue
        q = subprocess.run(["git", "rev-parse", "HEAD"], cwd=target,
                           capture_output=True, text=True)
        prov[scope] = {"url": f"https://github.com/{owner}/{repo}.git",
                       "head": q.stdout.strip()}
    (Path(args.root) / "corpus-provenance.json").write_text(
        json.dumps(prov, indent=1))
    print(f"done: {len(prov)} ok, {len(failures)} failed")
    if failures:
        print("failures:", failures)
        sys.exit(1)


if __name__ == "__main__":
    main()
