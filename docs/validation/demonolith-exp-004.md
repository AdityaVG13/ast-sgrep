# Demonolith EXP-004 — MCP protocol vs sandbox leaf cut

**Verdict:** SEAM_CONFIRMED  
**Run:** 2026-08-13-ast-sgrep-wt-demonolith-1  
**Branch:** `refactor/de-monolithize-isomorphic`  
**Finding:** mcp/lib.rs B4 hub watchlist — sandbox/IO leaf helpers vs JSON-RPC / McpServer

## What moved

| Step | Commit | Change |
|---|---|---|
| 1 | `182e76f` | Sandbox read helpers → `crates/ast-sgrep-mcp/src/sandbox.rs`; `mod sandbox;` + `use sandbox::read_node;` in `lib.rs` |

**Moved into `sandbox.rs` (`pub(crate)`):** `parse_node_id`, `same_opened_file`, `truncate_chars`, `read_node`, `scan_line_window`, plus `MAX_SCAN_BYTES` (scan-only constant).

**Left in `lib.rs`:** `McpServer`, JSON-RPC dispatch (`run_stdio` / `handle_request`), `SearcherCache`, wire types, `write_resp`, `fnv1a64`, `#[path]` unit tests for write_resp and cache (path depth unchanged).

No private-field widening: helpers were already free functions with no `McpServer` field access.

## Evidence

### Behavior
- Command: `rch exec -- cargo test --workspace --no-fail-fast` (spark-1672)
- Result: **488 passed / 0 failed / 4 ignored** (exit 0)
- Matches Phase 3 baseline 488/0/4

### Public API
- Command: `cargo +nightly public-api --simplified -p ast-sgrep-mcp` and `-p ast-sgrep-core`
- Diff vs workspace `api_snapshot_before.txt` (set compare of package bodies): **0 removals, 0 additions**
- No public `sandbox` leak (`mod sandbox;` is private; helpers are `pub(crate)` only)

### Structural
- No new `Box<dyn` / `Arc<dyn` / trait-object indirection
- No public symbol renames; no sqlite/index/search core edits
- `write_resp` + `fnv1a64` deliberately left in `lib.rs` (protocol / session-hash, not sandbox leaf)

### Gate script / SKIPPED
- GATE 4/5: **SKIPPED** (Phase 3 benches/compile-RSS incomplete; `--quick` class)
- Binding proof is the manual suite + public-api runs above (same class as EXP-001/002/003)
- Workspace log: `ast-sgrep-wt-demonolith__demonolith_workspace/phase5_experiment_results/EXP-004.log`

## Non-goals (this pass)
- Moving `write_resp` / `fnv1a64` into a protocol module  
- Splitting `McpServer` / SearcherCache  
- sqlite / index / search core monoliths  
