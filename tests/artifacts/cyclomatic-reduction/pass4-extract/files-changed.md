# Pass 4 — files changed

| Path | What changed |
|---|---|
| [`crates/ast-sgrep-core/src/index.rs`](../../../../crates/ast-sgrep-core/src/index.rs) | `index_all` → `commit_prepared_files` + `post_index_hooks`; `index_content_at` → `try_structure_skip_refresh` |
| [`crates/ast-sgrep-core/src/store/sql.rs`](../../../../crates/ast-sgrep-core/src/store/sql.rs) | `delete_file_lines` arms call `exec_line_deletes` with ordered SQL tables (DELETE order preserved) |
| [`crates/ast-sgrep-cli/src/lib.rs`](../../../../crates/ast-sgrep-cli/src/lib.rs) | `run_codemode_batch` → `load_batch_raw` + `apply_cli_batch_defaults`; envelope fields unchanged |
| [`crates/ast-sgrep-mcp/src/lib.rs`](../../../../crates/ast-sgrep-mcp/src/lib.rs) | `read_node` → `scan_line_window`; TOCTOU path checks stay inline |
| [`packages/pi/extension/src/code-mode.ts`](../../../../packages/pi/extension/src/code-mode.ts) | `parseSearchHit` → `isValidHitShape` for protocol field gate |

No public API signature changes. No commit performed (orchestrator constraint).
