"""Phase 3 holdout builder: fetch pinned corpora, extract candidates, gate queries.

Usage:
  python3 build_holdout.py --bench DIR fetch       # clone 12 pinned repos
  python3 build_holdout.py --bench DIR candidates  # extract (target, test) pairs
  python3 build_holdout.py --bench DIR assemble    # validate queries + write bench files
  python3 build_holdout.py --bench DIR check       # gates only (schema, 6-gram, frozen disjoint)

Queries are authored by hand in queries-authored.json (see spec
docs/plans/phase3-holdout.md). This script never writes queries, only gates.
Assembled bench layout mirrors the frozen suite (corpus/holdout/<scope>/ +
holdout-queries.json in the frozen schema + provenance sidecar), so
run_asgrep_arm.py scores it with a one-line scope-path tweak.
"""
import argparse
import hashlib
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
UA = "OpenAI File Downloader, XaiImageApiFetch/1.0"

# scope, repo, sha (full), language, license marker expected in LICENSE file
PINS = [
    ("python-psf__requests", "psf/requests", "611c6162cbc4ac2020a2f91c7cfa4f3abf9bbb60", "python", "Apache"),
    ("python-encode__httpx", "encode/httpx", "b5addb64f0161ff6bfe94c124ef76f6a1fba5254", "python", "BSD"),
    ("cpp-fmtlib__fmt", "fmtlib/fmt", "fdf31252e61ca7efe99af41bb4780353cc2253dd", "cpp", "MIT"),
    ("cpp-gabime__spdlog", "gabime/spdlog", "76d458055b006fd8b86675b67ad51bccc66274c7", "cpp", "MIT"),
    ("java-mockito__mockito", "mockito/mockito", "613ab039480c8dbf1fe0fe4c0c91022e81cd7b75", "java", "MIT"),
    ("java-ben-manes__caffeine", "ben-manes/caffeine", "e0cc61784b2dc4c23867a29fe0c79d80b7528c54", "java", "Apache"),
    ("typescript-colinhacks__zod", "colinhacks/zod", "680d19f7d1e042e55934209caeb79bc843b94f44", "typescript", "MIT"),
    ("typescript-tj__commander.js", "tj/commander.js", "ba6d13ddb4243e5913367734f8c159089ffe7834", "typescript", "MIT"),
    ("rust-clap-rs__clap", "clap-rs/clap", "ccc89b83875474399e9d9d70841163f6319d1a41", "rust", "MIT"),
    ("rust-dtolnay__anyhow", "dtolnay/anyhow", "2cbb4e1bcbf8e7015f8d8b04a291c4c0597030b4", "rust", "MIT"),
    ("go-spf13__cobra", "spf13/cobra", "adbc8813901bba65827259daa8e22ff94ec1f30e", "go", "Apache"),
    ("go-spf13__viper", "spf13/viper", "528f7416c4b56a4948673984b190bf8713f0c3c4", "go", "MIT"),
]

# Documented binary bulk excluded from the corpus (never source, never tests).
PRUNE = {
    "java-ben-manes__caffeine": (
        "simulator/src/main/resources/com/github/benmanes/caffeine/cache/simulator/parser/*",
    ),
}

SRC_EXT = {
    "python": (".py",), "cpp": (".h", ".hpp", ".cc", ".cpp", ".hxx"),
    "java": (".java",), "typescript": (".ts", ".js", ".mjs", ".cjs"),
    "rust": (".rs",), "go": (".go",),
}
TEST_DIR_HINT = ("test", "tests", "__tests__", "spec", "e2e")


def sh(cmd, **kw):
    subprocess.run(cmd, check=True, **kw)


