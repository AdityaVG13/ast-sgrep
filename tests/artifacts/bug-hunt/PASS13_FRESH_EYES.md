# Pass 13 — Fresh-eyes on fixed files

**Date:** 2026-08-07  
**Scope:** Re-read files this bug-hunt loop fixed, looking for **new** bugs introduced by those fixes (not re-litigating pre-existing design).  
**Verdict:** **CLEAN** — no new real correctness bugs found; no beads filed.

Prior pass notes consulted: `PASS1_SURFACE.md`, `PASS4_INTEGRATION.md`, `PASS5_SEARCH.md`, `PASS8_INTEGRATION.md`, `PASS11_CONCURRENCY.md`.

## Re-reads (≥5) — why the fix is still correct

### 1. `search/types.rs` + `search/mod.rs` — confidence assignment (pass5 / d2a1.7–.8)

**What was fixed:** `dedup=false` left `confidence` at 0.0; multi-channel merge could inflate trust from display `signal` after margins; JSON round-trip dropped confidence.

**Re-read findings:**
- `finish_response_checked` always runs `assign_signal_margins` then `assign_hit_confidence` (including `dedup=false`).
- `estimate_confidence` bases strength on **strongest contributor + kind**, not post-margin display `signal`.
- Wire deserialize: non-finite confidence → 0.0; missing field → 0.0; contributors intentionally rebuilt from `kind` (untrusted wire evidence discarded; confidence float preserved).
- `dedup_hits` assigns confidence, then finish re-assigns after margins — order is correct, not double-wrong.

**Why not a regression:** unit tests pin strongest-contributor math and semantic-only nonzero without dedup. No new path skips confidence.

### 2. `search/passes/embed.rs` — query embed cache poison (pass11)

**What was fixed:** `if let Ok(lock)` left `QUERY_EMBED_CACHE` permanently dead after poison.

**Re-read findings:**
- `lock_clear_on_poison` clears poison + empties the map before reuse (fail-closed; no half-written vectors reused).
- Cache key binds query + backend + model + dim + preference.
- Insert still caps at 64 without LRU eviction (pre-existing capacity policy, not introduced by poison recovery).

**Why not a regression:** recovery clears untrusted state rather than reusing poisoned map contents; test `query_embed_cache_poison_recovers_fail_closed` green.

### 3. `store/sqlite.rs` + `index.rs` — bulk rollback / restore_synchronous (d2a1.2 residual / pass9)

**What was fixed:** `let _ = rollback_bulk_tx()` could hide stuck `synchronous=OFF` after write failure.

**Re-read findings:**
- `apply_bulk_write_result`: on write Err, prefer `rollback_bulk_tx`/restore error over original write Err when restore fails.
- `end_bulk_tx` / `end_file_tx` always call `restore_synchronous` when owning the write set; errors propagate.
- Nested `with_file_tx` poison: Ok-closure after nested rollback returns Err (not silent success).
- `clear_all_data` bumps both gens inside the same `with_file_tx` as the wipe.

**Why not a regression:** fail-visible durability is intentional; successful commit still rebuilds sidecars on the index_all happy path. Tests cover commit/rollback restore inject and prefer-restore-over-write.

### 4. `semantic_ann.rs` + `embed/math.rs` — MIN_SIMILARITY gate + zero-dim (pass / 8049042, e62ca79)

**What was fixed:** IVF member scoring used plain `sim > MIN` (ULP-incoherent vs flat); query re-gated on `DEFAULT_ANN_THRESHOLD` so lowered ANN thresholds never took effect; `dim=0` could panic on `/ dim`.

**Re-read findings:**
- `score_members` routes through `top_k_similarity(..., Some(MIN_SIMILARITY))` → ULP-stable `exceeds_threshold`.
- Query path uses IVF when **centroids exist**; eligibility remains build-time `should_use_ann` in `load_or_build` / `cached_semantic_ivf` (not silent ANN on every small corpus).
- `top_k_flat_similarity` uses `checked_div` + empty return on dim/limit/n=0.
- `flatten_vectors_for_search` rejects dim=0 with chunks and `checked_mul` before allocation.

**Why not a regression:** mid-size IVF with all probes matches flat in tests; 1ulp/2ulp boundary fixtures match both paths.

### 5. `cli/supervisor.rs` — worker nonce (pass1 / d2a1.1)

**What was fixed:** ignored `read_exact` left an all-zero buffer that still passed the hex-shape auth check.

**Re-read findings:**
- `generate_worker_nonce` requires successful `read_exact`; all-zero buffer forces `fill_nonce_fallback`.
- Auth still requires ≥32 hex digits + parent pid + exe check (macOS `ps` basename best-effort under `unsafe_code = forbid`).
- Fallback mixes pid/time/thread — not a fixed zero token.

**Why not a regression:** test requires 32 hex, not all-zero, successive draws differ.

### 6. `cli/machine.rs` + `cli/lib.rs` — broken pipe, batch caps, `--format` scope

**What was fixed:** agents piping JSON hit BrokenPipe panics; unbounded batch stdin; `--format` silently ignored on index.

**Re-read findings:**
- `write_line` maps BrokenPipe → Ok; other IO errors propagate.
- `read_utf8_capped` takes `max+1` bytes and rejects oversize without unbounded alloc; file path re-caps after stat (TOCTOU growth).
- `--format` allowed only for default/search/keyword/semantic (not index/reindex/bench/chain) — matches error text.

