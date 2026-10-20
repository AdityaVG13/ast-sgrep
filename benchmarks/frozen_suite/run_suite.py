"""Frozen RepoQA-600 / CoSQA-500 retrieval suite — shared harness for asgrep arms.

Materializes the frozen corpora/queries, runs a tool arm query-by-query in a
fresh CLI process, scores everything with one fixed scorer, and aggregates.
The arm drivers (`run_asgrep_arm.py`, `fusion_capture.py`) import this module
for Tools/retrieve/scoring.

Anti-cheat contract (frozen-suite discipline):
- Corpora and queries are byte-frozen before any scored run.
- No tuning of the tool against these datasets. The adapter uses default
  ranking with only --json and a fixed result cap (50 on RepoQA, 100 on
  CoSQA). Default flags otherwise.
- Every query counts: timeouts/errors stay in the denominator with rank=None.
- No retries, no outlier removal. Setup (indexing) is measured separately
  from query latency. Fresh CLI process per query.
- Raw stdout/stderr for every call is retained (records/) so any row can be
  re-scored without rerunning a tool.
- WARNING: retrieve() RESUMES from an existing retrieval-rows.jsonl in the
  bench dir and skips those queries without executing the tool. Reusing a
  bench dir for a new measurement without deleting retrieval-rows.jsonl
  (and records/) silently re-scores the previous run's rows. Fresh bench
  dir per arm, or delete both first.

Scoring:
- rank: 1-based position of the gold file among distinct result files
  (RepoQA), or of the gold snippet among distinct snippet files (CoSQA).
- hitK = fraction rank<=K; mrr10 = mean(1/rank for rank<=10).
- RepoQA coverage at token budget B in {2000, 8000}: walk results in order,
  union their source lines (overlaps counted once), charging o200k tokens
  per newly covered line, stopping at the first line that would exceed B.
  coverage = covered needle lines / total needle lines; half = >=0.5,
  full = 1.0.

Usage:
  run_suite.py --bench DIR prep        # download + materialize corpora/queries
  run_suite.py --bench DIR retrieve    # 600 RepoQA + 500 CoSQA, asgrep arm
  run_suite.py --bench DIR latency     # warm panel, 6 repeats (needs latency-panel.json in the bench dir)
  run_suite.py --bench DIR setup       # empty-index first use + storage
  run_suite.py --bench DIR report      # aggregate to summary.json/csv
  run_suite.py --bench DIR rescore     # re-score retained records/ rows

The binary comes from --asgrep or $ASGREP_BIN.
"""
import argparse
import csv
import gzip
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import time
import urllib.request

TIMEOUT_S = 180
METHODS = ("asgrep",)
REPOQA_N = 50
COSQA_N = 100
REPEATS = 6
UA = {"User-Agent": "OpenAI File Downloader, XaiImageApiFetch/1.0"}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load(p: Path):
    return json.loads(p.read_text())


def save(p: Path, obj):
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(obj, ensure_ascii=False, indent=1))


# ---------------------------------------------------------------- prep
def fetch(url: str, dest: Path):
    if dest.exists():
        return
    dest.parent.mkdir(parents=True, exist_ok=True)
    req = urllib.request.Request(url, headers=UA)
    with urllib.request.urlopen(req) as r, open(dest, "wb") as f:
        shutil.copyfileobj(r, f)