def digest(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def fetch(bench: Path):
    corpus = bench / "corpus" / "holdout"
    for scope, repo, sha, lang, lic in PINS:
        root = corpus / scope
        if (root / ".fetch-sha").exists() and (root / ".fetch-sha").read_text().strip() == sha:
            print(f"have {scope}")
            continue
        shutil.rmtree(root, ignore_errors=True)
        root.mkdir(parents=True)
        sh(["git", "init", "-q"], cwd=root)
        sh(["git", "remote", "add", "origin", f"https://github.com/{repo}.git"], cwd=root)
        sh(["git", "fetch", "-q", "--depth", "1", "origin", sha], cwd=root)
        sh(["git", "checkout", "-q", "FETCH_HEAD"], cwd=root)
        got = subprocess.run(["git", "rev-parse", "HEAD"], cwd=root,
                             capture_output=True, text=True, check=True).stdout.strip()
        assert got == sha, (scope, got, sha)
        (root / ".fetch-sha").write_text(sha)
        # prune: version control and documented binary bulk (never source)
        shutil.rmtree(root / ".git", ignore_errors=True)
        for pat in PRUNE.get(scope, ()):
            for p in root.glob(pat):
                if p.is_dir():
                    shutil.rmtree(p, ignore_errors=True)
                else:
                    p.unlink(missing_ok=True)
        # license gate: a LICENSE file must exist and name a permissive license
        lic_files = list(root.glob("LICENSE*")) + list(root.glob("LICENCE*"))
        assert lic_files, f"{scope}: no LICENSE file"
        blob = "\n".join(p.read_text(errors="replace") for p in lic_files)
        permissive = ("MIT License", "Apache License", "Redistribution and use",
                        "Permission is hereby granted")
        assert any(m in blob for m in permissive) and \
            "General Public License" not in blob, \
            f"{scope}: no permissive marker or copyleft present"
        print(f"fetched {scope} {sha[:12]} license-ok")


def is_test_path(p: Path) -> bool:
    s = p.as_posix().lower()
    name = p.name.lower()
    if any(f"/{h}/" in f"/{s}" or s.startswith(f"{h}/") for h in TEST_DIR_HINT):
        return True
    return (name.startswith("test_") or name.endswith("_test.py")
            or name.endswith(".test.ts") or name.endswith(".test.js")
            or name.endswith(".spec.ts") or name.endswith(".spec.js")
            or name.endswith("_test.go") or name.endswith("test.java")
            or (name.startswith("test") and p.suffix in (".cpp", ".cc")))


def defined_symbols(text: str, lang: str):
    pats = {
        "python": r"(?m)^(?:def|class)\s+([A-Za-z_][A-Za-z0-9_]*)",
        "cpp": r"(?m)^[A-Za-z_][A-Za-z0-9_:<>~]*\s+([A-Za-z_][A-Za-z0-9_:~]*)\s*\(",
        "java": r"(?m)^\s*(?:public|protected|private|static|final|synchronized|\s)*"
                r"(?:class|interface|enum|[A-Za-z_][A-Za-z0-9_<>\[\]]*)\s+([A-Za-z_][A-Za-z0-9_]*)"
                r"\s*(?:\(|{)",
        "typescript": r"(?m)^\s*(?:export\s+)?(?:function|class|const|interface|type)\s+([A-Za-z_][A-Za-z0-9_]*)",
        "rust": r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:fn|struct|enum|trait|impl)\s+([A-Za-z_][A-Za-z0-9_]*)",
        "go": r"(?m)^func\s+(?:\([^)]*\)\s+)?([A-Z][A-Za-z0-9_]*)",
    }
    return sorted(set(re.findall(pats[lang], text)))


def candidates(bench: Path):
    out = []
    for scope, repo, sha, lang, lic in PINS:
        root = bench / "corpus" / "holdout" / scope
        assert root.exists(), f"missing {scope}: run fetch first"
        src_syms = {}   # symbol -> rel path (first definition wins)
        test_blobs = {}  # rel path -> text
        for p in sorted(root.rglob("*")):
            if not p.is_file() or ".git/" in p.as_posix():
                continue
            if p.suffix not in SRC_EXT[lang]:
                continue
            rel = p.relative_to(root).as_posix()
            try:
                text = p.read_text(errors="replace")
            except OSError:
                continue
            if is_test_path(p):
                test_blobs[rel] = text
            else:
                for s in defined_symbols(text, lang):
                    src_syms.setdefault(s, rel)
        # candidate: src symbol named in >=1 test file
        for sym, rel in sorted(src_syms.items()):
            if len(sym) < 3:
                continue
            hits = [t for t, b in test_blobs.items()
                    if re.search(rf"\b{re.escape(sym)}\b", b)]
            if hits:
                out.append({"scope": scope, "language": lang, "symbol": sym,
                            "target_file": rel, "test_files": sorted(hits)[:5],
                            "repo": repo, "sha": sha})
    dest = HERE / "candidates.json"
    dest.write_text(json.dumps(out, indent=1))
    print(f"{len(out)} candidates -> {dest}")