**Why not a regression:** unit tests for broken pipe, oversize reject, and machine detection.

### 7. `cli/index_cmd.rs` — dry-run `walk_errors`

**What was fixed:** dry-run under-count on permission/IO was silent.

**Re-read findings:**
- Walk sets `walk_errors` on `read_dir`/`file_type`/`entry` failures; machine JSON includes the flag; human path prints a warning when not `--json`.
- Dry-run extension set is intentionally broader/narrower than full index (documented product set).

**Why not a regression:** flag is observational; does not mutate index.

### 8. `mcp/lib.rs` — searcher invalidation + structuredContent + protocol negotiate

**What was fixed:** soft deadline after disk mutation could skip cache invalidation; parse errors omitted `id: null`.

**Re-read findings:**
- `tool_index_repo` always advances generation / clears path registry + elisions **before** post-work deadline check.
- Warm searcher restore only if generation still matches (in-flight reindex cannot reinstall stale Searcher).
- Parse / oversize line errors use `id: null` (JSON-RPC 2.0).
- `structuredContent` only when tool body is valid JSON; text content always present.
- Protocol negotiate: known client revision echoed; unknown → current `2026-07-28`.

**Why not a regression:** cache tests green for generation fence + post-index invalidation.

### 9. `lsp/{backend,server,main}.rs` — shutdown/exit + dirty poison

**What was fixed:** shutdown exited the loop early; dirty_buffers poison bricked doc sync.

**Re-read findings:**
- `shutdown_received` keeps the loop alive; post-shutdown requests get `-32600` until `exit`.
- `process_exit_code`: exit without prior shutdown → 1; clean path → 0.
- Dirty map poison: `clear_poison` + clear map (fail-closed), same pattern as `index_lock`.
- Background index and doc ops share `index_lock`; dirty re-apply after disk index while still holding the lock.

**Why not a regression:** lifecycle + dirty poison unit tests green.

### 10. Dual FTS `lines_code_fts` (8e8e809 / 28b50e4 SCHEMA_DDL)

**What was fixed:** SQL `--` comments inside line-continued `SCHEMA_DDL` ate the rest of the batch (dropped FTS/embed tables). Code/prose dual field added with schema 8 backfill.

**Re-read findings:**
- `SCHEMA_DDL` has no `--`; contains `lines_code_fts`, `embeddings`, `semantic_chunks` (verified by decoding the constant).
- Insert/delete/clear paths keep `lines_code_fts` in lockstep with `lines_fts` (`insert_lines`, `delete_file_lines`, `CLEAR_ALL_SQL`).
- Schema 8 backfill: `DELETE` + `INSERT ... SELECT` from `lines`.
- `query_is_code_like` is deliberately conservative (long hybrid NL stays on porter); identifier shape uses code field.

**Why not a regression:** insert/delete symmetry is complete; analyzer choice is documented product policy with integration tests under `code_prose_fields`.

### 11. `bench_suite.rs` — `percentile_99` empty (d2a1.3)

**Re-read:** empty → 0 without panic; index uses `saturating_mul`/`div_ceil`/`min(len-1)`. Call sites remain non-empty sample windows. Correct.

### 12. `embedder.rs` — no redirects + API key redaction

**Re-read:** `embed_http_agent` uses `redirects(0)` so allowlist is hop-final; `CloudEmbeddingConfig` Debug redacts key. Correct vs SSRF-via-redirect residual.

## Residual notes (not filed as new bugs)

| Item | Why not a bead |
|------|----------------|
| QCACHE cap 64 with no LRU when full | Pre-existing capacity policy; poison fix does not worsen it |
| Confidence deserialize accepts any finite f64 (not clamped to [0, 0.99]) | Incomplete sanitization only; production assign clamps; agents rarely re-parse and re-trust wire confidence alone |
| `query_is_code_like` word-count >3 forces prose | Documented conservative heuristic for dual FTS; not a fix regression |
| `COUNT_TABLE_ALLOWLIST` omits `lines_code_fts` | Only used by `count_star` on allowlisted tables; status does not count that table |
| LSP `index_ready` set after releasing `index_lock` | Advisory flag lag; search paths do not gate on it (PASS11 residual) |

## Verification evidence (this pass)

Isolated target: `CARGO_TARGET_DIR=target-pass13`.

| Filter | Result |
|--------|--------|
| `ast-sgrep-core --lib confidence` | 6 ok |
| `ast-sgrep-core --lib query_embed_cache` | 1 ok |
| `ast-sgrep-core --lib percentile_99` | 3 ok |
| `ast-sgrep-core --lib restore_synchronous` | 6 ok |
| `ast-sgrep-core --lib min_similarity` | 3 ok |
| `ast-sgrep-cli --lib worker_nonce` | 1 ok |
| `ast-sgrep-cli --lib machine::tests` | 6 ok |
| `ast-sgrep-lsp --lib lifecycle` | 4 ok |
| `ast-sgrep-lsp --lib dirty_buffers` | 1 ok |
| `ast-sgrep-mcp --lib cache_tests` | 2 ok |

**Totals:** 33 passed, 0 failed.

## Beads

None filed (no new confirmed correctness bugs).

## Status

**PASS13 CLEAN** — fresh-eyes re-read of fix surfaces; no new product bugs; smoke of fix-related unit tests green. No commit (per mission).
