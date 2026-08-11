# Assumptions & blockers — loop 1

Recorded `2026-08-11T01:35:40Z` @ `fb932aac852f5496c0a7035cc5a0b508e05111cb`.

## Environment assumptions

1. **Host:** macOS 26.5, Darwin arm64 (`stable-aarch64-apple-darwin`).
2. **Rust:** 1.97.1 via rustup stable; matches Pi release pin mentioned in docs/RELEASING.md.
3. **Node:** v24.14.1 / npm 11.11.0 via fnm multishell path — may differ across shells.
4. **Python:** 3.14.6 used only for skill helper scripts (`spin.py`, `ensure_rotation_ignore.py`).
5. **Prior build artifacts:** `target/debug` and `target/release` already populated; `cargo check -p ast-sgrep-core` finishing in 0.21s is a **cache hit**, not a cold compile proof.
6. **Dirty tree:** freeze includes local beads DB mutations, dist rebuilds, skill-loop progress files, and untracked `target-pass*` trees. Later loops must not treat dirty paths as product truth without re-hash against clean HEAD when claiming findings on those paths.
7. **ZeroStack:** `zs` CLI 1.3.0 present but codemode engines (`tokenzero-codemode`, `zerostack-codemode-host`) **unavailable**. Native `run_terminal_command` used after stating gap. Do not claim ZeroStack FS/graph evidence this wave.
8. **Snapshot inflation:** spin discovery counted ~49k files (assets/binary/D-hinted mass). Nondeterminism risk: untracked dirs and DB WAL files can change between rotations; `check_frozen_state` may report drift if those paths were marked in-scope.

## Nondeterminism

- `.beads/beads.db` / `-wal` mutate on tracker use.
- Untracked `target-pass*/` and progress markdown may grow during skill-loop work outside this audit.
- `packages/pi/extension/dist/*` dirty — generated outputs may not match committed sources.
- Parallel cargo/index jobs not run; no timing baselines claimed.

## Initial blockers

| ID | Severity | Description | Mitigation for later loops |
|---|---|---|---|
| B-ZS-ENGINES | low (audit) | ZeroStack engines missing | native tools; optional reinstall engines |
| B-DIRTY-FREEZE | medium | Dirty tree + large untracked skill-loop trees | loop 2 exclusion ledger; prefer HEAD-tracked paths for product claims |
| B-SNAPSHOT-NOISE | medium | Snapshot includes asset/binary/D inflation | loop 2 re-classify; exclude `target-pass*`, large fixtures with reasons |
| B-NO-COLD-BUILD | low | No cold `cargo check --workspace` | run in verification ring if needed |
| B-NO-TEST-RUN | info | No tests executed this pass | pass 2+ may run selective parity smoke |

## Not blockers

- Product compile of `ast-sgrep-core` succeeds under current toolchain.
- Workspace metadata parseable without network (`--no-deps`).