def prep(bench: Path):
    data = bench / "data"
    fetch("https://github.com/evalplus/repoqa/releases/download/dev-dataset/repoqa-2024-06-23.json",
          data / "repoqa-2024-06-23.json")
    coclr = data / "coclr"
    if not (coclr / "data/search/cosqa-retrieval-test-500.json").exists():
        coclr.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git", "init", "-q"], cwd=coclr, check=True)
        subprocess.run(["git", "remote", "add", "origin", "https://github.com/Jun-jie-Huang/CoCLR.git"],
                       cwd=coclr, check=True)
        subprocess.run(["git", "fetch", "-q", "--depth", "1", "origin",
                        "14ebcacf9e9bc3e7109102632bc63047876f27d2"], cwd=coclr, check=True)
        subprocess.run(["git", "checkout", "-q", "FETCH_HEAD"], cwd=coclr, check=True)

    corpus = bench / "corpus"
    d = load(data / "repoqa-2024-06-23.json")
    queries = []
    for lang, repos in d.items():
        for r in repos:
            scope = f"{lang}-" + r["repo"].replace("/", "__")
            root = corpus / "repoqa" / scope
            for rel, text in r["content"].items():
                p = root / rel
                if not p.exists():
                    p.parent.mkdir(parents=True, exist_ok=True)
                    p.write_text(text)
            for i, nd in enumerate(r["needles"]):
                qid = f"{scope}-{i:02d}"
                # Gold lines derive from the needle BYTE span: RepoQA's
                # start_line points at the blank line before the def, while
                # start_byte points at the def itself.
                raw = r["content"][nd["path"]].encode()
                g0 = raw[: nd["start_byte"]].count(b"\n") + 1
                g1 = raw[: nd["end_byte"]].count(b"\n") + 1
                if nd["end_byte"] > nd["start_byte"] and raw[nd["end_byte"] - 1 : nd["end_byte"]] == b"\n":
                    g1 -= 1
                queries.append(dict(id=qid, scope=scope, language=lang, query=nd["description"],
                                    path=nd["path"], start_line=g0, end_line=g1,
                                    repoqa_start_line=nd["start_line"], repoqa_end_line=nd["end_line"],
                                    name=nd["name"]))
    assert len(queries) == 600, len(queries)
    save(bench / "repoqa-queries.json", queries)

    cm = json.loads((coclr / "data/search/code_idx_map.txt").read_text())
    inv = {v: k for k, v in cm.items()}
    assert len(inv) == 6267
    man = {}
    for idx, code in inv.items():
        name = f"{idx:04d}.py"
        raw = code.encode()
        p = corpus / "cosqa" / name
        if not p.exists():
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(raw)
        man[name] = {"sha256": digest(raw), "snippet_idx": idx}
    save(bench / "cosqa-manifest.json", man)
    t = load(coclr / "data/search/cosqa-retrieval-test-500.json")
    cq = [dict(id=x["idx"], query=x["doc"], gold_idx=x["retrieval_idx"]) for x in t]
    assert len(cq) == 500, len(cq)
    save(bench / "cosqa-queries.json", cq)
    print("prep OK: RepoQA 600 queries; CoSQA 6267 snippets, 500 queries")


# ---------------------------------------------------------------- adapters
class Tools:
    def __init__(self, bench: Path, args):
        self.bench = bench
        self.asgrep = args.asgrep or os.environ.get("ASGREP_BIN")

    def argv(self, method: str, query: str, n: int):
        # --index-path pins the per-scope index (asgrep otherwise resolves an
        # ancestor index).
        return [self.asgrep, query, ".", "--json", "--limit", str(n),
                "--index-path", "./.asgrep/index.db"]

    def env(self, method: str):
        return dict(os.environ)

    def parse(self, method: str, stdout: bytes):
        """Return ordered [(relpath, start_line, end_line)] from native JSON."""
        out = json.loads(stdout.decode())
        hits = out["hits"] if isinstance(out, dict) else out
        rows = []
        for x in hits:
            path = x.get("path") or x.get("file") or x.get("file_path")
            lines = x.get("lines") if isinstance(x.get("lines"), dict) else {}
            start = x.get("start_line") or x.get("line_start") or x.get("line") or x.get("start") or lines.get("start")
            end = x.get("end_line") or x.get("line_end") or x.get("end") or lines.get("end") or start
            rows.append((str(path).removeprefix("./"), int(start), int(end)))
        return rows


def run_one(tools: Tools, method: str, query: str, root: Path, n: int):
    argv = tools.argv(method, query, n)
    wall, start = time.time(), time.perf_counter()
    try:
        p = subprocess.run(argv, cwd=root, env=tools.env(method), capture_output=True, timeout=TIMEOUT_S)
        stdout, stderr, code, status = p.stdout, p.stderr, p.returncode, "complete"
    except subprocess.TimeoutExpired as e:
        stdout, stderr, code, status = e.stdout or b"", e.stderr or b"", None, "timeout"
    secs = time.perf_counter() - start
    results = None
    if status == "complete":
        try:
            results = tools.parse(method, stdout)
        except Exception:
            status = "error"
    if code not in (0, 1) and status == "complete" and not results:
        status = "error"
    return dict(argv=argv, stdout=stdout, stderr=stderr, returncode=code,
                seconds=secs, clock_gap_s=(time.time() - wall) - secs,
                status=status, results=results)


