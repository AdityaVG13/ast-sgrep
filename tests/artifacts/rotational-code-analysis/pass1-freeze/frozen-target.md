# Frozen target — iteration 1 / loop 1

| Field | Value |
|---|---|
| **target_root** | `/Users/aditya/Developer/ast-sgrep` |
| **git_revision** | `fb932aac852f5496c0a7035cc5a0b508e05111cb` |
| **branch** | `perf/software-optimization` |
| **upstream** | `origin/perf/software-optimization` (tracking; status shows no ahead/behind in freeze) |
| **dirty** | **true** (working tree not clean) |
| **action_mode** | `audit` (no product source edits under `crates/` or `packages/`) |
| **campaign** | rotational-code-analysis pass 1 of 12 — Freeze + baseline |
| **skill** | rotational-code-analysis 2.0.0 (skill folder read-only) |
| **frozen_at** | `2026-08-11T01:35:40Z` |
| **snapshot_sha256** | `c7b14742a308e688ced488c9b7828b27de13703ffd9785c8835cf3b0cb24d9fb` |
| **snapshot_file** | `.rotational-code-analysis/snapshot.json` |
| **state_file** | `.rotational-code-analysis/state.json` |

## Axes this rotation (≥2)

| Axis | Value |
|---|---|
| scale | repository |
| time | baseline |
| observer | operator |
| evidence | source + runtime commands |

## Scope (operator freeze)

- **In product:** Rust workspace under `crates/*` (11 members), npm workspaces under `packages/pi/*`, docs, benchmarks, tests, CI.
- **Mode:** audit — no product edits; books only under `.rotational-code-analysis/` (+ slim fixture mirror).
- **Exclusions for later attack waves (not reclassified here):** untracked `target-pass*/` skill-loop worktrees, `.skill-loop-progress-*.md`, large binary/asset inventory noise from spin snapshot — loop 2 census must refine exclusions with reasons.
- **Gitignored books:** `.rotational-code-analysis/` (via `ensure_rotation_ignore.py`).

## Dirty tree summary (at freeze)

### Modified (tracked)

- `.beads/` tracker DB + issues.jsonl + last-touched (local runtime)
- `.beads/.br_history/*` — 5 history pairs **deleted**
- `.gitignore` — rotational ignore line(s) applied
- `.papercuts.jsonl`
- `Cargo.lock`
- `packages/pi/extension/dist/*` (generated dist JS/TS)

### Untracked (notable)

- `.skill-loop-progress-{conformance,fuzzing,gauntlet,golden-artifacts,rotational-code-analysis}.md`
- `target-pass4/`, `target-pass8/`, `target-pass11/`, `target-pass13/`, `target-pass14/`, `target-pass15/`
- `tests/artifacts/bug-hunt/`

**Short status count:** 34 lines from `git status --short` at freeze.

## Spin inventory (helper, not census)

`spin.py init` snapshot summary (raw discovery; loop 2 will re-classify):

- discovered_files: 48991
- in_scope_files: 6313
- modules: 22
- estimated_source_tokens: ~3.3e6
- languages (extension-hinted): Rust 168, JS/TS 63, D 1080 (likely fixture/noise), Shell 10, Python 7, …
- kinds: source 1196, test 98, config 1803, docs 144, asset 38867, binary 2947, oversized 839, …

**Caution:** high D-language and asset counts suggest `target-pass*` / fixture corpora inflated the snapshot. Treat as freeze inventory, not trusted semantic census.

## Workspace members (cargo metadata --no-deps)

1. ast-sgrep-core 1.4.0
2. ast-sgrep-embed 1.4.0
3. ast-sgrep-lang 1.4.0
4. ast-sgrep-testkit 1.4.0
5. ast-sgrep-lsp 1.4.0
6. ast-sgrep-mmap 1.4.0
7. ast-sgrep-cli 1.4.0
8. ast-sgrep-codemode 1.4.0
9. ast-sgrep-plugins 1.4.0
10. ast-sgrep-mcp 1.4.0
11. ast-sgrep-codemode-napi 1.4.0

Workspace `exclude = ["fuzz"]`. default-members: `crates/ast-sgrep-cli`.

## Binaries present on disk (not re-run)

- `target/debug/asgrep`, `target/release/asgrep` exist (prior builds).
