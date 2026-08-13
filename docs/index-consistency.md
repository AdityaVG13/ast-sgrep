# Index consistency model

This document is the audit record for epics `ast-sgrep-ht1h` and `ast-sgrep-esyi.4`: what a searcher may observe under concurrent reindex, how IVF and hybrid caches invalidate, and what SQLite durability settings guarantee.

## Source of truth

| Layer | Role |
|---|---|
| `.asgrep/index.db` (SQLite, WAL) | Authoritative indexed facts: files, lines, symbols, callers, imports, semantic_chunks |
| `.asgrep/lexical.db` | Derived FTS sidecar; used only when `meta.lines > 0` (never when empty) |
| `.asgrep/semantic.ivf` | Derived ANN sidecar; accepted only when fingerprint matches |

Sidecars are never independent sources of truth. A fingerprint or readiness miss falls back to SQLite (flat cosine / in-DB FTS).

Source reads on supported platforms are rooted to an open project-directory
capability: Unix walks components with descriptor-relative `openat` and
`NOFOLLOW`; Windows uses `cap-std` handle-relative resolution and refuses a
symlink/reparse-point leaf. Both reject non-relative indexed paths and re-check
that the opened object is a bounded regular file. Unsupported non-Unix,
non-Windows targets use canonicalization plus containment before open; that
fallback rejects static escapes but does not provide the same rename-race
guarantee and is not part of the published platform matrix.

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

Hybrid search holds one SQLite read transaction across every pass, so a response
cannot fuse rows from adjacent committed snapshots. Snapshot setup, generation
lookup, and commit failures fail the query closed. Semantic hits additionally
require `SemanticCache` lang/max_id/backend/`semantic_data_version` identity;
IVF hits require fingerprint match or flat fallback.

## Multi-connection guarantees

| Setting | Value | Intent |
|---|---|---|
| `journal_mode` | WAL | Readers proceed during writers |
| steady `synchronous` | FULL for `strict`; NORMAL for `balanced` (default) and `fast-unsafe` | Selected durability outside write transactions |
| `busy_timeout` | 5000 ms | Writers wait briefly instead of immediate `SQLITE_BUSY` |
| Bulk / file transactions | FULL for `strict`; NORMAL for `balanced`; OFF only for explicitly selected `fast-unsafe` | Always restored to the steady profile after commit, rollback, or failed admission |
| Nested `with_file_tx` | Depth-tracked; inner rollback poisons outer | Inner error must not commit outer work |

**Concurrent writers:** supported at the SQLite level via WAL + busy timeout. Application-level indexing should prefer a single indexer process per index path; two bulk indexers on one DB will serialize on `BEGIN IMMEDIATE` and may contend. Searchers may open additional read connections safely.

**Cross-process Searcher caches (R-XPROC-MULTIWRITER Option C lite):** writers (`Indexer::index_all`, watch `update_paths` / deferred sidecar flush) publish a unique `writer_generation` epoch beside the index home (`.asgrep/writer_generation`, or next to a pinned `ASGREP_INDEX_PATH`). The stamp is not `read+1`: concurrent writers must not publish the same value, or a peer that already observed it will skip the second mutation. Long-lived MCP and Code Mode Searcher caches poll the stamp for the **cached Searcher's root** (the per-call index, not the session workspace) and reopen when it changes, so `asgrep watch` / CLI index cannot silently leave a warm peer serving a pre-mutation snapshot. This is an epoch poll, not a flock or IPC bus. `asgrep status` reports `writer_generation`.

**Writer-generation fail-open (intentional contract):**

| Surface | Contract |
|---|---|
| `advertise_writer_generation` | Best-effort after a successful SQLite commit. Stamp I/O failure logs and **skips**; it must **not** fail the index command. Durable rows may already be visible to other connections; turning a committed mutation into a command error is worse than a missed peer reopen hint. |
| `read_writer_generation` | Returns `0` when the stamp is absent or unreadable. That `0` is the **first-run / cold-start** protocol (no stamp yet), not a soft error to fail closed on. Peers treat epoch `0` as "no advertised external writer yet." |

Residual risk of advertise skip: a warm peer that already cached under epoch `0` may not reopen until a later successful bump (or its own cache invalidation). Accepted trade-off versus fail-closed-after-commit. Observability: skipped bumps print `asgrep: writer_generation stamp skipped ...` with the stamp path.

**Pinned `ASGREP_INDEX_PATH`:** an explicit `--index-path` / `IndexOptions.index_path` / `ASGREP_INDEX_PATH` pins a specific DB file. Prefer one shared path for MCP + CLI watch so the SQLite file and `writer_generation` stamp stay co-located.

**Corruption recovery:** ordinary opens fail closed when SQLite reports a corrupt
or non-database file. Explicit `reindex` additionally runs bounded
`PRAGMA quick_check(1)`; a failed check preserves the database and its WAL/SHM
or rollback-journal sidecars under a unique `index.db.corrupt[.N]` name before
creating a replacement. An adjacent advisory lock serializes cooperating
recovery attempts, and hard-link admission never overwrites an earlier recovery
copy. Derived lexical/ANN sidecars are invalidated first, and replacement
generation counters receive a fresh high seed so a sidecar retained by another
process cannot pass an identity check by coincidence. If the authoritative DB
cannot be preserved, reindex also fails closed.

## Atomic sidecar publish

`semantic.ivf` is written to `*.ivf.tmp`, `fsync`ed, then `rename`d into place. Readers either see the previous complete file or the new complete file, never a torn write.

## Language-filtered index

`Indexer` with `--lang` / `lang_filter` updates only matching languages. Filtered paths are skipped; prune-missing only removes absent files for the filtered language. Other languages remain searchable.

## Non-UTF8 indexed paths (ast-sgrep-kqhp)

Relative paths stored in `files.path` (and therefore all graph/meta keys derived from them) must be valid UTF-8. Index walks and watch updates call `indexed_rel_path`, which **rejects** non-UTF8 `OsStr` components with a machine-readable error (`non-UTF8 path rejected (asgrep-kqhp)`). Lossy `to_string_lossy` conversion is not used for indexed keys, so two distinct non-UTF8 paths cannot collide into one DB row.
