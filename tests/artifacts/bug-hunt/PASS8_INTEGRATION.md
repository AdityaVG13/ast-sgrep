# Pass 8 — Integration mid-loop

**Date:** 2026-08-07  
**Scope:** Re-test all bug-hunt surfaces fixed so far; fix regressions if any.

Isolated target dir: `CARGO_TARGET_DIR=target-pass8` (avoids concurrent workspace cargo lock contention).

Full log: `tests/artifacts/bug-hunt/PASS8_test_run.log`

## Commands + results

| Command | Result |
|---|---|
| `cargo test -p ast-sgrep-core --lib confidence` | **ok** 4 passed |
| `cargo test -p ast-sgrep-core --lib pass3_deep_core` | **ok** 2 passed |
| `cargo test -p ast-sgrep-core --lib restore_synchronous` | **ok** 4 passed |
| `cargo test -p ast-sgrep-cli --lib worker_nonce` | **ok** 1 passed |
| `cargo test -p ast-sgrep-cli --lib machine::tests` | **ok** 4 passed |
| `cargo test -p ast-sgrep-cli --test machine_contracts` | **ok** 20 passed |
| `cargo test -p ast-sgrep-mcp --lib cache_tests` | **ok** 2 passed |
| `cargo test -p ast-sgrep-lsp --lib` | **ok** 4 passed |
| `cargo test -p ast-sgrep-core --test durability_epics` | **ok** 19 passed |

**Totals for this gate:** 60 passed, 0 failed, 0 ignored.

### Surface detail

1. **confidence** (`ast-sgrep-core` lib)  
   - empty hits noop; semantic-only nonzero without dedup; strongest contributor vs display signal; `finish_response` assigns when `dedup=false`.

2. **pass3_deep_core** (`store::sqlite`)  
   - `with_file_tx` poisoned-Ok-closure → Err; `semantic_chunks_by_ids` fails closed on corrupt blob.

3. **restore_synchronous** (`store::sqlite`)  
   - bulk/file tx commit and rollback all surface restore-synchronous failure.

4. **worker_nonce** (`ast-sgrep-cli` supervisor)  
   - 32 hex chars, not all-zero.

5. **machine::tests** (`ast-sgrep-cli`)  
   - raw machine false for plain search; codemode batch without `--json`; `read_utf8_capped` at/over limit.

6. **machine_contracts** (`ast-sgrep-cli` integration)  
   - envelopes, doctor/index/search shapes, edit-distance typos, dry-run, oversized stdin/file, agent modes, bench suite single envelope.

7. **cache_tests** (`ast-sgrep-mcp`)  
   - reindex generation rejects in-flight stale searcher; `index_repo` invalidates after disk mutation.

8. **lsp lib** (`ast-sgrep-lsp`)  
   - limit remap/cap; exit-without-shutdown → code 1; shutdown stays up until exit; dirty-buffers poison recovers fail-closed (intentional panic in poison path; test still **ok**).

9. **durability_epics** (`ast-sgrep-core` integration)  
   - 19 tests (was 18 in Pass 4): gen fence + hybrid response cache, IVF stale/fingerprint/atomic save, `clear_all_data`, remove_file/graph, body hash, integrity quarantine, nested file_tx, SQL allowlist, busy timeout, etc.

## Regressions

**None.** No compile breaks, no failing tests. No code changes in this pass.

## Beads

None filed (no new confirmed bugs).

## Notes

- LSP dirty-lock poison test prints `intentional dirty lock poison` on stderr; that is expected harness output, not a failure.
- `durability_epics` count grew from 18 (Pass 4) to 19; all green.
- No commit (per mission).
