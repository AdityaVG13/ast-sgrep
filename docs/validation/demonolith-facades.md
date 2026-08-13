# Demonolith façades + isomorphism (post EXP-001..004)

**Verdict:** ALREADY CORRECT — no façade holes; no code changes this pass  
**Run:** 2026-08-13-ast-sgrep-wt-demonolith-1  
**Branch:** `refactor/de-monolithize-isomorphic`  
**Scope:** Façade + isomorphism verification only (no new cluster extractions)

Prior extractions kept: EXP-001 sqlite/`queries.rs`, EXP-002 `index_recovery.rs`, EXP-003 `search/finish.rs`, EXP-004 `mcp/sandbox.rs`.

## Checks performed

### 1. Extracted-module visibility (`pub` free fns / modules)

| File | Free-fn / module visibility | Notes |
|---|---|---|
| `crates/ast-sgrep-core/src/index_recovery.rs` | `recover_corrupt_index` is `pub(crate)`; helpers private | No `pub fn` / `pub struct` / `pub mod` |
| `crates/ast-sgrep-core/src/store/sqlite/queries.rs` | No free fns; `impl IndexStore { pub fn … }` only | Method `pub` is pre-existing `IndexStore` surface; module is private (`mod queries;`) |
| `crates/ast-sgrep-core/src/search/finish.rs` | **`pub fn finish_response` only** among extracted free fns; `finish_response_checked` is `pub(crate)`; helpers private/`pub(super)` | Matches EXP-003 contract |
| `crates/ast-sgrep-mcp/src/sandbox.rs` | All helpers `pub(crate)` | No public free fns |

Command evidence: `rg -n "pub (fn|struct|mod)"` / `^pub fn` on the four files.

### 2. Existing import paths still resolve

| Path / symbol | Status |
|---|---|
| `ast_sgrep_core::search::finish_response` | OK — `pub mod search` + `pub use finish::finish_response` in `search/mod.rs` |
| `store::IndexStore` + DTOs from `store/mod.rs` | OK — `pub use sqlite::{ IndexStore, … }` façade unchanged |
| `pub mod index` public index surface | OK — `lib.rs` keeps `pub mod index;` |
| `index_recovery` public? | **No** — `mod index_recovery;` (crate-private) |
| MCP sandbox public? | **No** — `mod sandbox;` + `use sandbox::read_node;` |

`cargo +nightly public-api --simplified` still lists exactly one `finish_response` entry (`ast_sgrep_core::search::finish_response`); zero hits for `index_recovery` / `sandbox` / `recover_corrupt`.

### 3. `#[path]` test includes still resolve

Still present (paths unchanged by façades):

- `search/mod.rs` → `../../../../tests/unit/core/search.rs`
- `mcp/lib.rs` → `../../../tests/unit/mcp/lib__write_resp_tests.rs`, `lib__cache_tests.rs`
- `store/sqlite/mod.rs` → restore_synchronous + pass3 deep-core unit paths

Full suite compile/run proves they resolve (see check 5).

### 4. Public API vs `api_snapshot_before.txt` — 0 removals

```text
cargo +nightly public-api --simplified -p ast-sgrep-core
cargo +nightly public-api --simplified -p ast-sgrep-mcp
```

Set-compare of package bodies vs workspace `api_snapshot_before.txt`:

- **ast-sgrep-core:** 0 removals, 0 additions (set size 1951)
- **ast-sgrep-mcp:** 0 removals, 0 additions (set size 12)

(Note: multi-`-p` in one argv is rejected by current `cargo-public-api`; packages captured separately, same as EXP-003/004.)

### 5. Full suite counts

```text
rch exec -- cargo test --workspace --no-fail-fast
```

- Host: spark-1672 (RCH offload)
- Result: **488 passed / 0 failed / 4 ignored** (exit 0)
- Matches Phase 3 / EXP-001..004 baseline

### 6. No new `dyn` dispatch in extracted files

- `index_recovery.rs`, `search/finish.rs`, `mcp/sandbox.rs`: no `dyn`
- `sqlite/queries.rs`: two `&dyn ToSql` casts only — identical count/pattern as `origin/main` pre-extract `store/sqlite.rs` (2× `dyn ToSql`)
- No new `Box<dyn` / `Arc<dyn` / trait-object indirection

## Issues found vs already correct

1. **Visibility façades** — already correct (only `finish_response` is `pub` among extracted free fns).
2. **Re-export / import paths** — already correct (`finish_response`, `IndexStore` DTOs, private `index_recovery` / `sandbox`).
3. **`#[path]` unit includes** — already correct; suite compiles.
4. **API snapshot** — already empty diff (0 removals / 0 additions).
5. **Behavior counts** — already 488/0/4.
6. **dyn neutrality** — already correct (moved `ToSql` casts only).

## This pass

- **Extractions:** none (by design)
- **Code fixes:** none (no missing `pub use`, no accidental pub leaks)
- **Artifact:** this file only