def record(bench: Path, dataset: str, method: str, qid: str, rec: dict, suffix=""):
    p = bench / "records" / dataset / method / f"{qid}{suffix}.json.gz"
    p.parent.mkdir(parents=True, exist_ok=True)
    body = {k: (v.decode(errors="replace") if isinstance(v, bytes) else v)
            for k, v in rec.items() if k != "results"}
    body["results"] = rec["results"]
    p.write_bytes(gzip.compress(json.dumps(body, ensure_ascii=False).encode(), mtime=0))


# ---------------------------------------------------------------- scoring
class Scorer:
    def __init__(self, bench: Path):
        import tiktoken
        self.enc = tiktoken.get_encoding("o200k_base")
        self.bench = bench
        self._line_tokens = {}

    def file_lines(self, root: Path, rel: str):
        key = (str(root), rel)
        if key not in self._line_tokens:
            text = (root / rel).read_text(errors="replace")
            lines = text.split("\n")
            toks = [len(self.enc.encode(ln + "\n", disallowed_special=())) for ln in lines]
            self._line_tokens[key] = (lines, toks)
        return self._line_tokens[key]

    @staticmethod
    def file_rank(results, gold_path):
        seen = []
        for path, _, _ in results or []:
            if path not in seen:
                seen.append(path)
        return seen.index(gold_path) + 1 if gold_path in seen else None

    def coverage(self, results, root: Path, gold_path, g0, g1, budget):
        covered, tokens = set(), 0
        for path, s, e in results or []:
            try:
                _, toks = self.file_lines(root, path)
            except OSError:
                continue
            stop = False
            for ln in range(max(s, 1), min(e, len(toks)) + 1):
                if (path, ln) in covered:
                    continue
                cost = toks[ln - 1]
                if tokens + cost > budget:
                    stop = True
                    break
                tokens += cost
                covered.add((path, ln))
            if stop:
                break
        gold = {(gold_path, ln) for ln in range(g0, g1 + 1)}
        cov = len(covered & gold) / len(gold) if gold else 0.0
        return dict(half=cov >= 0.5, full=cov >= 1.0, coverage=cov, tokens=tokens)


def score_repoqa(scorer: Scorer, q, root: Path, rec):
    rank = scorer.file_rank(rec["results"], q["path"]) if rec["status"] == "complete" else None
    hits = {str(b): scorer.coverage(rec["results"], root, q["path"], q["start_line"], q["end_line"], b)
            if rec["status"] == "complete" else dict(half=False, full=False, coverage=0.0, tokens=0)
            for b in (2000, 8000)}
    return rank, hits


def score_cosqa(man, q, rec):
    if rec["status"] != "complete":
        return None
    seen = []
    for path, _, _ in rec["results"] or []:
        name = Path(path).name
        idx = man[name]["snippet_idx"] if name in man else None
        if idx is not None and idx not in seen:
            seen.append(idx)
    return seen.index(q["gold_idx"]) + 1 if q["gold_idx"] in seen else None


# ---------------------------------------------------------------- retrieve
def retrieve(bench: Path, tools: Tools, only=None, limit=None):
    scorer = Scorer(bench)
    rows = []
    rq = load(bench / "repoqa-queries.json")
    cq = load(bench / "cosqa-queries.json")
    man = load(bench / "cosqa-manifest.json")
    if limit:
        rq, cq = rq[:limit], cq[:limit]
    out_path = bench / "retrieval-rows.jsonl"
    done = set()
    if out_path.exists():
        for line in out_path.read_text().splitlines():
            r = json.loads(line)
            done.add((r["dataset"], r["method"], r["id"]))
            rows.append(r)
    jobs = []
    for q in rq:
        jobs.append(("repoqa", q, bench / "corpus/repoqa" / q["scope"], REPOQA_N))
    for q in cq:
        jobs.append(("cosqa", q, bench / "corpus/cosqa", COSQA_N))
    # Unmeasured warmup: one query per scope per method so timed one-pass
    # queries run on warm indexes/page cache (their warmups were separate).
    seen_scopes = set()
    for method in METHODS:
        for dataset, q, root, n in jobs:
            key = (method, str(root))
            if key in seen_scopes:
                continue
            seen_scopes.add(key)
            run_one(tools, method, q["query"], root, n)
    with open(out_path, "a") as f:
        for method in METHODS:
            for dataset, q, root, n in jobs:
                if only and q["id"] not in only:
                    continue
                if (dataset, method, q["id"]) in done:
                    continue
                rec = run_one(tools, method, q["query"], root, n)
                record(bench, dataset, method, q["id"], rec)
                if dataset == "repoqa":
                    rank, hits = score_repoqa(scorer, q, root, rec)
                    row = dict(dataset=dataset, method=method, id=q["id"], seconds=rec["seconds"],
                               status=rec["status"], rank=rank, language=q["language"], hits=hits)
                else:
                    row = dict(dataset=dataset, method=method, id=q["id"], seconds=rec["seconds"],
                               status=rec["status"], rank=score_cosqa(man, q, rec))
                f.write(json.dumps(row) + "\n")
                f.flush()
                rows.append(row)
                print(dataset, method, q["id"], row["status"], "rank", row["rank"],
                      f"{rec['seconds']*1000:.0f}ms", flush=True)
    return rows


