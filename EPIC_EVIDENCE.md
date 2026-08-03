# EPIC evidence: `ast-sgrep-lsp-state-zblv` (+ LSP `x46g` / `ziij`)

Branch: `fix/lsp-symbol-correctness-p1` (PR #14)
Worktree: `/workspace/.worktrees/pr14`
Date: 2026-08-03

## Scope closed

| Bead | Title | Disposition |
|---|---|---|
| `ast-sgrep-lsp-state-zblv.2` | Stop flipping `index_ready` on single-file lock ops | Fixed + tested |
| `ast-sgrep-lsp-state-zblv.3` | Reapply dirty buffers after disk `index_all` | Fixed + tested |
| `ast-sgrep-lsp-state-zblv.4` | Non-blocking lock **or** truthful README | README rewritten to match blocking mutex |
| `ast-sgrep-x46g` | Doc sync / text edit must surface errors not wrong Ok | Fixed + tested |
| `ast-sgrep-ziij` (LSP parts) | Cross-surface parity — LSP error swallow via `x46g` | LSP portion fixed |
| Epic acceptance | No panic on blank lines (`zblv.1` regression still present in tree) | Fixed + tested |

Note: `.beads` was **not** modified in this session (per orchestrator). Tracker close/sync is left to the parent agent.

## File:line changes

### `ast-sgrep-lsp-state-zblv.2` — ready only after full `index_all`

- `crates/ast-sgrep-lsp/src/backend.rs:85-94` — `with_locked_indexer` no longer calls `record_index_result`; single-file ops do not touch `index_ready`.
- `crates/ast-sgrep-lsp/src/backend.rs:146-173` — `start_background_index` / `ensure_index` are the only writers of `index_ready`, set from full `run_full_index` success/failure.
- Removed the previous `record_index_result` helper entirely.

### `ast-sgrep-lsp-state-zblv.3` — dirty buffers survive disk reindex

- `crates/ast-sgrep-lsp/src/backend.rs:24-27,41` — `dirty_buffers: Arc<Mutex<HashMap<String, String>>>`.
- `crates/ast-sgrep-lsp/src/backend.rs:115-127` — `remember_dirty` / `forget_dirty`.
- `crates/ast-sgrep-lsp/src/backend.rs:129-144` — `run_full_index` runs disk `index_all` then re-applies every dirty buffer via `index_content`.
- `crates/ast-sgrep-lsp/src/backend.rs:187-214` — `index_content` / `apply_document_changes` remember dirty text under the index lock; `reindex_file` forgets after a successful on-disk reindex.

### `ast-sgrep-lsp-state-zblv.4` — truthful concurrency docs

- Prefer README rewrite over implementing non-blocking `try_lock` + `index_hold_p99`.
- `crates/ast-sgrep-lsp/README.md:11-13` — documents blocking `Mutex`, absence of busy/retry errors and `index_hold_p99`, dirty-buffer reapply, and `window/showMessage` on sync errors.
- Lock implementation remains `Mutex::lock` at `crates/ast-sgrep-lsp/src/backend.rs:78-83`.

### `ast-sgrep-x46g` / `ast-sgrep-ziij` (LSP)

- `crates/ast-sgrep-lsp/src/backend.rs:176-184` — `reindex_file` returns `Err` when the path is not a file (`file not found for reindex`), not bare `Ok(())`.
- `crates/ast-sgrep-lsp/src/support.rs:240-257` — `apply_text_edit` returns `anyhow::Result<String>`; inverted/invalid ranges bail with `invalid text edit range` instead of silently returning original content.
- `crates/ast-sgrep-lsp/src/server.rs:75-124,218-231` — `didOpen` / `didChange` / `didSave` no longer `let _ =` index errors; failures go to stderr and `window/showMessage` (type=1 Error).

### Empty-line navigation panic (epic acceptance / `zblv.1` residual)

- `crates/ast-sgrep-lsp/src/support.rs:296-310` — `ident_idx` returns `None` when `chars` is empty (blank line), preventing `chars[0]` panic.

## Verification commands

```bash
export PATH="/usr/local/cargo/bin:$PATH"
cd /workspace/.worktrees/pr14

# Focused LSP integration/regression suite (11 tests)
cargo test -p ast-sgrep-lsp --test lsp -- --nocapture
```

### Observed result (this session)

```
running 11 tests
test invalid_text_edit_range_returns_error ... ok
test malformed_regex_does_not_mark_healthy_index_unready ... ok
test blank_line_navigation_does_not_panic ... ok
test nonzero_range_length_replaces_correct_span ... ok
test pure_insertion_preserves_following_char ... ok
test dirty_buffer_survives_full_disk_reindex ... ok
test lsp_smoke ... ok
test missing_reindex_file_errors_without_clearing_ready ... ok
test single_file_index_does_not_mark_index_ready ... ok
test successful_read_does_not_heal_failed_index ... ok
test uppercase_symbol_resolves_through_definition_and_reference_endpoints ... ok

test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### Test → bead map

| Test | Bead |
|---|---|
| `single_file_index_does_not_mark_index_ready` | `zblv.2` |
| `missing_reindex_file_errors_without_clearing_ready` | `zblv.2` + `x46g` |
| `dirty_buffer_survives_full_disk_reindex` | `zblv.3` |
| `invalid_text_edit_range_returns_error` | `x46g` |
| `blank_line_navigation_does_not_panic` | epic / `zblv.1` residual |
| Existing readiness tests (`malformed_regex…`, `successful_read…`) | still green under new ready semantics |

## Hard-evidence claims (falsifiable)

1. After `index_content` alone, `is_index_ready()` stays `false` until `ensure_index` / background full index succeeds.
2. After a successful full index, a missing-file `reindex_file` returns `Err` and leaves `is_index_ready() == true`.
3. Unsaved buffer text indexed via `apply_document_changes` remains searchable after rewriting the on-disk file and calling `ensure_index`.
4. Inverted LSP edit ranges return `Err` containing `invalid text edit range`.
5. `extract_identifier_at("", 0)` is `None`; `goto_definition` on a blank line returns `no symbol at cursor` without panicking.
6. README no longer claims non-blocking lock / `index_hold_p99`; code still uses blocking `Mutex::lock`.