def ngrams(words, n=6):
    return {" ".join(words[i:i + n]) for i in range(len(words) - n + 1)}


def comments_of(text: str, lang: str) -> str:
    if lang == "python":
        return "\n".join(re.findall(r"#.*", text)) + "\n" + \
            "\n".join(re.findall(r'"""(.*?)"""', text, re.S))
    if lang in ("java", "cpp", "typescript", "rust", "go"):
        return "\n".join(re.findall(r"//.*", text)) + "\n" + \
            "\n".join(re.findall(r"/\*(.*?)\*/", text, re.S))
    return ""


def check(bench: Path):
    aq = json.loads((HERE / "queries-authored.json").read_text())
    assert isinstance(aq, list) and aq, "queries-authored.json empty"
    # frozen disjointness
    frozen_ids, frozen_desc = set(), set()
    for name in ("repoqa-queries.json", "cosqa-queries.json"):
        for bdir in (bench.parent / d for d in ("bench-2d",)):
            p = bdir / name
            if not p.exists():
                continue
            for q in json.loads(p.read_text()):
                frozen_ids.add(q["id"])
                frozen_desc.add(digest(q["query"].encode()))
    seen_ids = set()
    for i, q in enumerate(aq):
        for k in ("id", "scope", "language", "query", "path",
                  "start_line", "end_line", "name", "provenance"):
            assert k in q, (i, f"missing {k}")
        assert q["id"] not in seen_ids, f"dup id {q['id']}"
        seen_ids.add(q["id"])
        assert q["id"] not in frozen_ids, f"collides frozen {q['id']}"
        assert digest(q["query"].encode()) not in frozen_desc, f"query {q['id']} dupes frozen text"
        assert q["query"].strip() and len(q["query"].split()) >= 20, (q["id"], "too short")
        root = bench / "corpus" / "holdout" / q["scope"]
        tgt = root / q["path"]
        assert tgt.exists(), (q["id"], "target missing")
        lines = tgt.read_text(errors="replace").splitlines()
        assert 1 <= q["start_line"] <= q["end_line"] <= len(lines), (q["id"], "bad span")
        # 6-gram gate vs target file comments/docstrings
        cgrams = ngrams(comments_of(tgt.read_text(errors="replace"), q["language"]).split())
        qgrams = ngrams(q["query"].split())
        overlap = qgrams & cgrams
        assert not overlap, (q["id"], f"6-gram leak: {sorted(overlap)[0][:80]}")
        # identifier gate: target symbol must not appear as a whole word
        assert not re.search(rf"\b{re.escape(q['name'])}\b", q["query"]), \
            (q["id"], "target symbol in query")
    print(f"check OK: {len(aq)} queries, gates pass")


def assemble(bench: Path):
    check(bench)
    aq = json.loads((HERE / "queries-authored.json").read_text())
    out = [dict(id=q["id"], scope=q["scope"], language=q["language"],
                query=q["query"], path=q["path"], start_line=q["start_line"],
                end_line=q["end_line"], repoqa_start_line=q["start_line"],
                repoqa_end_line=q["end_line"], name=q["name"]) for q in aq]
    (bench / "holdout-queries.json").write_text(json.dumps(out, indent=1))
    (bench / "holdout-provenance.json").write_text(json.dumps(
        {q["id"]: q["provenance"] for q in aq}, indent=1))
    print(f"assembled {len(out)} -> {bench / 'holdout-queries.json'}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bench", required=True)
    ap.add_argument("cmd", choices=("fetch", "candidates", "assemble", "check"))
    a = ap.parse_args()
    bench = Path(a.bench)
    bench.mkdir(parents=True, exist_ok=True)
    {"fetch": fetch, "candidates": candidates,
     "assemble": assemble, "check": check}[a.cmd](bench)


if __name__ == "__main__":
    sys.exit(main())