# ---------------------------------------------------------------- latency
def latency(bench: Path, tools: Tools):
    panel = load(bench / "latency-panel.json")
    rq = {q["id"]: q for q in load(bench / "repoqa-queries.json")}
    cq = {q["id"]: q for q in load(bench / "cosqa-queries.json")}
    cases = []
    for c in panel:
        q = rq[c["id"]] if c["dataset"] == "repoqa" else cq[c["id"]]
        root = (bench / "corpus/repoqa" / q["scope"]) if c["dataset"] == "repoqa" else (bench / "corpus/cosqa")
        n = REPOQA_N if c["dataset"] == "repoqa" else COSQA_N
        cases.append(dict(dataset=c["dataset"], id=c["id"], query=q["query"], root=root, n=n))
    rows, baseline = [], {}
    for repeat in range(-1, REPEATS):
        order = {ds: sorted([c for c in cases if c["dataset"] == ds],
                            key=lambda c: digest(f"{repeat}/{c['id']}".encode()))
                 for ds in ("repoqa", "cosqa")}
        ordered = [c for pair in zip(order["repoqa"], order["cosqa"]) for c in pair]
        for i, c in enumerate(ordered):
            shift = (i + repeat + 1) % len(METHODS)
            for method in METHODS[shift:] + METHODS[:shift]:
                rec = run_one(tools, method, c["query"], c["root"], c["n"])
                record(bench, "latency-" + c["dataset"], method, c["id"], rec, suffix=f".{repeat}")
                sig = digest(json.dumps(rec["results"] or []).encode())
                key = (c["dataset"], c["id"], method)
                if repeat == -1:
                    baseline[key] = sig
                rows.append(dict(dataset=c["dataset"], id=c["id"], method=method, repeat=repeat,
                                 phase="prepare" if repeat < 0 else "warm",
                                 ms=rec["seconds"] * 1000, clock_gap_ms=rec["clock_gap_s"] * 1000,
                                 load1=os.getloadavg()[0], returncode=rec["returncode"],
                                 status=rec["status"], same_ranked_source=(sig == baseline.get(key))))
                with open(bench / "latency-rows.jsonl", "a") as f:
                    f.write(json.dumps(rows[-1]) + "\n")
        print("latency repeat", repeat, "done", flush=True)
    save(bench / "latency-rows.json", rows)


# ---------------------------------------------------------------- setup
def du_bytes(p: Path) -> int:
    return sum(f.stat().st_size for f in p.rglob("*") if f.is_file()) if p.exists() else 0


