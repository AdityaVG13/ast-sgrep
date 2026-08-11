# Pass 4 — before/after CC

Analyzer: `scripts/measure_complexity.py` (lizard)  
Measure JSON: `.cyclomatic-reduction/runs/2026-08-10T235424Z-baseline/06-transformed-code/pass4-{before,after}/`

## Target functions

| Function | File | Before CC | After CC | Δ |
|---|---|---:|---:|---:|
| `index_all` | `crates/ast-sgrep-core/src/index.rs` | 23 | 8 | −15 |
| `index_content_at` | same | 20 | 13 | −7 |
| `delete_file_lines` | `crates/ast-sgrep-core/src/store/sql.rs` | 18 | 2 | −16 |
| `run_codemode_batch` | `crates/ast-sgrep-cli/src/lib.rs` | 19 | 7 | −12 |
| `parseSearchHit` | `packages/pi/extension/src/code-mode.ts` | 21 | 4 | −17 |
| `read_node` | `crates/ast-sgrep-mcp/src/lib.rs` | 20 | 10 | −10 |

## New helpers

| Function | File | After CC |
|---|---|---:|
| `commit_prepared_files` | core `index.rs` | 8 |
| `post_index_hooks` | core `index.rs` | 9 |
| `try_structure_skip_refresh` | core `index.rs` | 9 |
| `exec_line_deletes` | core `store/sql.rs` | 4 |
| `load_batch_raw` | cli `lib.rs` | 7 |
| `apply_cli_batch_defaults` | cli `lib.rs` | 6 |
| `isValidHitShape` | extension `code-mode.ts` | 18 |
| `scan_line_window` | mcp `lib.rs` | 12 |

## Scope ΣCC

| Scope | Before | After | Δ |
|---|---:|---:|---:|
| core `index.rs` | 242 | 246 | +4 |
| core `store/sql.rs` | 88 | 76 | −12 |
| cli `lib.rs` | 61 | 62 | +1 |
| mcp `lib.rs` | 181 | 183 | +2 |
| extension `code-mode.ts` | 144 | 145 | +1 |
| **combined touched** | **716** | **712** | **−4** |

Repo baseline ΣCC **6022** not re-scanned this wave.
