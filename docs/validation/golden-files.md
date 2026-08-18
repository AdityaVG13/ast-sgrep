# Golden files

Frozen dumps live next to their tests (`tests/*/fixtures/`) or under
`tests/golden/`. Provenance: [`tests/golden/PROVENANCE.md`](../../tests/golden/PROVENANCE.md).
Compare helper: `assert_golden` / `assert_golden_json_at` in `ast-sgrep-testkit`.

## Env

| Value | Mode |
|---|---|
| unset, `0`, `false`, `off` | **compare** (default; CI) |
| `1`, `true`, `yes`, `on` (case-insensitive) | **update** (local only) |

Use `ASGREP_UPDATE_GOLDENS` only. Never `UPDATE_GOLDENS` or `INSTA_UPDATE`.
Mismatches write `{golden}.actual` (gitignored). Do not commit `*.actual`.

## Local update

1. Run the targeted test with `ASGREP_UPDATE_GOLDENS=1`.
2. `git diff` the golden(s) file-by-file. Reject host paths (`/Users/`, `/home/`,
   `/var/folders/`). Scrub via `Scrubber` presets (`search_dump`,
   `machine_contract`); keep scores unless the product format omits them.
3. Commit the freeze. CI never rewrites goldens.

If tests run on Spark via `rch exec`, UPDATE writes on the worker and does **not**
rsync back. Copy the files immediately (`scp` or `tar` over ssh). The next `rch`
sync can delete uncopied dumps (`rsync --delete`).

## CI

CI is **compare only**. `ASGREP_UPDATE_GOLDENS=0` is pinned on golden-bearing
jobs in `.github/workflows/ci.yml`. Never set update mode under `.github/`.
Failed jobs upload `*.actual` artifacts.

## Not goldens

Do **not** use this SOP for [`benchmarks/results/baselines.md`](../../benchmarks/results/baselines.md).
Published numbers follow Agents.md honesty (fingerprint + status tag, or
`UNREPRODUCIBLE`). Metric files are not auto-rewritten.

## PR vs dispatch (B4)

GitHub Actions is `workflow_dispatch` only (no `pull_request` / `push`
triggers). Dispatch **CI** when you want compare-only goldens on GitHub.
Do not add a silent full matrix on every PR. The cheaper local gate is the
targeted default bar in [CONTRIBUTING.md](../../CONTRIBUTING.md).