def setup(bench: Path, tools: Tools):
    rq = load(bench / "repoqa-queries.json")
    cq = load(bench / "cosqa-queries.json")
    panel = {(c["dataset"], c["id"]) for c in load(bench / "latency-panel.json")}
    first_rq = {}
    for q in rq:
        if ("repoqa", q["id"]) in panel:
            first_rq[q["scope"]] = q
    assert len(first_rq) == 60
    first_cq = next(q for q in cq if ("cosqa", q["id"]) in panel)
    out = {"repoqa": {}, "cosqa": {}}

    # asgrep: explicit index build per scope from no index, then first query.
    times, qtimes, storage = [], [], {}
    for scope, q in sorted(first_rq.items()):
        root = bench / "corpus/repoqa" / scope
        idx = root / ".asgrep"
        if idx.exists():
            shutil.rmtree(idx)
        t0 = time.perf_counter()
        p = subprocess.run([tools.asgrep, "index", ".", "--index-path", "./.asgrep/index.db"],
                           cwd=root, capture_output=True, timeout=1800)
        it = time.perf_counter() - t0
        assert p.returncode == 0, (scope, p.stderr.decode(errors="replace")[-500:])
        rec = run_one(tools, "asgrep", q["query"], root, REPOQA_N)
        times.append(it * 1000)
        qtimes.append(rec["seconds"] * 1000)
        storage[scope] = du_bytes(idx)
        print("setup asgrep repoqa", scope, f"index {it*1000:.0f}ms first-q {qtimes[-1]:.0f}ms", flush=True)
    root = bench / "corpus/cosqa"
    idx = root / ".asgrep"
    if idx.exists():
        shutil.rmtree(idx)
    t0 = time.perf_counter()
    p = subprocess.run([tools.asgrep, "index", ".", "--index-path", "./.asgrep/index.db"],
                       cwd=root, capture_output=True, timeout=1800)
    cit = time.perf_counter() - t0
    assert p.returncode == 0, p.stderr.decode(errors="replace")[-500:]
    rec = run_one(tools, "asgrep", first_cq["query"], root, COSQA_N)
    st = sorted(times)
    out["repoqa"]["asgrep"] = dict(scopes=60, median_ms=statistics.median(times),
                                   p95_ms=st[math.ceil(.95 * len(st)) - 1],
                                   total_seconds=sum(times) / 1000,
                                   first_query_median_ms=statistics.median(qtimes),
                                   meaning="Explicit `asgrep index .` from no index (excl. first query); "
                                           "index+first-query = median_ms+first_query_median_ms approx")
    out["cosqa"]["asgrep"] = dict(scopes=1, median_ms=cit * 1000, p95_ms=cit * 1000,
                                  total_seconds=cit, first_query_median_ms=rec["seconds"] * 1000,
                                  meaning="Explicit `asgrep index .` from no index (excl. first query)")
    out["storage_bytes"] = {}
    out["storage_bytes"]["asgrep_repoqa_total"] = sum(storage.values())
    out["storage_bytes"]["asgrep_cosqa"] = du_bytes(idx)
    out["storage_bytes"]["asgrep_both_datasets"] = sum(storage.values()) + du_bytes(idx)
    out["limits"] = ("asgrep setup is the explicit index command, first query reported separately.")
    save(bench / "setup-summary.json", out)
    print(json.dumps(out, indent=1)[:1200])


# ---------------------------------------------------------------- report
def aggregate(group):
    n = len(group)
    times = sorted(r["seconds"] * 1000 for r in group)
    out = dict(n=n, errors=sum(r["status"] == "error" for r in group),
               timeouts=sum(r["status"] == "timeout" for r in group),
               p50_ms=statistics.median(times),
               p95_ms=times[math.ceil(.95 * n) - 1], p99_ms=times[math.ceil(.99 * n) - 1])
    for k in (1, 5, 10, 100):
        out[f"hit{k}"] = sum(r["rank"] is not None and r["rank"] <= k for r in group) / n
    out["mrr10"] = sum(1 / r["rank"] for r in group if r["rank"] is not None and r["rank"] <= 10) / n
    if group and group[0]["dataset"] == "repoqa":
        for metric, budget in (("half", 2000), ("half", 8000), ("full", 8000)):
            out[f"{metric}{budget}"] = sum(r["hits"][str(budget)][metric] for r in group) / n
    return out


