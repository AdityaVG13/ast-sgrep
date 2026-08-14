# FLOOR promotion protocol (S2 cold-index-self ANN-on)

**Status: `historical` process doc.** Does not rewrite any canonical number.
hoy3.3. Human ACK is required before any `baselines.md` / `speed.md` FLOOR
row mutation.

## Current canonical MEASURED FLOOR

Until a promotion event that passes the checklist below, the T1-R residual
campaign MEASURED FLOOR remains:

| Claim | p95 | Class | Must not be treated as |
|---|---:|---|---|
| **C1 MEASURED FLOOR** | **4.304 s** | versioned fingerprint (campaign `20260806T211603Z`) | superseded by a laptop run |
| **C4 post-T1-R live/self** | **1.965 s** | live chain WIN, same surface family | new FLOOR |
| **C14 aspirational** | ≤ **2.5 s** | met-as-live on the C4 host | FLOOR promotion |

**Live p95 1.965 s alone does not promote FLOOR.** C14 met-as-live does not
promote FLOOR. A single `ASGREP_PERF_PROFILE` wall (hoy3.2: 2.979 s `index_all`
on Darwin, n=1) does not promote FLOOR.

These campaign rows are **not** the 285 ms archived 110-file budget (breached)
and **not** the 2026-08-05 `speed.md` release/1.4.0 cold-index **2.257 s** p95
(`cea904a` lineage). Those are different fingerprints. Two current FLOORs for
the same MATCH tuple is an honesty fail.

## MATCH axes (all required)

A candidate may replace C1 only when **all** of the following match the
candidate run **and** the row it would supersede, or the mismatch is recorded
as a **new** fingerprint id rather than a silent overwrite.

1. **Surface** -- S2 `cold-index-self` (`asgrep index <repo-root>` on a fresh
   `--index-path`, not warm reindex, not `asgrep bench` fixture, not sample
   IVF-off).
2. **ANN gate** -- semantic embed on, IVF built (`semantic_ivf_present: true`),
   chunk count ≥ `DEFAULT_ANN_THRESHOLD` (2000). IVF-off is a different surface.
3. **Corpus fingerprint** -- SHA-256 of the sorted tracked-path manifest
   (`path TAB git-blob-sha`). File **count alone is not a fingerprint**
   (adversarial: same count, swapped contents).
4. **Binary features** -- `release-perf` (or documented equivalent), T1-R
   unit-dot IVF path as shipped, no extra `RUSTFLAGS` beyond the row's pin.
5. **Hyperfine protocol** -- `n ≥ 10`, cold DB each iteration, nearest-rank p95
   on sorted walls: `idx = floor((n - 1) * 95 / 100)` (0-based). Same rule as
   C1.
6. **Host class** -- ISA + OS family recorded (Darwin arm64 vs Linux x86_64
   are not interchangeable for absolute p95). Ratios may still be cited with
   the host tag.

Missing any axis → candidate is a **new row**, not a FLOOR replace.

## Promotion checklist

Record yes/no. Any **no** blocks promotion. On failure, add a negative-ledger
note under `benchmarks/results/` (do not delete the miss).

- [ ] MATCH axes 1–6 recorded for **both** C1 and the candidate
- [ ] Corpus manifest SHA equal, or candidate filed under a **new fingerprint id**
- [ ] `n ≥ 10` hyperfine JSON kept (gitignored raw OK; cite path + SHA)
- [ ] p95 rule identical (nearest-rank, not mean, not Excel percentile)
- [ ] ANN-on evidenced (`asgrep status` → `semantic_ivf_present: true`)
- [ ] Human ACK in the promoting PR / bead Handoff (this packet has **none**)
- [ ] `baselines.md` / `speed.md` would show **exactly one** current FLOOR for
      this MATCH tuple; C1 row marked SUPERSEDED with the new id, not deleted

## Demotion / supersede

A superseded FLOOR stays in the ledger with status `SUPERSEDED` and the
replacing fingerprint id. Do not leave two rows labeled current. Do not
promote sample-fixture p95 (~tens of ms) onto this S2 surface (C18 mismatch).

## Fingerprint command (document / dry-run)

```bash
python3 - <<'PY'
from hashlib import sha256
from pathlib import Path
import subprocess
root = Path(".")
files = subprocess.check_output(["git", "ls-files", "-z"], cwd=root).split(b"\0")
files = sorted(f.decode() for f in files if f)
h = sha256()
h.update(f"{len(files)}\n".encode())
for rel in files:
    blob = subprocess.check_output(["git", "hash-object", rel], cwd=root).strip()
    h.update(rel.encode() + b"\t" + blob + b"\n")
print("tracked_files", len(files))
print("manifest_sha256", h.hexdigest())
print("head", subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root).decode().strip())
PY
```

### Dry-run (not a promotion)

| Field | Value |
|---|---|
| When | 2026-08-14 |
| HEAD | `ac6bcf1` (`feat/golden-assert-testkit`) |
| tracked_files | 597 |
| manifest_sha256 | `a98b7ca756a2bb9da7ff2f12e5b6fcafdd6ee117c9c22e01144af19c0c0a77ae` |
| Role | evidence that a content manifest exists; **does not** MATCH C1 corpus |

## Hyperfine n protocol (verify, do not treat output as FLOOR)

```bash
# Cold DB per iteration. n>=10. Do not paste p95 into baselines.md from this recipe.
rm -f /tmp/asgrep-s2-floor/i*.db*
hyperfine --runs 10 --prepare 'rm -f /tmp/asgrep-s2-floor.db /tmp/asgrep-s2-floor.db-wal /tmp/asgrep-s2-floor.db-shm' \
  --export-json /tmp/hyperfine_s2_cold_index_self.json \
  './target/release-perf/asgrep --json --index-path /tmp/asgrep-s2-floor.db index .'
python3 - <<'PY'
import json, math
from pathlib import Path
times = sorted(json.loads(Path("/tmp/hyperfine_s2_cold_index_self.json").read_text())["results"][0]["times"])
n = len(times)
idx = ((n - 1) * 95) // 100
print("n", n, "nearest_rank_p95_s", times[idx])
PY
```

Pass 5 (hoy3) does **not** auto-promote. Canonical FLOOR stays **4.304 s**.
