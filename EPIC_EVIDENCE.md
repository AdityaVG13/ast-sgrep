# Epic evidence — P1 correctness / durability batch (PR #20)

Branch: `fix/p1-correctness-batch`  
Evidence date: 2026-08-03  
Primary test crate: `ast-sgrep-core` (`tests/durability_epics.rs` + existing semantic/response/store tests)

Run focused suite:

```bash
export CARGO_BUILD_RUSTC_WRAPPER= RUSTC_WRAPPER=
/usr/local/cargo/bin/cargo test -p ast-sgrep-core --test durability_epics --test semantic_cache_version --test semantic_ivf_roundtrip --test response_cache_version --test store_pragmas --test store_delete
/usr/local/cargo/bin/cargo test -p ast-sgrep-embed --lib math::contract_tests
```

### Observed results (2026-08-03)

| Suite | Result |
|-------|--------|
| `durability_epics` | **16 passed** |
| `response_cache_version` | **2 passed** |
| `semantic_cache_version` | **4 passed** |
| `semantic_ivf_roundtrip` | **3 passed** |
| `store_delete` | **8 passed** |
| `store_pragmas` | **1 passed** |
| `ast-sgrep-embed` `contract_tests` | **3 passed** |
---

## Epic `ast-sgrep-sgrep-store-durability-y1oy` (`ast-sgrep-y1oy`)

| Kid | Requirement | Implementation | Hard evidence |
|-----|-------------|----------------|---------------|
| `.3` | Write `semantic.ivf` atomically (tmp+fsync+rename) | `semantic_ivf::save_semantic_ivf` writes `*.ivf.tmp`, `sync_all`, `rename`, parent dir fsync | `durability_epics::semantic_ivf_save_is_atomic_tmp_rename` |
| `.4` | Never treat empty `lexical.db` as ready | `TantivySidecar::is_search_ready` requires `meta.lines > 0`; search uses `open_existing_for_search` (no create, rejects empty/zero-byte) | `durability_epics::empty_lexical_db_is_not_search_ready` |
| `.5` | `clear_all_data` transactional + complete | Wrapped in `with_file_tx`; clears content tables, `embed_cache`, `struct:`/`body:`/`eol:` meta; VACUUM; IVF stale; bumps generations | `durability_epics::clear_all_data_is_transactional_and_complete` |
| `.6` | Atomic `remove_file`; delete struct/body meta; safe IVF stale | `remove_file` in `with_file_tx`; deletes `eol:`/`struct:`/`body:`; `mark_semantic_ivf_stale` propagates errors | `durability_epics::remove_file_deletes_struct_body_meta_and_ivf`, `remove_file_clears_graph_rows` |
| `.8` | `--lang` must not wipe other languages | Filtered paths skipped (no delete); `prune_missing_files` lang-scoped; `language_filter_allows` non-destructive | `durability_epics::lang_filter_index_does_not_wipe_other_languages` |

---

## Epic `ast-sgrep-semantic-cache-jiyy`

| Kid | Requirement | Implementation | Hard evidence |
|-----|-------------|----------------|---------------|
| `.2` | Bind IVF fingerprint to vector/content identity | Fingerprint domain `asgrep-semantic-ivf-v2` includes `semantic_data_version` generation; helpers `vectors_content_digest` / `compute_ann_fingerprint_with_content` | `durability_epics::ivf_fingerprint_binds_generation_and_content`; `semantic_cache_version::*` |
| `.3` | Invalidate SemanticCache on content/count/version change | Cache identity: lang + max_id + backend + `semantic_data_version`; bumps on insert/remove/clear/empty re-upsert | `semantic_cache_version.rs` (insert/remove/readd, clear, empty re-upsert) |
| `.4` | Fail closed when ResponseCache cannot read `data_version` | `Searcher::index_gen` returns `Result` (no `unwrap_or(0)`); `cached` propagates errors | `response_cache_version.rs`; `durability_epics::hybrid_response_cache_invalidates_on_index_generation` |
| `.5` | Unify cosine/threshold paths; fail closed on corrupt embeddings | `top_by_similarity` uses same ULP `exceeds_threshold` as `top_k_*`; `emb_vec` / by-id readers error on corrupt blobs | `durability_epics::cosine_threshold_paths_are_unified`, `corrupt_embedding_blob_fails_closed`; embed `math::contract_tests` |

