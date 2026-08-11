# Pass 9 — Surface residual (cli / mcp / lsp / lang / launcher)

Run: `2026-08-11Tpass9-surface` (canonical via `.cyclomatic-reduction/LATEST`)  
Mode: **module-pass residual** · Technique family: **shared collapse** + **consolidate predicates**  
zerostack: unavailable (`fszero-codemode` not found); lizard 1.23.0 + `measure_complexity.py`

## Census (surface packages, product hotspots CC>10)

| Package | ΣCC | max CC | hotspots >10 (prod) |
|---|---:|---:|---:|
| `ast-sgrep-cli` | 632 → **630** | 29 → **24** | 12 → fewer (run_bench under ceiling) |
| `ast-sgrep-mcp` | 226 | 12 | 2 (`run_stdio`, `scan_line_window`) |
| `ast-sgrep-lsp` | 277 | 9 | **0** |
| `ast-sgrep-lang` | 336 | 20 | 8 (mostly Keep domain) |
| `packages/pi/launcher` | 218 | 26 | 3 (`resolveHost`/`Binary`/`Codemode`) |

## Transforms landed (Cut)

| Function | CC before → after | Technique | File |
|---|---|---|---|
| `run_bench_suite` | 29 → **24** | shared collapse (ratchet + ast-grep print + skip JSON) | `crates/ast-sgrep-cli/src/bench.rs` |
| `run_bench` | 15 → **9** | same shared helpers | same |
| `run_search` | 13 → **10** | consolidate predicates (`uses_semantic_channel`) | `crates/ast-sgrep-cli/src/search_cmd.rs` |

### New private helpers

| Helper | Role |
|---|---|
| `enforce_bench_ratchet` | One ratchet gate for suite + query (eliminates duplicate decision tree) |
| `print_ast_grep_human` | Shared human ast-grep comparison lines (suite indent vs single-query) |
| `uses_semantic_channel` | Single semantic-channel predicate for `run_search` |

Public APIs unchanged.

## ΣCC bill

| Scope | Before | After | Δ |
|---|---:|---:|---:|
| Touched files (`bench.rs` + `search_cmd.rs`) | 151 | **149** | **-2** |
| `ast-sgrep-cli` package | 632 | **630** | **-2** |
| launcher (no product edit) | 218 | 218 | 0 |

**Displacement check: pass** (ΣCC down; not a dump).

## Refused this wave (≥3 measured attempts — pure extract raised ΣCC)

| Attempt | Measured touched/file ΔΣCC | Resolve |
|---|---|---|
| `run_process` → `exit_clap_parse_error` + `exit_run_error` + tip helper | **+3** (`lib.rs` 62→65) | **Refuse** |
| launcher `assertHostManifestMatches` + addon override/host helpers | **+6** (`index.js` 102→108) | **Refuse** |
| `measure_suite_case` + `print_suite_human` pure extract (with shared helpers) | **+3** (`bench.rs` 101→104 on first trial) | **Refuse** |

Pass 8 precedent: pure extract without decision elimination rejected until Σ funded.

## Keep / Defer residuals (surface)

| Function | CC | Resolve | Note |
|---|---:|---|---|
| `resolveHost` | 26 | Defer/Refuse pure extract | Pass 3 guards done; further extract bill +6 measured |
| `resolveCodemodeAddon` | 18 | Defer/Refuse pure extract | same |
| `resolveBinary` | 17 | Defer | PATH fallback domain + already thin |
| `run_process` | 16 | Defer/Refuse pure extract | clap/agent error ladder; +3 measured |
| `run_bench_suite` | 24 | Defer residual | case-loop identity contract essential; pure extract Refuse |
| `run_bench_batch` | 16 | Defer | batch top-10 packing |
| `run_chain` / `run_watch` | 14 | Defer | event/output loops |
| `classify_native` | 20 | **Keep** | language pattern taxonomy (ledger) |
| `cached_pattern_signatures` | 19 | **Keep** | signature domain |
| `apply_kind_rule` | 15 | **Keep**-leaning | KindRule match arms = domain |
| MCP `run_stdio` / `scan_line_window` | 12 | Defer | `read_node` already pass-4 extracted |
| LSP | max 9 | **no action** | under hard ceiling |

## Parity

| Check | Result |
|---|---|
| `cargo check -p ast-sgrep-cli` | ok |
| `cargo test -p ast-sgrep-cli --test machine_contracts bench` | 3/3 ok |
| `cargo test -p ast-sgrep-cli --test cli_smoke` | 2/2 ok |
| `cargo test -p ast-sgrep-cli --lib` | 10/10 ok |
| launcher `node --test` npm-native + binary-env-alias | 13/13 ok (unchanged product; regression floor) |

Ratchet messages preserve wording: `bench ratchet failed for suite {name}:…` / `for query {query:?}:…`.

## Metric-gaming auditor (edit pass)

- Shared collapse funds helpers (duplicate trees removed).  
- No public API rename.  
- No domain scatter of KindRule / platform security codes.  
- First-wave pure extracts **reverted** after measure showed +ΣCC.

## Mirror files

- `pass9-summary.json` — package summaries + refuse ledger  
- `bench-before.json` / `bench-after.json`  
- `search_cmd-before.json` / `search_cmd-after.json`  
- this notes file · `07-parity-report-pass9.md` · scorecard stub  