def report(bench: Path):
    rows = [json.loads(l) for l in (bench / "retrieval-rows.jsonl").read_text().splitlines() if l.strip()]
    summary = {}
    for ds in ("repoqa", "cosqa"):
        summary[ds] = {}
        for m in METHODS:
            g = [r for r in rows if r["dataset"] == ds and r["method"] == m]
            if g:
                summary[ds][m] = aggregate(g)
    by_lang = {}
    for lang in sorted({r.get("language") for r in rows if r.get("language")}):
        by_lang[lang] = {}
        for m in METHODS:
            g = [r for r in rows if r.get("language") == lang and r["method"] == m]
            if g:
                by_lang[lang][m] = aggregate(g)
    lat = {}
    lp = bench / "latency-rows.json"
    if lp.exists():
        lrows = load(lp)
        warm = [r for r in lrows if r["phase"] == "warm"]
        for ds in ("repoqa", "cosqa"):
            lat[ds] = {}
            for m in METHODS:
                g = [r for r in warm if r["dataset"] == ds and r["method"] == m]
                if not g:
                    continue
                times = sorted(r["ms"] for r in g)
                ids = sorted({r["id"] for r in g})
                medians = {i: statistics.median(r["ms"] for r in g if r["id"] == i) for i in ids}
                lat[ds][m] = dict(calls=len(g), queries=len(ids), p50_ms=statistics.median(times),
                                  p95_ms=times[math.ceil(.95 * len(times)) - 1],
                                  median_query_ms=statistics.median(medians.values()),
                                  query_medians=medians,
                                  ranked_source_mismatches=sum(not r["same_ranked_source"] for r in g),
                                  errors=sum(r["status"] != "complete" for r in g))
    meta = dict(platform=platform.platform(), machine=platform.machine())
    p = os.environ.get("ASGREP_BIN")
    if p and Path(p).exists():
        meta["asgrep"] = dict(path=p, sha256=digest(Path(p).read_bytes()))
    out = dict(summary=summary, by_language=by_lang,
               latency=lat, meta=meta)
    save(bench / "summary.json", out)
    with open(bench / "summary.csv", "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["dataset", "method", "n", "hit1", "hit5", "hit10", "hit100", "mrr10",
                    "half2000", "half8000", "full8000", "p50_ms", "p95_ms", "p99_ms", "errors", "timeouts"])
        for ds, ms in summary.items():
            for m, s in ms.items():
                w.writerow([ds, m, s["n"], s["hit1"], s["hit5"], s["hit10"], s["hit100"], s["mrr10"],
                            s.get("half2000", ""), s.get("half8000", ""), s.get("full8000", ""),
                            s["p50_ms"], s["p95_ms"], s["p99_ms"], s["errors"], s["timeouts"]])
    print(json.dumps(summary, indent=1))


# ---------------------------------------------------------------- rescore
def rescore(bench: Path):
    """Rebuild retrieval-rows.jsonl from retained records/ (no tool reruns).

    Seconds/status are preserved from the original run; only rank/coverage
    are recomputed. Exists so scorer calibration fixes never require
    re-executing a tool (which would break the frozen one-pass timing)."""
    scorer = Scorer(bench)
    rq = {q["id"]: q for q in load(bench / "repoqa-queries.json")}
    cq = {q["id"]: q for q in load(bench / "cosqa-queries.json")}
    man = load(bench / "cosqa-manifest.json")
    rows = []
    for dataset, queries in (("repoqa", rq), ("cosqa", cq)):
        for method in METHODS:
            d = bench / "records" / dataset / method
            if not d.exists():
                continue
            for p in sorted(d.glob("*.json.gz")):
                qid = p.name[:-len(".json.gz")]
                rec = json.loads(gzip.decompress(p.read_bytes()))
                rec["results"] = [tuple(x) for x in rec["results"]] if rec.get("results") else None
                q = queries[qid]
                if dataset == "repoqa":
                    root = bench / "corpus/repoqa" / q["scope"]
                    rank, hits = score_repoqa(scorer, q, root, rec)
                    rows.append(dict(dataset=dataset, method=method, id=qid, seconds=rec["seconds"],
                                     status=rec["status"], rank=rank, language=q["language"], hits=hits))
                else:
                    rows.append(dict(dataset=dataset, method=method, id=qid, seconds=rec["seconds"],
                                     status=rec["status"], rank=score_cosqa(man, q, rec)))
    with open(bench / "retrieval-rows.jsonl", "w") as f:
        for r in rows:
            f.write(json.dumps(r) + "\n")
    print("rescored", len(rows), "rows from records/")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--bench", type=Path, required=True)
    ap.add_argument("--asgrep")
    ap.add_argument("--limit", type=int, help="smoke: first N queries per dataset")
    ap.add_argument("--only", help="comma-separated query ids (smoke)")
    ap.add_argument("phase", choices=("prep", "retrieve", "latency", "setup", "report", "rescore"))
    args = ap.parse_args()
    bench = args.bench.resolve()
    bench.mkdir(parents=True, exist_ok=True)
    if args.phase == "prep":
        prep(bench)
    elif args.phase == "report":
        report(bench)
    elif args.phase == "rescore":
        rescore(bench)
    else:
        tools = Tools(bench, args)
        if args.phase == "retrieve":
            retrieve(bench, tools, only=set(args.only.split(",")) if args.only else None, limit=args.limit)
        elif args.phase == "latency":
            latency(bench, tools)
        else:
            setup(bench, tools)


if __name__ == "__main__":
    main()