---

## Epic `ast-sgrep-j97d` (SQLite durability)

| Kid | Requirement | Implementation | Hard evidence |
|-----|-------------|----------------|---------------|
| `5kj8` | Restore `PRAGMA synchronous` after file_tx and bulk rollback | `restore_synchronous` on every `end_file_tx` (owning) and `end_bulk_tx` (commit **and** rollback) | `durability_epics::synchronous_restored_after_file_tx_and_bulk_rollback` |
| `37er` | Nested `with_file_tx` must not commit outer on inner error | Depth + owns + poisoned flags; nested commit is depth-only; nested rollback poisons; outer commit refuses | `durability_epics::nested_file_tx_inner_error_rolls_back_outer` |
| `3ddd` | Propagate index body-hash `set_meta` failures | `index.rs` uses `?` on `body:{path}` meta writes (index_all + index_file) | `durability_epics::body_hash_meta_persisted_after_index` |
| `5qpa` | Reject corrupt embedding blobs | `sql::emb_vec` and `semantic_chunks_by_ids` map `embed_from_bytes` errors (no zero default) | `durability_epics::corrupt_embedding_blob_fails_closed` |
| `045r` | Allowlist SQL dynamic column/table identifiers | `assert_sql_ident` + allowlists for caller columns, count tables, file-child deletes | `durability_epics::sql_identifier_allowlist_rejects_unknown` |

---

## Epic `ast-sgrep-ht1h` (Index/IVF/hybrid snapshot consistency)

| Kid | Requirement | Implementation | Hard evidence |
|-----|-------------|----------------|---------------|
| `.1` | Audit doc of consistency model | `docs/index-consistency.md` (+ links from ARCHITECTURE / how-it-works / docs README) | Doc present; cited by this evidence file |
| `.2` | Strengthen ANN/IVF fingerprint with content generation counter | `semantic_data_version` hashed as `gen` in v2 fingerprint | `ivf_fingerprint_binds_generation_and_content`; `semantic_cache_version` |
| `.3` | Hybrid cache key / docs for concurrent reindex | ResponseCache `(PRAGMA data_version, index_data_version)`; documented in `index-consistency.md` | `hybrid_response_cache_invalidates_on_index_generation`; `response_cache_version.rs` |
| `.4` | Targeted IVF fingerprint invalidation unit tests | Generation + content-digest unit tests; existing roundtrip fingerprint gate | `ivf_fingerprint_binds_generation_and_content`; `semantic_ivf_roundtrip` |
| `.5` | Docs for hybrid multi-connection guarantees | WAL / busy_timeout / sync restore / nested tx / integrity section in `index-consistency.md` | Doc + `open_sets_busy_timeout_and_normal_sync` |

---

## Epic `ast-sgrep-esyi` durability kids

| Kid | Requirement | Implementation | Hard evidence |
|-----|-------------|----------------|---------------|
| `esyi.3` | Index open: integrity_check + corruption repair/fail path | Existing DB: `PRAGMA integrity_check`; on failure quarantine to `index.db.corrupt` and error | `durability_epics::open_integrity_check_quarantines_corrupt_db` |
| `esyi.4` | Document/guard concurrent writers + busy_timeout + SYNC OFF bulk | `configure_connection` busy_timeout=5s, NORMAL sync; bulk/file_tx OFF only inside tx then restore; docs | `open_sets_busy_timeout_and_normal_sync`; `store_pragmas`; `index-consistency.md` |

---

## Notes

- `.beads` was **not** modified (per task instructions).
- Fingerprint domain bump to `asgrep-semantic-ivf-v2` intentionally invalidates older on-disk IVF sidecars (rebuild on next ANN use).
