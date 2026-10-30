"""#2 Generate the frozen structural-intent corpus from repoqa index dbs.

Classes per repo (deterministic, name-sorted selection):
  callers:K  callees defined in the repo (symbols.name) with 2-20 call
            sites across >=2 files; query `callers:<name>`; gold = site files
  defs:K     named symbols (len>=4) with exactly 1 def file; gold = def file
  imports:K  module_paths imported by >=2 files; gold = importing files

Output: struct-queries.json (frozen; checksum printed for the manifest).
"""
import hashlib, json, sqlite3
from pathlib import Path

BENCH = Path("~/.hermes/cache/scratch/asgrep-frozen-20261028").expanduser()
REPOS = sorted((BENCH / "corpus/repoqa").iterdir())
N_CALLERS, N_DEFS, N_IMPORTS = 5, 3, 2

queries = []
for repo in REPOS:
    db = repo / ".asgrep" / "index.db"
    if not db.exists():
        print("SKIP (no index):", repo.name)
        continue
    conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    scope = repo.name
    files = {i: p for i, p in conn.execute("SELECT id, path FROM files")}
    sym_names = {r[0] for r in conn.execute("SELECT DISTINCT name FROM symbols")}

    # callers
    sites = {}
    for callee, fid, line in conn.execute("SELECT callee, file_id, line_no FROM callers"):
        sites.setdefault(callee, []).append((fid, line))
    picked = 0
    for callee in sorted(s for s in sites if s in sym_names):
        if picked >= N_CALLERS:
            break
        s = sites[callee]
        fset = {files[f] for f, _ in s}
        if 2 <= len(s) <= 20 and len(fset) >= 2:
            queries.append(dict(id=f"struct-{scope}-callers-{picked:02d}", type="callers",
                                scope=scope, query=f"callers:{callee}",
                                gold=sorted(fset), symbol=callee, n_sites=len(s)))
            picked += 1

    # defs
    picked = 0
    defs = {}
    for name, fid in conn.execute("SELECT name, file_id FROM symbols"):
        if len(name) >= 4:
            defs.setdefault(name, set()).add(files[fid])
    for name in sorted(d for d in defs if len(defs[d]) == 1):
        if picked >= N_DEFS:
            break
        queries.append(dict(id=f"struct-{scope}-defs-{picked:02d}", type="defs",
                            scope=scope, query=f"defs:{name}",
                            gold=sorted(defs[name]), symbol=name, n_sites=1))
        picked += 1

    # imports
    picked = 0
    imps = {}
    for mod, fid in conn.execute("SELECT module_path, file_id FROM imports"):
        imps.setdefault(mod, set()).add(files[fid])
    for mod in sorted(m for m in imps if len(imps[m]) >= 2 and len(m) >= 4):
        if picked >= N_IMPORTS:
            break
        queries.append(dict(id=f"struct-{scope}-imports-{picked:02d}", type="imports",
                            scope=scope, query=f"imports:{mod}",
                            gold=sorted(imps[mod]), symbol=mod, n_sites=len(imps[mod])))
        picked += 1
    conn.close()

out = BENCH / "struct-queries.json"
out.write_text(json.dumps(queries, indent=1, sort_keys=True))
digest = hashlib.sha256(out.read_bytes()).hexdigest()
from collections import Counter
print(f"queries: {len(queries)} | by type: {dict(Counter(q['type'] for q in queries))}")
print(f"sha256: {digest}")
print("written:", out)
