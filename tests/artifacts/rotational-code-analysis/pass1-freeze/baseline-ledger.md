# Baseline command ledger — loop 1

Frozen revision: `fb932aac852f5496c0a7035cc5a0b508e05111cb` · recorded `2026-08-11T01:35:40Z`  
Policy: **safe cheap probes only**. Did **not** run `cargo test --workspace` or full workspace builds.

## Toolchain versions

| Command | Exit | Concise output |
|---|---:|---|
| `cargo --version` | 0 | `cargo 1.97.1 (c980f4866 2026-06-30)` |
| `rustc --version` | 0 | `rustc 1.97.1 (8bab26f4f 2026-07-14)` |
| `rustup show active-toolchain` | 0 | `stable-aarch64-apple-darwin (default)` |
| `node --version` | 0 | `v24.14.1` |
| `npm --version` | 0 | `11.11.0` |
| `python3 --version` | 0 | `Python 3.14.6` |
| `zs --version` | 0 | `zs 1.3.0` (CLI present) |
| `zerostack-codemode-host --version` | 127 | **not found** |
| `zs --json -C TARGET token '…'` | 127 | engine `tokenzero-codemode` missing — **zerostack gap stated**; native shell used |

## Discovery / freeze

| Command | Exit | Notes |
|---|---:|---|
| `python3 …/ensure_rotation_ignore.py TARGET` | 0 | `.rotational-code-analysis/` gitignore updated |
| `git rev-parse HEAD` | 0 | `fb932aac852f5496c0a7035cc5a0b508e05111cb` |
| `git rev-parse --abbrev-ref HEAD` | 0 | `perf/software-optimization` |
| `git status -sb` / `--short` | 0 | dirty; 34 short lines |
| `git log -1 --oneline` | 0 | `fb932aa skill-loop pass 12/12: cyclomatic absolute convergence (ΣCC stable)` |
| `python3 …/spin.py init --repo . --out .rotational-code-analysis --action audit --iteration 1` | 0 | state + snapshot written; 6313 in_scope / 48991 discovered |

## Cargo / compile probes (safe)

| Command | Exit | Notes |
|---|---:|---|
| `cargo metadata --no-deps --format-version 1` | 0 | 11 packages; full target list captured (bins/tests/benches) |
| `cargo check -p ast-sgrep-core --message-format=short` | 0 | `Finished dev profile in 0.21s` (cache hit; already built) |

## Discovered but **not** run (ledger only)

From README.md / CONTRIBUTING.md / package.json / docs/RELEASING.md:

| Command | Why deferred |
|---|---|
| `cargo check --workspace -j1` | heavier; reserved for later verification loops |
| `cargo test -p ast-sgrep-core --test parity -j1 -- --test-threads=1` | recommended smoke test; not required for freeze |
| `cargo test --workspace` | **explicitly forbidden** this pass |
| `cargo build --release -p ast-sgrep-cli` / `-p ast-sgrep-mcp` / `-p ast-sgrep-lsp` | release builds; not freeze probes |
| `npm run check:pi-contract` | needs node workspace install discipline |
| `npm run check:pi-release` | release gate |
| `npm run test:pi-release-gate` | packs artifacts |
| `npm run test:pi-e2e` | e2e; heavier |
| `npm run release:preflight` | release only |

## Integration test binaries (metadata only)

`ast-sgrep-core` integration tests include (sample): `parity`, `e2e_smoke`, `properties`, `metamorphic`, `ranking_oracle`, `semantic_*`, `store_*`, … (~40 `[[test]]` targets).  
Bins: `asgrep`, `ast-sgrep` (cli), `asgrep-lsp`, `asgrep-mcp`. Bench: `search`. N-API: `ast_sgrep_codemode_napi` cdylib.

## Evidence kind

- **source:** Cargo.toml workspace members, package.json scripts, README/CONTRIBUTING command tables
- **runtime:** version probes, `cargo metadata`, `cargo check -p ast-sgrep-core`, git freeze commands
