# Index consistency model

This document is the audit record for epics `ast-sgrep-ht1h` and `ast-sgrep-esyi.4`: what a searcher may observe under concurrent reindex, how IVF and hybrid caches invalidate, and what SQLite durability settings guarantee.

## Source of truth

| Layer | Role |
|---|---|
| `.asgrep/index.db` (SQLite, WAL) | Authoritative indexed facts: files, lines, symbols, callers, imports, semantic_chunks |
| `.asgrep/lexical.db` | Derived FTS sidecar; used only when `meta.lines > 0` (never when empty) |
| `.asgrep/semantic.ivf` | Derived ANN sidecar; accepted only when fingerprint matches |

Sidecars are never independent sources of truth. A fingerprint or readiness miss falls back to SQLite (flat cosine / in-DB FTS).

## Generations and fingerprints

1. **`index_data_version`** (meta) — monotonic counter bumped on every searchable-index mutation on any connection. Used by `ResponseCache` together with `PRAGMA data_version` (other-connection commits).
2. **`semantic_data_version`** (meta) — content-generation counter bumped on every `semantic_chunks` mutation. Included in the IVF ANN fingerprint and `SemanticCache` identity.
3. **IVF fingerprint** — `blake3("asgrep-semantic-ivf-v2" ‖ count ‖ max_id ‖ dim ‖ backend ‖ "gen" ‖ semantic_data_version)`. A delete/re-add that reuses `max_id` still changes the generation and forces rebuild.

Optional helper `vectors_content_digest` / `compute_ann_fingerprint_with_content` bind fingerprints to raw vector bytes in tests and tooling; the on-disk gate uses the generation-backed fingerprint so lazy ANN and rebuild paths agree.

## Hybrid cache keys (concurrent reindex)

`ResponseCache` keys responses by `(kind, query)` under generation `(PRAGMA data_version, index_data_version)`.

- If either generation cannot be read, caching **fails closed** (error; no stale hit as generation 0).
- Same-connection writes bump `index_data_version` even when `PRAGMA data_version` is unchanged.
- External writers bump `PRAGMA data_version` after commit; readers observe the new generation on the next query.

Under concurrent reindex, a hybrid query may fuse passes from adjacent **committed** snapshots. It must not serve a response cached against a prior generation. Semantic hits additionally require `SemanticCache` lang/max_id/backend/`semantic_data_version` identity; IVF hits require fingerprint match or flat fallback.

## Multi-connection guarantees

| Setting | Value | Intent |
|---|---|---|
| `journal_mode` | WAL | Readers proceed during writers |
| `synchronous` | NORMAL (restored after every file_tx / bulk end, including rollback) | Durable commits without FULL fsync cost |
| `busy_timeout` | 5000 ms | Writers wait briefly instead of immediate `SQLITE_BUSY` |
| Bulk / file transactions | `synchronous=OFF` only inside the open write tx | Throughput during index; always restored afterward |
| Nested `with_file_tx` | Depth-tracked; inner rollback poisons outer | Inner error must not commit outer work |

**Concurrent writers:** supported at the SQLite level via WAL + busy timeout. Application-level indexing should prefer a single indexer process per index path; two bulk indexers on one DB will serialize on `BEGIN IMMEDIATE` and may contend. Searchers may open additional read connections safely.

**Integrity on open:** existing DBs run `PRAGMA integrity_check`. Failure quarantines the file to `index.db.corrupt` and returns an error (fail closed; reindex required).

## Atomic sidecar publish

`semantic.ivf` is written to `*.ivf.tmp`, `fsync`ed, then `rename`d into place. Readers either see the previous complete file or the new complete file, never a torn write.

## Language-filtered index

`Indexer` with `--lang` / `lang_filter` updates only matching languages. Filtered paths are skipped; prune-missing only removes absent files for the filtered language. Other languages remain searchable.
