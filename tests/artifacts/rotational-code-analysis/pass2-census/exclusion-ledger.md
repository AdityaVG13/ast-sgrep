# Exclusion ledger — Pass 2 / Loop 2

Freeze: `fb932aac852f5496c0a7035cc5a0b508e05111cb`  
Method: `git ls-tree -r --name-only FREEZE` for tracked surface; `find`/`du` for disk-only noise.  
**Supersedes** pass1 spin inventory (48991 discovered / 6313 in_scope) for scope decisions.

## Policy

- Every tracked path is **in-scope** or listed here with reason.
- Unmatched readable product text stays in-scope (open-world).
- Exclusions are for **attack scope**, not for deleting from git.

## Tracked exclusions (62 files)

| Module | Count | Kind | Reason |
|---|---:|---|---|
| X-pi-dist | 24 | generated | `packages/pi/extension/dist/**` — tsc output; rebuild via extension `npm run build` |
| X-beads | 20 | tracker | `.beads/**` — issue-tracker DB/history/locks; not product source |
| X-native-assets | 15 | binary-asset | `packages/pi/platforms/*/asgrep(.exe)`, `*.node`, `*.sha256` — release native slots (often LFS empty placeholders) |
| X-skill-loop | 3 | skill-loop-noise | tracked `.skill-loop-progress-*.md` campaign notes |

## Disk-only exclusions (~91,860 files; not in freeze tree)

| Pattern | Approx files | Size | Reason |
|---|---:|---|---|
| `target/` | 18974 | 3.9G | primary cargo build tree (gitignored) |
| `target-pass4/` | 6669 | 1.4G | skill-loop isolated cargo target (**B-SNAPSHOT-NOISE**) |
| `target-pass8/` | 11935 | 2.1G | skill-loop isolated cargo target |
| `target-pass11/` | 12064 | 2.7G | skill-loop isolated cargo target |
| `target-pass13/` | 8633 | 1.6G | skill-loop isolated cargo target |
| `target-pass14/` | 10019 | 2.1G | skill-loop isolated cargo target |
| `target-pass15/` | 1416 | 294M | skill-loop isolated cargo target |
| `packages/pi/extension/node_modules/` | 20241 | 208M | npm vendor |
| `node_modules/` | 3 | 52K | root npm stub |
| `fuzz/target/` | 1881 | — | cargo-fuzz build tree |
| `.rotational-code-analysis/` | 9 | 80M | this campaign books (gitignored) |
| `tests/artifacts/bug-hunt/` | 11 | — | untracked skill-loop artifact dump |
| untracked `.skill-loop-progress-*.md` | 5 | small | extra progress notes beyond tracked set |

**Sum of `target-pass*` alone ≈ 50.7k files / ~10G** — root cause of pass1 D-language/asset inflation.

## Explicitly NOT excluded (stay in scope)

- `fuzz/` sources, seeds, dictionaries (workspace-excluded package but first-class safety surface)
- `tests/**` first-class suites, fixtures, goldens
- `docs/**`, `benchmarks/**` (behavior/docs that constrain claims)
- `scripts/**`, `.github/workflows/**`
- Pi TS/JS sources under `packages/pi/{extension,launcher,scripts}` (non-dist)
- Platform `package.json` / LICENSE / prepack scripts (config/meta; binaries excluded)

## Residual classification notes

- **noext (64 in-scope):** mostly `fuzz/seed_corpus/**` seed files + LICENSE + scripts without extension — classified as fuzz/meta/script, not "D language".
- **Polyglot under tests/fixtures:** sample languages for extraction; not product ownership.
- **HEAD drift:** live `HEAD=9234d113…` is 1 commit after freeze (pass1 books commit). Freeze rev unchanged; do not re-baseline.
