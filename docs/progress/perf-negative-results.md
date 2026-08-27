# Performance negative results

Campaign ledger for perf ideas that were measured and rejected, or that must
not be closed as green without artifacts. Check before a new optimization pass.

Skill headers: gauntlet WP3 / K-3. Predicate forms: `docs/progress/README.md`.

**Closed:** empty on seed. Do not invent keep-gate closes.

## Closed

### `gauntlet-2026-08-26-caller-subquery-id-list` (REJECTED 2026-08-26 — raw-SQL A/B, not shipped)

- **date:** 2026-08-26
- **candidate_name:** `caller-file-restriction-via-id-list`
- **target_workload:** warm distinct hybrid search through codemode-serve, self corpus (545 indexed files); symbol_pass_for_files caller-rows SQL (sampler: 38.8% of worker time; likeFunc 9%, TEXT materialization trio ~13%)
- **files_touched:** prototype patched and reverted (`crates/ast-sgrep-core/src/search/passes/symbol.rs` restrict_to_files); no shipped change
- **correctness_proof:** row sets byte-identical in raw-SQL A/B on an index copy (both variants); golden battery captured separately for the L2 lever in the same session
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/sql_probe2.py` (interleaved microbench), `/tmp/asgrep-bench/flames_g0.txt` (8 s worker sample), `/tmp/asgrep-bench/probe_g.db`
- **baseline_configuration:** `AND f.path IN (?…)` after the LIKE OR-filter; macOS arm64 M5 Max, release-perf, HEAD `8bc467cb`, warm distinct p50 ~1.9 ms
- **candidate_configuration:** (a) `AND f.id IN (SELECT id FROM files WHERE path IN (?…))` — subquery form measured SLOWER (2.5 vs 1.9 ms standalone); (b) pre-resolved integer `c.file_id IN (?…)` with one id-resolution probe — within noise (+0.1 ms on a deliberately inflated 100-path probe)
- **measured_result:** not keeps. The planner already drives from idx_callers_file_id via the join; the LIKE evaluation dominates over path-probe overhead at this corpus shape.
- **retry_condition_predicate:** Reopen ONLY if a profiler attributes >=10% of warm-path time to `sqlite3BtreeMove`/rowid-probe frames inside the caller query on a corpus whose allowed_files set exceeds ~1000 paths per query (form 3 + form 4: corpus-shape-gated).
- **bead_id:** (none)

### `gauntlet-2026-08-26-trigram-survivor-identity` (MEASURED NEUTRAL — reverted before commit)

- **date:** 2026-08-26
- **candidate_name:** `trigram-scan-deferred-identity-resolution`
- **target_workload:** literal_trigram_scan span = 11.7% of wall on warm distinct queries; rusqlite Rows streaming = 21.9% of worker samples (path/language TEXT materialization for rejected postings)
- **files_touched:** prototype landed, verified, measured, reverted (`crates/ast-sgrep-core/src/search/passes/literal.rs` scan_trigram_matches)
- **correctness_proof:** 35/35 golden battery byte-identical between asgrep_g0 (base) and asgrep_g1 (lever), same-session index
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/golden_g0/manifest.json`, interleaved bench rounds in session log; sampler `flames_g0.txt`
- **baseline_configuration:** joined stream `SELECT f.path, f.language, l.line_no, l.content … JOIN files`; p50 {1.99,1.83,1.84,1.85} across 4 interleaved rounds
- **candidate_configuration:** stream `(file_id, line_no, content)` unjoined; HashMap-memoized per-file identity resolution only for rows passing content_matches_literal; matches_lang moved after reverify (filters commute)
- **measured_result:** p50 {1.93,1.81,1.84,1.82} — deltas within run-to-run noise; only p10 improved consistently (~5%). Root cause: `l.content` must materialize per posting regardless (the reverify reads it); path/language were the minor slice of valueToText frames.
- **retry_condition_predicate:** Reopen ONLY when a profiler attributes >=5% of worker time specifically to `files`-table row materialization (not `lines.content`) under literal scans — e.g., if excerpt/preview handling starts copying full file identity per posting (form 3).
- **bead_id:** (none)

### `gauntlet-2026-08-26-like-prelowered-bind` (WITHIN NOISE — reverted before commit)

- **date:** 2026-08-26
- **candidate_name:** `or-like-prelowered-pattern-bind`
- **target_workload:** same caller/symbol LIKE chain as above; two lower() evaluations per candidate row
- **files_touched:** prototype patched and reverted (`crates/ast-sgrep-core/src/store/sql.rs` or_like_filter)
- **correctness_proof:** 35/35 goldens byte-identical (pattern mirrors SQLite ASCII-only lower() exactly); all consumers bind-only, verified by grep
- **evidence_artifacts_paths:** interleaved rounds in session log; `flames_g0.txt`
- **baseline_configuration:** `'%' || lower(?) || '%'` per-row expression; p50 {1.76,1.75,1.70,1.69}
- **candidate_configuration:** fully pre-lowered `%term%` pattern bound once per query; p50 {1.77,1.74,1.94,1.62} — median-equal, wider spread
- **measured_result:** within noise; below keep-gate threshold.
- **retry_condition_predicate:** Reopen ONLY if lower() appears >=8% in a flame profile of the caller query on some corpus (it was <2% here) (form 3).
- **bead_id:** (none)

### `gauntlet-2026-08-26-index-write-shaping` (REJECTED — probes only, index surface untouched)

- **date:** 2026-08-26
- **candidate_name:** `cold-index-write-phase-shaping`
- **target_workload:** cold full index build, self corpus: 1.448 s ± 0.039 s (hyperfine, 5 runs); sqlite_upsert span = 66% of wall (959 ms), walk+parse = 34%; FTS5 maintenance dominates upsert (fts5UpdateMethod 25.8% incl. trigram tokenize 10.5%)
- **files_touched:** `no-source-patch-attempted` (raw-SQL probes on schema clones)
- **correctness_proof:** posting-set equality verified between fill strategies (MATCH counts identical for probe terms); multi-VALUES probe abandoned before any integration
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/{batch_probe,tri_probe}.py`, `/tmp/asgrep-bench/sample_index.py`, `/tmp/asgrep-bench/flames_idx.txt`, idx_spans.jsonl
- **baseline_configuration:** per-line loop of 4 prepared INSERTs (lines, lines_fts porter, lines_code_fts unicode61, lines_trigram external-content) inside bulk tx; page_size default
- **candidate_configuration:** (a) chunked multi-VALUES INSERT (64/batch) — REGRESSED 674 vs 441 ms/50k lines: fresh statement text per chunk defeats prepare_cached; (b) trigram backfill via `INSERT INTO lines_trigram(rowid,content) SELECT rowid,content FROM lines` — only −6% of trigram stage (~28 ms/build): tokenization dominates, not insert machinery; (c) page_size 8k/16k/32k — noise; (d) post-load `optimize` merge — +3 MB DB size for ~8% cold MATCH gain, warm unchanged
- **measured_result:** no adoptable lever; write floor is FTS5 tokenization itself.
- **retry_condition_predicate:** Reopen batched writes ONLY with stable-statement batching (fixed max placeholder count padded with no-op rows) AND a profiler showing sqlite3RunParser/prepare churn >=5% of index wall (form 3). Revisit tokenize choice only as a product decision (changes postings, needs reindex contract) (form 8).
- **bead_id:** (none)

## Open (pointer imports)

### `historical-baselines-unreproducible`

- **target_workload:** published MRR / latency rows
- **files_touched:** `no-source-patch-attempted`
- **correctness_proof:** not-measured
- **evidence_artifact_paths:** `benchmarks/results/baselines.md`, `DISC-baselines-unreproducible`
- **baseline_configuration:** pointer-only
- **candidate_configuration:** pointer-only
- **measured_result:** not claimed here (see UNREPRODUCIBLE banner on the results files)
- **retry_condition_predicate:** Worth reconsidering when `benchmarks/results/baselines.md` marks a fingerprint row reproducible with harness + corpus + competitor pins in this tree.
- **bead_id:** `ast-sgrep-gauntlet-remediation-program-1vhy.2`

### `budget-rebaseline-open`

- **target_workload:** error budgets / keep-gate thresholds
- **files_touched:** `no-source-patch-attempted`
- **evidence_artifact_paths:** `docs/benchmarks.md`, `benchmarks/README.md`, WP1 keep-gate bead
- **retry_condition_predicate:** Blocked until WP1 keep-gate that refuses to lie lands; track as `ast-sgrep-gauntlet-remediation-program-1vhy.1`.
- **bead_id:** `ast-sgrep-gauntlet-remediation-program-1vhy.1`

### `losses-rg-std-printer`

- **target_workload:** ripgrep 14-query gold, `rg_std_printer`
- **evidence_artifact_paths:** `benchmarks/results/losses.md`
- **retry_condition_predicate:** Retry only if this workload class exhibits measurable reciprocal rank of `rg_std_printer` below the published loss narrative **and** the row is regenerated by an in-tree harness (today UNREPRODUCIBLE).
- **bead_id:** (none)

### `losses-rg-json-output`

- **target_workload:** `rg_json_output`
- **evidence_artifact_paths:** `benchmarks/results/losses.md`
- **retry_condition_predicate:** Retry only if this workload class exhibits measurable reciprocal rank of `rg_json_output` below the published loss narrative **and** the row is regenerated by an in-tree harness.
- **bead_id:** (none)

### `losses-rg-overrides`

- **target_workload:** `rg_overrides`
- **evidence_artifact_paths:** `benchmarks/results/losses.md`
- **retry_condition_predicate:** Retry only if this workload class exhibits measurable reciprocal rank of `rg_overrides` below the published loss narrative **and** the row is regenerated by an in-tree harness.
- **bead_id:** (none)

### `losses-rg-search-core-shared-miss`

- **target_workload:** `rg_search_core` (shared miss)
- **evidence_artifact_paths:** `benchmarks/results/losses.md`
- **retry_condition_predicate:** Retry only if a profiler attributes a clearly-above-noise share to hybrid fusion miss-ranking on a frozen ripgrep corpus with an in-tree gold harness.
- **bead_id:** (none)

### `withdrawn-dirty-eval-pack`

- **target_workload:** `./benchmarks/run_eval.sh` dirty worktree run
- **evidence_artifact_paths:** `benchmarks/results/baselines.md` (Candidate evaluation pack)
- **retry_condition_predicate:** Do not retry from a cold read; use comprehensive-bench attribution instead -- specifically a clean worktree `run_eval.sh` on a frozen/foreign corpus. The withdrawn dirty run is not canonical.
- **bead_id:** (none)

### `ivf-residual-unmeasured`

- **target_workload:** IVF/ANN post-T1R worker residual
- **evidence_artifact_paths:** none in this tree yet
- **retry_condition_predicate:** Retry only if a profiler attributes a clearly-above-noise share to IVF residual leaf work on a frozen corpus (hoy3.1 MEASURE).
- **bead_id:** `ast-sgrep-ho-ivf-residual-ho-20260807-hoy3.1`

### `gauntlet-2026-08-26-semantic-batched-file-fetch` (KEPT 2026-08-26 — equivalence hardening, perf neutral on measured corpora)

- **date:** 2026-08-26
- **candidate_name:** `semantic-chunks-batched-in-list` (B1)
- **target_workload:** the flat (non-IVF) embed path's two per-file loops — `semantic_chunks_for_files` + `semantic_field_vectors_for_files` ran one point-query per allowed file per call (~100–545 statements × 2 per query). Raw-SQL probe on the populated index: batched IN-list is sequence-identical and 1.4–1.7× faster standalone.
- **files_touched:** `crates/ast-sgrep-core/src/store/sqlite/queries.rs` (`semantic_rows_batched`; both functions rewired; `map_sorted_files` retained for `legacy_embeddings_for_files`)
- **correctness_proof:** sequence equality by construction — loops emit byte-sorted-path groups, `sc.id`-ascending within path; single `ORDER BY f.path, sc.id` reproduces it (Rust String sort == BINARY collation). Padding uses impossible value `''` (no real indexed path is empty) so row multiplicity is exact — unlike the caller-query bucket trick which repeats a real value. Golden battery 35/35 identical vs base; populated-index hybrid/semantic/lang-filtered batch payloads identical.
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/batch_sem_probe.py`, `/tmp/asgrep-bench/probe_sem.db`, golden_final manifest, interleaved rounds in session log
- **baseline_configuration:** per-path cached point queries; populated+embed p50 {11.52,11.70,11.60,11.54} ms
- **candidate_configuration:** one power-of-two-bucket IN-list query per call
- **measured_result:** p50 {11.47,11.81,11.58,11.77} (+0.9% = noise) on the IVF-served populated corpus, and {1.12→1.09 ms} on a below-threshold index — because on live workloads the IVF lazy path (chunks ≥ 2000) or small allowed_files sets make these loops a minor cost. KEPT for statement-count scaling: cost was O(files×2 statements) with prepare-cache pressure from varying placeholder counts at lang-filter boundaries; now O(1) stable-text statements. No regression anywhere measured.
- **retry_condition_predicate:** Perf re-measurement ONLY on a corpus where hybrid queries pass >1000 allowed_files to an embed-enabled index WITHOUT a valid IVF sidecar (fingerprint mismatch or below-threshold build) — there the removed O(files) fan-out dominates (form 4: corpus-shape-gated).
- **bead_id:** (none)

### `gauntlet-2026-08-26-ivf-byids-prepare-cache` (LANDED 2026-08-26 — BELOW GATE on this corpus; scaling-motivated)

- **date:** 2026-08-26
- **candidate_name:** `semantic-by-ids-statement-cache` (I5a)
- **target_workload:** populated index (6062 chunks), embed ON, IVF lazy path: `semantic_chunks_by_ids` + `semantic_field_vectors_by_ids` ran `conn.prepare()` (NOT cached) per 500-id batch — ~22 statement parses per cache-miss query at ~5.4k candidates.
- **files_touched:** `crates/ast-sgrep-core/src/store/sqlite/queries.rs` (two `prepare` → `prepare_cached`)
- **correctness_proof:** byte-identical by construction (same SQL text, same binds, same row map); golden battery 35/35 identical vs base
- **evidence_artifacts_paths:** interleaved A/B rounds in session log
- **baseline_configuration:** fresh prepare per batch; p50 {11.32,11.35,11.14,11.06} ms (median 11.23)
- **candidate_configuration:** `prepare_cached`; p50 {11.15,11.52,11.00,10.89} (median 11.07, −1.4%, direction-consistent 3/4 rounds but below the −3% gate)
- **measured_result:** BELOW GATE on this corpus. Kept anyway as pure infra hygiene: identical SQL/binds, removes parse churn that scales linearly with candidate volume (bigger semantic corpora pay proportionally more), zero risk surface.
- **retry_condition_predicate:** Re-measure on an embed-enabled corpus with >=50k chunks through the IVF path; expect the delta to cross the gate there (form 4: corpus-shape-gated).
- **bead_id:** (none)

### `gauntlet-2026-08-26-callers-lower-expression-index` (LANDED 2026-08-26 — keep, schema v13; 2000x lookup)

- **date:** 2026-08-26
- **candidate_name:** `callers-lower-expression-indexes` (schema 13)
- **target_workload:** graph surfaces (`chain`, `call-path`) and any consumer of `store.incoming_calls`/`outgoing_calls`: `calls_matching` ran `WHERE lower(c.callee) = lower(?1)` — a FULL SCAN of all caller rows (25k) per lookup, 20 ms each, because the existing raw-column indexes cannot serve `lower()` expressions.
- **files_touched:** `crates/ast-sgrep-core/src/store/sql.rs` (SCHEMA_DDL + `idx_callers_callee_lower`/`idx_callers_caller_lower`), `crates/ast-sgrep-core/src/store/sqlite/mod.rs` (SCHEMA_VERSION 12 → 13, `< 13` migration arm)
- **correctness_proof:** expression indexes are on the IDENTICAL expressions the query already evaluated (`lower(callee)`, `lower(caller)`) — same query text, same results, planner-only change. Chain JSON output byte-identical g7 vs g8 on the migrated index (all battery keys); migration verified on a copied v12 index (user_version 12→13, both indexes present, no data rebuild).
- **evidence_artifacts_paths:** EXPLAIN before (`SCAN c`) vs after (`SEARCH c USING INDEX idx_callers_callee_lower`); raw-SQL timings in session log; `golden_v13/manifest.json`
- **baseline_configuration:** incoming_calls('run_search') = 20.2 ms per lookup on the populated corpus
- **candidate_configuration:** two lower() expression indexes; 0.01 ms per lookup (~2000x)
- **measured_result:** KEEP. One-shot CLI walls for chain/call-path stay ~105–230 ms — spawn + seed search + BFS breadth dominate at this corpus's hop counts — but every per-hop lookup drops from 20 ms to microseconds, scaling with traversal volume. Migration is lazy (next open bumps user_version and builds two indexes inside the existing transaction).
- **retry_condition_predicate:** No reopen path needed. If a future schema bump lands alongside, keep both migrations ordered (`< N` arms) per the never-reuse rule.
- **bead_id:** (none)

### `gauntlet-2026-08-26-inlist-bucket-shrink-bugfix` (FIXED 2026-08-26 — latent correctness bug found by round-11 probing)

- **date:** 2026-08-26
- **candidate_name:** `inlist-bucket-power-of-two-shrink` (bugfix)
- **target_workload:** ANY hybrid/symbol query whose allowed_files size is exactly `2^k + 1` (9, 17, 33…): `restrict_to_files` computed `(n-1).next_power_of_two()` = n−1 for those sizes, emitting FEWER placeholders than bound paths → rusqlite "Wrong number of parameters passed to query. Got 9, needed 8". The bug shipped in br-perf-inlist-bucket and was inherited by the B1 batched fetch. Reproduced deterministically: hybrid `run_search` at limit 8/16 (allowed_files = 9) failed; limits 3/32 passed.
- **files_touched:** `crates/ast-sgrep-core/src/search/passes/symbol.rs` (restrict_to_files), `crates/ast-sgrep-core/src/store/sqlite/queries.rs` (semantic_rows_batched). Fix: `n.next_power_of_two().max(8)` (round UP), plus empty-set guards (`AND 0 = 1` / early return) replacing the old malformed `IN ()` shape.
- **correctness_proof:** limit sweep 1..65 × six queries × migrated index: 72/72 OK post-fix (previously 9/17 shapes failed); golden battery re-captured post-fix (`golden_v13`); e2e_smoke 9, snapshot_generation 6, trigram_shortcut 4, cli_smoke 14 all green.
- **evidence_artifacts_paths:** `/tmp/b1repro` (in-process reproducer sweeping SearchOptions.limit), session log A/B rounds
- **baseline_configuration:** `(n - 1).next_power_of_two().max(8)`
- **candidate_configuration:** `n.next_power_of_two().max(8)`
- **measured_result:** FIXED. Perf neutral (placeholder count changes only at former failure shapes).
- **retry_condition_predicate:** None — defect class eliminated at both sites. Any future bucketed IN-list must use round-up semantics; add to review checklist.
- **bead_id:** (none)

### `gauntlet-2026-08-26-trigram-sql-reverify` (LANDED 2026-08-26 — keep, tail −16% on dense scans)

- **date:** 2026-08-26
- **candidate_name:** `trigram-scan-sql-side-glob-reverify` (T1)
- **target_workload:** literal_prefilter = 74% of worker samples on lang-filtered broad queries over a 351k-line corpus (1501 files); inside it, `likeFunc`+`patternCompare`+`strcspn` ≈ 40% — the Rust-side `content_matches_literal` reverify ran per streamed posting with full TEXT materialization of path/language/content for every candidate, including rejected ones.
- **files_touched:** `crates/ast-sgrep-core/src/search/passes/literal.rs` (scan_trigram_matches): for case-sensitive non-word needles the reverify predicate (identically `GLOB '*<needle>*'`, escaped via the same `escape_glob_literal` helper as the literal_sql arm) is pushed into SQL; word_mode and case_insensitive keep the Rust verify.
- **correctness_proof:** same rows, same predicate, same streaming order — output identical by construction. Golden battery 35/35 byte-identical (`golden_v13`); big-corpus equivalence sweep (dense/sparse/word/case-insensitive/metachar needles) g8↔g9 all identical.
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/flames_big.txt` (worker sample at scale), raw-SQL probe (sql-glob 0.6 vs rust-verify 1.8 ms per dense scan), interleaved rounds in session log
- **baseline_configuration:** Rust reverify per posting; big-corpus dense-needle p90 {8.85, 8.92, 9.99} ms
- **candidate_configuration:** SQL-side GLOB; p90 {6.89, 7.49, 9.24} ms (median −16%); cold-start worst case avoided entirely (g8 r0 outlier 51.9 ms mean-top vs g9 15.2); warm steady-state neutral (~2.4 ms both); repo corpus unchanged (g9 1.66 vs g8 1.61–1.68)
- **measured_result:** KEEP — tail win concentrated exactly where predicted (dense postings × sparse content), zero regression elsewhere.
- **retry_condition_predicate:** If a future word_mode/case-insensitive tail shows up in profiles, extend pushdown with the corresponding SQL predicates (word boundaries need a REGEXP/function arm or post-filter) — only under sampler evidence ≥5%.
- **bead_id:** (none)

### `gauntlet-2026-08-26-ivf-byids-prepare-cache-retest` (MEASURED AT SCALE — prediction failed, entry updated)

- **date:** 2026-08-26
- **candidate_name:** `semantic-by-ids-statement-cache` (I5a) — retry-predicate test
- **target_workload:** synthetic 54,722-chunk corpus (1501 files, 289k caller edges), embed ON through IVF path: g5 (no I5a) vs g6 (I5a) on the same v12 index, 800 distinct symbol needles.
- **files_touched:** none this round
- **correctness_proof:** not-applicable (measurement pass)
- **evidence_artifacts_paths:** session log interleaved rounds; `/tmp/asgrep-bench/gen_bigcorpus.py`, idx_bigv12/index.db
- **baseline_configuration:** p50 {14.09, 14.03} ms (g5)
- **candidate_configuration:** p50 {14.12, 13.98} ms (g6) — −0.2%, below gate
- **measured_result:** RETRY PREDICTION FAILED. The original entry predicted the delta would cross the −3% gate at ≥50k chunks; measured −0.2–0.8%. Statement-parse churn was already amortized by SQLite's internal schema cache; the by-ids cost is row fetch + decode, not parsing. I5a stays as harmless hygiene but its scaling rationale is retired.
- **retry_condition_predicate:** CLOSED as scaling-motivated-only. No further measurement passes warranted absent a profiler showing prepare/parse frames ≥5% on the IVF path (form 3).
- **bead_id:** (none)

### `gauntlet-2026-08-26-b1-flat-path-at-scale` (MEASURED 2026-08-26 — hypothesis closed, predicate shape unreachable)

- **date:** 2026-08-26
- **candidate_name:** `semantic-chunks-batched-in-list` (B1) — retry-predicate test at scale
- **target_workload:** the original B1 retry predicate required hybrid queries passing >1000 allowed_files to an embed-enabled index WITHOUT a valid IVF sidecar. Built exactly that: 54,722-chunk corpus, sidecar removed to force the flat path, g0 (per-file loops) vs g5 (batched).
- **files_touched:** none this round
- **correctness_proof:** not-applicable (measurement pass)
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/idx_bigv12/` (sidecar `.ivf.bak`), session log rounds; capsule-shape probes (35/80/163/176 ms for 1/2/3/4-term needles)
- **baseline_configuration:** g0 per-path loops; battery + broad natural-language needles
- **candidate_configuration:** g5 batched IN-list
- **measured_result:** NO WIN AVAILABLE. p90 on battery shapes {16.2–18.5} both binaries; capsule shapes ~100ms p90 both. Root cause: allowed_files reaching the embed pass is bounded by the prefilter output — the >1000-file shape requires the prefilter to pass >1000 files AND the IVF sidecar to be absent, which co-occur only on pathological indexes (stale sidecar + near-empty lexical channel). The predicate's premise was wrong: statement fan-out never dominates because file sets are pre-narrowed.
- **retry_condition_predicate:** CLOSED. Only reachable if a future surface passes unfiltered (whole-corpus) file sets into the embed passes — e.g., a semantic-only sweep command. Re-check then (form 4).
- **bead_id:** (none)

### `gauntlet-2026-08-26-delta-reindex-ivf-rebuild` (LANDED 2026-08-26 — Door A: centroid-preserving reassign)

- **date:** 2026-08-26
- **candidate_name:** `delta-reindex-centroid-preserving-reassign` (Door A)
- **target_workload:** incremental reindex on a large semantically-chunked index (54,722 chunks, 1501 files): editing ONE file cost **~48–58 s** per dir-mode delta pass. Span attribution on HEAD: `semantic_ivf_build` = 46.5 of 48.1 s (97%); walk+parse 7 ms; sqlite_upsert 180 ms. No-op passes cost ~1.5 s — hashing is not the bottleneck.
- **files_touched:** `crates/ast-sgrep-core/src/semantic_ann.rs` (`reassign_all` keeps centroids; `mark_semantic_ivf_stale` no longer deletes sidecar; `drop_semantic_ivf`; `reassign_stale_ivf_partition` allows count drift), `index.rs` (`force_reindex` still invalidates sidecar so k-means runs), `store/sqlite/mod.rs` (wipe sites call `drop_semantic_ivf`), `tests/core/{semantic_ivf_roundtrip,durability_epics}.rs`, `docs/semantic-search.md`
- **correctness_proof:** form-8: observable ANN recall may change; lexical/structural 35-contract goldens stay byte-identical. Fixture recall@10 vs frozen centroids (SLO 0.99): n=2048 0.998437; +1 0.998444; +10 0.998450; +50 0.998479. Centroids byte-identical across those reassigns. `semantic_ivf_roundtrip` 11/11 (1 ignored scale job); `durability_epics` 18/18. Sidecar kept on delta `remove_file`; `drop_semantic_ivf` still deletes.
- **evidence_artifacts_paths:** cargo test `centroid_preserving_reassign` output; `/tmp/asgrep-bench/delta_spans.jsonl` (pre-change attribution)
- **baseline_configuration:** any chunk-count change deleted `semantic.ivf` and fell through to 12-iter k-means (`reassign_all` was `*self = Self::build_from_flat`)
- **candidate_configuration:** keep existing centroids; nearest-centroid assign every current vector; rewrite cluster postings and sidecar. Full k-means only on cold build / explicit `asgrep reindex` / embedding-identity wipe.
- **measured_result:** recall KEEP on the 2048-vector CI fixture. 54k-chunk dir-mode wall-time NOT YET MEASURED — do not claim 48 s → 1.5 s (hashing already 1.5 s on no-op). Expected span name `semantic_ivf_reassign` instead of `semantic_ivf_build`.
- **retry_condition_predicate:** Falsify if recall@10 < 0.99 after +1/+10/+50 appends vs frozen centroids — then stop; optional rebuild trigger only if that fails (`|Δn|/n > 0.25` or `sqrt(n).clamp(16,256)` changed). 54k keep-gate still open: dir-mode delta after one-function append must show the IVF span ≥2× faster, no-op still ~1.5 s, cold full index within ±3% (form 3).
- **bead_id:** (none)

## Retired

_(none)_

### `trigram-order-by-temp-btree`

- **target_workload:** warm distinct literal/trigram search, self corpus (1,100+ files)
- **files_touched:** `crates/ast-sgrep-core/src/search/passes/literal.rs`
- **correctness_proof:** 35-contract golden battery byte-identical on under-budget queries (overflow >=16-hit subsets shift by posting order, same class as pre-existing lazy cut)
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/` (golden.py, golden_fresh/, asgrep_base2 vs asgrep_v3); EXPLAIN QUERY PLAN showed `USE TEMP B-TREE FOR ORDER BY` materializing up to 28k-row doclists before row 1
- **baseline_configuration:** `ORDER BY f.path, l.line_no` in trigram SQL; warm distinct p50 4.2ms
- **candidate_configuration:** no SQL ORDER BY; lazy stream + Rust re-sort of <=budget candidate set; warm distinct p50 2.9ms (combined with lexical join-free lever, commit ebfaace3)
- **measured_result:** -31% p50, -19% p10; identical-repeat cache-hit path ~0.11ms (sub-1ms proven)
- **retry_condition_predicate:** Revisit only if a profiler attributes >30% of warm distinct-query time to the Rust candidate re-sort after the FTS scan (form 3: profiler-gated).
- **bead_id:** (none — closed as keep, commit ebfaace3)

### `lexical-per-row-join`

- **target_workload:** warm distinct hybrid search, lexical bm25 stage
- **files_touched:** `crates/ast-sgrep-core/src/search/passes/lexical.rs`
- **correctness_proof:** same golden battery; bm25 ranking order preserved; identity resolution batched per surviving file_id set
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/` rawsql.py: fts-only 0.80ms vs joined 1.59ms on 1163-row match set
- **baseline_configuration:** two per-row JOINs (files + lines) over every candidate
- **candidate_configuration:** rank inside FTS table (already stores file_id/line_no/content); one bounded files IN-list for <=limit survivors
- **measured_result:** ~0.8ms saved per lexical stage invocation
- **retry_condition_predicate:** Revisit only if bm25 top-k heap behavior changes in vendored SQLite such that the join becomes free (form 5: dependency-version-gated).
- **bead_id:** (none — closed as keep, commit ebfaace3)

### `trigram-posting-cap-in-sql-limit`

- **target_workload:** warm distinct literal/hybrid search through codemode-serve, self corpus (1,102 files, 103k trigram lines); literal_trigram_scan span
- **files_touched:** `crates/ast-sgrep-core/src/search/passes/literal.rs`
- **correctness_proof:** 35-contract golden battery byte-identical between A/B binaries (asgrep_base4 @ 1be70c9b vs asgrep_v5)
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/` doclist_probe.py (postings distribution over the 300 bench terms), single2.py interleaved rounds, ASGREP_PERF_PROFILE span dumps spans_v5_a/b.jsonl
- **baseline_configuration:** unbounded SQL stream + hit-count break at max(limit,100) (ebfaace3 shape); warm distinct p50 ~2.55ms; literal_trigram_scan = 25% of warm-path time, avg 906us/scan
- **candidate_configuration:** SQL LIMIT = max(limit,100)x24 postings (2,400 at the prefilter limit) so low-density terms stop streaming instead of walking their whole doclist
- **measured_result:** no improvement: p50 base {2.60, 2.64, 2.52} vs lever {2.62, 2.71, 2.52}; trigram span share 25.0% vs 24.8%; avg/scan 906us vs 910us. Postings probe explains why: p50=85, p75=441, p90=1332, max=4,874 — the hit-count break already bounds every dense term at ~100 rows, so the only population the posting cap trims (doclist >2,400 AND <100 hits) is empty on this corpus.
- **retry_condition_predicate:** Revisit only if a postings probe on the target corpus shows a non-empty tail (terms whose doclist exceeds the hit-break budget while yielding fewer hits than the budget), or a profiler attributes >=10% of warm distinct-query time to fts5NextMethod/sqlite3_step frames under literal_trigram_scan AFTER a two-phase deferred-join prototype measures >=15% span reduction (form 3: profiler-gated).
- **bead_id:** (none — measured and rejected this campaign, reverted before commit)

### `finish-coverage-comparator-recompute`

- **target_workload:** warm distinct literal/hybrid search through codemode-serve, self corpus (1,100+ files); finish.rs response finishing
- **files_touched:** `crates/ast-sgrep-core/src/search/finish.rs`
- **correctness_proof:** 35-contract golden battery byte-identical between A/B binaries (golden.py capture on base HEAD build, verify on lever build)
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/` single2.py (repo-root serve driver, warm-up excluded, 299 distinct queries), asgrep_base3 vs asgrep_v4, golden_v4base/manifest.json
- **baseline_configuration:** excerpt_term_coverage evaluated inside the prune select_nth comparator (two full excerpt scans + to_lowercase allocation per comparison) at commit 7dd5fa32; warm distinct p50 ~2.5ms
- **candidate_configuration:** coverage computed once per hit into (key, hit) pairs before prune/select/sort (permutation-proof by construction — keys travel with hits); identical comparator values
- **measured_result:** no improvement: p50 base {4.08, 2.50, 2.68, 2.65, 2.46} vs lever {2.42, 2.67, 2.59, 2.54, 2.80}; limit=25 rounds {2.41/2.66/2.74 base vs 2.67/2.75/3.07 lever}. Deltas within run-to-run noise; the prune branch rarely engages at real query shapes (prune_keep=4x+32 over the gate limit), so the comparator recomputes are not a measurable share of warm-path time.
- **retry_condition_predicate:** Revisit only when a profiler attributes >=5% of search_process_request time to excerpt_term_coverage frames on warm distinct queries (form 3: profiler-gated).
- **bead_id:** (none — measured and rejected this campaign, reverted before commit)

### `lexical-fts-fallback-double-query-scope-reclass`

- **target_workload:** warm distinct queries; lexical_from_fts fallback-field re-query (vvpk analyzer routing, lines_fts porter vs lines_code_fts identifier)
- **files_touched:** `no-source-patch-attempted`
- **correctness_proof:** not-applicable (measurement + call-site audit only)
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/` probe3.py (per-term two-field replay: primary/fallback timings, postings counts, new-key yield over the 300 bench terms); rg of lexical_pass call sites
- **baseline_configuration:** fallback fires when unique (path,line) keys < limit after the primary field query
- **candidate_configuration:** none built — the campaign brief's premise ("rare terms pay double on the codemode warm path") does not hold: lexical_pass is called ONLY from Searcher::search_lexical, which no codemode tool reaches (hybrid search_hybrid uses the trigram literal prefilter instead). Its real consumers are MCP AgentSearchMode::Keyword and one CLI path.
- **measured_result:** scope reclassification, not a rejection of a measured candidate. On the bench traffic the fallback fires on 65% of terms but averages 0.34 ms/query (15 postings avg when fired) — ~0.22 ms amortized per query, 30% of lexical SQL time, which itself is off the codemode hot path. Worth revisiting ONLY as an MCP-keyword-mode improvement.
- **retry_condition_predicate:** Revisit as an MCP Keyword-mode optimization if MCP keyword-search p50 becomes a tracked surface with a profile showing >=15% of its time in the fallback field query (form 3: profiler-gated, surface-scoped).
- **bead_id:** (none)

### `symbol-caller-per-term-batching-premise-stale`

- **target_workload:** hybrid search structural stage (symbol_pass_for_files / caller rows)
- **files_touched:** `no-source-patch-attempted`
- **correctness_proof:** not-applicable (premise refuted by code reading)
- **evidence_artifacts_paths:** crates/ast-sgrep-core/src/store/sql.rs like_terms_filter/or_like_filter (OR of lower(col) LIKE across ALL terms in ONE query); symbol.rs symbol_pass_for_files/caller_terms_filter call sites; direct sqlite3 timings on .asgrep/index.db
- **baseline_configuration:** current HEAD already batches every term into a single OR-LIKE statement per stage (one symbols query + one callers query), bounded by SYMBOL_SQL_LIMIT/CALLER_SQL_LIMIT=500 and the files IN-list
- **candidate_configuration:** "batch across terms" — already implemented upstream of this campaign entry
- **measured_result:** premise stale: there is no per-term loop left to batch. Direct measurement of the exact statement shapes with a 100-path IN-list on this corpus: <5 ms per query (below shell timer resolution) for both stages.
- **retry_condition_predicate:** Reopen only if a profiler attributes >=10% of warm distinct-query time to symbol_pass_for_files or caller_rows frames despite the existing batching (form 3: profiler-gated).
- **bead_id:** (none)

### `trigram-scan-cost-attribution` (CLOSED 2026-08-23 — predicate satisfied by br-umh)

- **target_workload:** literal_trigram_scan span = 25% of warm distinct-query time (avg 906us/scan over 3,700 scans)
- **files_touched:** `no-source-patch-attempted` (attribution pass); superseded by the br-umh implementation below
- **correctness_proof:** not-applicable (measurement pass)
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/` trigram_cost_model.py (cost vs doclist size: slope ~= 0us/posting, quartiles 1.497 vs 1.498 ms), phrase_vs_single.py (phrase vs single-middle-trigram MATCH: 457 vs 391 ms total, huge per-term variance, +8 ms regression worst case), defer_join_bench.py (deferred rowid->lines/files join past LIMIT: -3%, i.e. joins are free)
- **baseline_configuration:** current ebfaace3-shape trigram scan
- **candidate_configuration:** three prototypes evaluated in SQL directly: posting-cap LIMIT (see closed entry above), deferred join (rejected here), subset-trigram MATCH + Rust verify of remaining trigrams (content_matches_literal already guarantees exactness)
- **measured_result:** scan cost is FLAT vs doclist size and grows with TERM LENGTH (more trigrams intersected by FTS5 phrase machinery: fts5NextMethod/fts5ExprNodeTest_STRING frames). Deferred join saves nothing (joins are 1:1 rowid lookups on a warm page cache). Subset-trigram saves ~15% total ONLY with a lucky rare-trigram pick; blind picks regress badly (a common middle trigram floods the candidate pool).
- **retry_condition_predicate:** Reopen only with trigram document-frequency metadata available at query time (e.g., persisted per-token df sidecar or FTS5 function support) so the RAREST trigram can be picked deterministically AND a profiler still attributes >=10% of warm-path time to fts5 frames; then subset-MATCH + Rust verify is output-identical by construction and bounded-variance (form 3: profiler-gated + form 4: dependency/metadata-gated).
- **closure:** predicate satisfied and landed as br-umh (2026-08-23): ephemeral temp fts5vocab df source, deterministic rarest pick, ~21% warm distinct p50 reduction with 35/35 byte-identical goldens. Row: `benchmarks/results/speed.md::2026-08-23 trigram df rarest-trigram MATCH`. The >=10% profiler-attribution condition was measured at 25% (this entry).

### `trigram-df-gate-too-tight-256` (Open pointer)

- **target_workload:** warm distinct literal/trigram search, self corpus (median trigram doc-frequency 85, p75=441, p90=1332)
- **files_touched:** `crates/ast-sgrep-core/src/store/trigram_df.rs` (threshold constant only; final ship value 2048)
- **correctness_proof:** tests/core/trigram_shortcut.rs green at all measured thresholds
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/` single2.py rounds in `benchmarks/results/speed.md::2026-08-23 trigram df rarest-trigram MATCH` (256 rows) and binaries asgrep_base5/asgrep_v6/asgrep_v9
- **baseline_configuration:** full-phrase trigram MATCH (base p50 2.30-2.73 ms across interleaved rounds)
- **candidate_configuration:** rarest-trigram shortcut with RARE_ENOUGH_DF=256 (v6 binary)
- **measured_result:** not a keep at 256: p50 {2.79, 2.78, 2.60, 2.70} vs base {2.50, 2.54, 2.28, 2.59} — consistently ~+0.3 ms. The gate excluded the population that benefits: most battery needles' best trigram sits between 441 and 1332 df, so the picker paid lookup overhead (~35us x several probes) and then fell back to the unchanged full-phrase scan.
- **retry_condition_predicate:** Revisit a tight rarity gate ONLY on a corpus whose postings-probe shows a materially lower df distribution (e.g., median df < 64), or after per-term cost modeling shows single-posting scans winning below that median (form 4: corpus-shape-gated).
- **bead_id:** br-umh

### `warm-fixed-cost-memoization-probes` (Open pointer)

- **target_workload:** warm distinct single-term literal/hybrid search through codemode-serve over the self corpus (1,100+ files, ~103k indexed lines); post-br-umh baseline p50 ~2.0-2.2 ms
- **files_touched:** `no-source-patch-attempted` (prototypes measured, then reverted; only a routing-contract test landed)
- **correctness_proof:** tests/core/literal_threshold_probe.rs pins the trigram-vs-SQL routing decision (the observable effect of the probed value) so future memoization cannot change results
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/flames_head.txt` (10 s worker sample at HEAD: 7330 run-loop samples / 2679 calls ≈ 2.74 ms/call); raw-SQL microbenchmarks on an index copy (`df_probe.db`): threshold COUNT probe ~40 us/call, caller LIKE scan without file restriction ~4.2 ms, with the 100-file IN-list ~62 us
- **baseline_configuration:** HEAD `a160e30d` release-perf build
- **candidate_configuration:** (A) gen-keyed memoization of `indexed_line_count_at_least(BMH_LINE_THRESHOLD)` on the store; (B) reuse of the main scan's hits inside `literal_prefilter_pass` for single-term queries
- **measured_result:** not keeps — interleaved A/B (4 rounds) measured candidate p50 {2.54, 2.80, 2.43, 2.61} vs base {2.22, 2.03, 2.09, 2.02}: both fixes REGRESSED p50 by ~0.3-0.6 ms despite removing work the flame profile attributed at ~1% (probe) and ~15% (prefilter re-scan). Root cause hypotheses for the regression: (A) the Mutex lock + generation read on every call costs more than the 40 us COUNT it saves under this access pattern; (B) hoisting/branching around the prefilter loop perturbed inlining/code layout of the hottest loop. Neither hypothesis was confirmed with a targeted experiment before revert.
- **retry_condition_predicate:** Reopen either fix ONLY after a profiler attributes >=5% of warm-path time to the specific frame being memoized/hoisted AND a microbenchmark shows the saved operation costing more than the added synchronization (for A: COUNT probe > mutex+gen read, measured per-access) on the target hardware (form 3: profiler-gated + form 4: measurement-gated).
- **bead_id:** (none)

### `callers-fts-trigram-index` (Open pointer)

- **target_workload:** warm distinct single-term literal/hybrid search through codemode-serve over the self corpus (1,100+ files, 3.7k symbol rows, 27.8k caller rows); post-br-umh baseline p50 ~2.0 ms
- **files_touched:** prototype only — SCHEMA_DDL callers_fts table, insert/delete/clear sync, schema v13 backfill migration, FTS-restricted caller query in symbol_pass_for_files (all reverted)
- **correctness_proof:** prototype validated output-equivalence by raw SQL: 30/30 corpus terms produced identical candidate file sets vs the LIKE scan (trigram MATCH over caller+callee names); targeted tests written for insert/delete/clear/backfill sync passed at each step
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/flames_head.txt`, `df_probe.db` microbenchmarks, `single2.py` interleaved rounds (binaries asgrep_head/asgrep_vB2), `load3.py` throughput runs
- **baseline_configuration:** HEAD `17fbec27` (LIKE-based caller matching): single2 p50 {1.98, 2.03, 2.06, 2.06}, load3 29873 real calls @4.02 ms avg
- **candidate_configuration:** callers trigram FTS index (raw SQL microbench: unrestricted caller scan 4.2 ms -> MATCH+join 45-100 us, 41x) wired into symbol_pass_for_files as a candidate-file prefilter intersected with allowed_files
- **measured_result:** not a keep: single2 p50 {2.16, 2.01, 2.00, 2.10} (median 2.055 vs base 2.045 — within noise), load3 27843 calls @4.31 ms (~7% WORSE throughput). Root cause of the null result: the unrestricted caller LIKE scan shape (the 4.2 ms frame measured in isolation) does not occur on this workload — hybrid always passes allowed_files, so the live caller query is already file-list-driven (~62 us). The flame profile's likeFunc frames belong to the s.name LIKE (symbols, cheap) and the file-restricted caller query, not to an unbounded scan. Also measured and rejected along the way: removing ORDER BY from LITERAL_SQL saves ~11 ms raw on sub-3-char terms BUT violates the byte-identity gate even for under-budget queries because fusion assigns scores from candidate POSITION over a saturated SQL window (fx-lang-py order flip).
- **retry_condition_predicate:** Reopen ONLY if (a) a profiler shows >=10% of warm-path time in caller-table scans WITHOUT a file IN-list restriction on the same workload (i.e., a call path that reaches query_caller_rows with allowed_files=None), or (b) the product adds a callers_fts consumer for another feature so the index maintenance cost is amortized (form 3: profiler-gated).
- **bead_id:** (none)

### `pattern-walk-finer-partitioning-depth3-frontier` (CLOSED 2026-08-24)

- **date:** 2026-08-24
- **candidate_name:** `pattern-walk-finer-partitioning-depth3-frontier`
- **target_workload:** distinct braced structural pattern first-touch through codemode-serve, self corpus (545 indexed files after gitignore prune; ~164k on-disk entries)
- **files_touched:** prototype measured and reverted (three variants); shipped alternative is BFS-levels walker in `crates/ast-sgrep-core/src/pattern.rs` (`ASGREP_WALK_THREADS` knob) — see `3bffdbe5`
- **correctness_proof:** serial-vs-candidate hit-set oracle identical on four declaration patterns (253-hit `struct $NAME { $$$ }` set equal in `(file, start, end)`); the depth-3 frontier variant was correct but slower; two subroot-replacement variants produced file-set coverage bugs (554/522/557 vs oracle 545) and were reverted per three-strikes rule
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/phase2.py` (phase instrumentation), `/tmp/asgrep-bench/oracle_ab.py` (identity oracle), `/tmp/asgrep-bench/clamp_sweep.py` (2/4/8/16-worker sweep), binaries `asgrep_v28..v34`; numbers in `benchmarks/results/speed.md::2026-08-24 BFS parallel walk`
- **baseline_configuration:** macOS arm64 (M5 Max), release-perf, HEAD `9524e08c` — distinct pattern 43–48 ms
- **candidate_configuration:** (a) depth-3 fixed frontier (serial phase-1 stats to depth 3, depth-3 dirs as units); (b/c) mixed-depth subroot replacement sets — all rejected; BFS levels with capped pool adopted instead
- **measured_result:** depth-3 frontier: walk 40–55 ms + scan 6–23 ms → totals 57–104 ms vs shipped 43–48 ms — SLOWER (Amdahl: phase-1 serial stats grew with depth). Clamp sweep on adopted BFS: 2 workers 57/65/85 ms (min/avg/max), 4 workers 37/41/51 ms (shipped default), 8 workers 26/31/39 ms, 16 workers 31/35/41 ms.
- **retry_condition_predicate:** Reopen finer partitioning ONLY with a PARALLEL phase-1 (concurrent per-dir read_dir fan-out or the `ignore` crate if dependency policy allows); deeper SERIAL enumeration is measured counterproductive (form 4 + dependency gate).
- **bead_id:** br-kcx (closed: landed)

### `hybrid-cold-needle-tail-sub1ms` (CLOSED 2026-08-24 — goal infeasible as stated; partial levers landed)

- **date:** 2026-08-24
- **candidate_name:** `hybrid-cold-needle-tail-sub1ms`
- **target_workload:** FIRST-touch (response-cache-missing) high-df literal needles through hybrid search, codemode-serve, self corpus; baseline p99 14–20.5 ms / max 22–26 ms vs pipeline floor 0.156 ms
- **files_touched:** `crates/ast-sgrep-core/src/store/sql.rs` (cache_size −16384 → −71680), `crates/ast-sgrep-core/src/store/trigram_df.rs` (bulk vocab preload per generation), `crates/ast-sgrep-core/src/search/passes/symbol.rs` (IN-list bucket quantization) — commit `0a08adc`
- **correctness_proof:** golden battery 35/35 byte-identical; trigram_df 5, trigram_shortcut 4, pattern_routing, cli_smoke 14 green; IN-membership equivalence by construction (duplicates don't change membership)
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/{single_tail,tail_queries,discrim,percall_spans,isolated_spans,multi_cold,cache_fill,cache_fill37}.py`, samples `ws_*.txt`/`single_cold.txt`/`multi_cold.txt`, span dumps `hyb*.jsonl`
- **baseline_configuration:** macOS arm64 M5 Max, release-perf, `b06cdd43`; p50 1.7–2.0 / p90 10.6–11.8 / p99 14–20.5 / max 22–26 ms
- **candidate_configuration:** three levers above, measured individually and combined (`asgrep_v37/v38`)
- **measured_result:** combined: p99 18.9 ms / max 20.7 ms — a bounded improvement (~15–25% of tail), NOT the sub-1ms target. Triangulated attribution (env-gated spans + `sample` on worker + subtraction): the cold tail is SQLite row-streaming and B-tree page walking for each first-touch needle's trigram postings plus structural-stage SQL — candidate-volume work bounded below by data volume. Repeat queries already sit at 0.11–0.17 ms and literal:-direct cold needles at 0.3–0.7 ms.
- **retry_condition_predicate:** Sub-1ms p99 across ALL first-touch needles is achievable only by (a) persisting an answer cache across sessions with explicit staleness semantics (semantics-changing, needs product sign-off), or (b) restricting the metric to warm/repeat or literal:-direct workloads (already sub-ms). Reopen only if one of those two product decisions is made (form 8: blocked on architectural/product decision).
- **bead_id:** (none — closed this campaign)

### `hybrid-structural-excerpt-lazy-attach` (LANDED 2026-08-24 — keep, 2x on high-df cold)

- **date:** 2026-08-24
- **candidate_name:** `hybrid-structural-excerpt-lazy-attach`
- **target_workload:** first-touch hybrid needles (response-cache-missing), codemode-serve, self corpus; structural passes were fetching one indexed excerpt SQL per candidate hit before fusion discarded most of them
- **files_touched:** `crates/ast-sgrep-core/src/search/finish.rs`, `search/mod.rs` (hybrid finish → lazy variant), `search/passes/symbol.rs` (`*_opts(attach_excerpts)` params; `attach_indexed_excerpts_if_empty`) — commit `251446fe`
- **correctness_proof:** golden battery 35/35 byte-identical on a freshly rebuilt index; fusion input member sets identical (attachment moved, not removed); critic/prune see identical excerpts for every survivor. NOTE: verify goldens only against a same-session index — stale-index drift produces false DIFFs (both binaries agree pairwise).
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/{discrim,single_tail,burst_driver,multi_cold}.py`, samples `tail_sample.txt`/`ws_*.txt`, span dumps `hyb*.jsonl`
- **baseline_configuration:** macOS arm64 M5 Max, release-perf, `d59380a6`; high-df cold needles avg 24.9 ms; tail battery p99 19.4–19.9 ms
- **candidate_configuration:** symbol def/caller/anchor passes skip per-hit excerpt attachment in the hybrid path; `finish_response_checked_lazy` attaches once post-dedup/pre-prune via `attach_indexed_excerpts_if_empty`
- **measured_result:** KEEP — high-df cold needles 24.9 → 12.4 ms avg (2x); tail battery p99 17.3–18.8 ms / max ~23 ms (modest, low-df-dominated); sustained load unchanged (27.2k calls/120s, 0 errors). Triangulated attribution: rusqlite Rows streaming + sqlite3_step + string materialization were ≥70% of tail samples.
- **retry_condition_predicate:** Further tail reduction requires cutting row *streams*, not storage: batched/deferred join variants were measured neutral-to-negative warm (`trigram-scan-cost-attribution`), so reopen only with a parallel phase-1 walker or an async SQLite reader (form 8: architectural dependency).
- **bead_id:** (none)

### `gauntlet-2026-08-26-embed-empty-sources-guard` (LANDED 2026-08-26 — keep, correctness guard + small win)

- **date:** 2026-08-26
- **candidate_name:** `embed-pass-empty-sources-guard` (E1)
- **target_workload:** default-config (embed ON) hybrid queries against any index whose semantic layer is empty — including this repo's own `.asgrep` and every index built with `--no-embed`. `embed_pass_for_files_with_rescoring` ran three per-file query loops (`semantic_chunks_for_files`, `semantic_field_vectors_for_files`, `legacy_embeddings_for_files`) before its `survivors.is_empty()` early return: ~3×N pointless statements per query.
- **files_touched:** `crates/ast-sgrep-core/src/store/sqlite/queries.rs` (`semantic_sources_empty()`), `crates/ast-sgrep-core/src/search/passes/embed.rs` (guard at pass entry)
- **correctness_proof:** output-identical by construction — with zero chunks AND zero embeddings every loop contributes no rows and the old code returned `Ok(Vec::new())`; the guard only skips proving that one point-query at a time. Golden battery 35/35 byte-identical; populated-index batch hashes identical.
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/golden_final/manifest.json`, interleaved A/B rounds in session log
- **baseline_configuration:** macOS arm64 M5 Max, release-perf, HEAD `8bc467cb`, embed ON, repo index (0 chunks)
- **candidate_configuration:** one `SELECT CASE WHEN EXISTS(…chunks…) OR EXISTS(…embeddings…) THEN 0 ELSE 1 END` probe (microseconds on non-empty stores) before the loops
- **measured_result:** KEEP as a correctness/efficiency guard. Latency effect small on this corpus (~0.06–0.3 ms p50) because the loops are cheap per statement; cost scales with allowed_files count, so larger corpora benefit more.
- **retry_condition_predicate:** No reopen path needed; behavior is strictly skip-provably-dead-work. If a future semantic source is added beyond these two tables, extend the probe in the same commit.
- **bead_id:** (none)

### `gauntlet-2026-08-26-snapshot-stamp-memoization` (LANDED 2026-08-26 — keep, −7–10% populated+embed p50)

- **date:** 2026-08-26
- **candidate_name:** `snapshot-stamp-generation-keyed-memo` (S1)
- **target_workload:** FIRST surface mined with embed ON and a semantically POPULATED index (6062 chunks): hybrid distinct-query p50 12.5 ms vs 2.7 ms no-embed. Sampler: `snapshot_stamp` = 17.5% of worker CPU — per cache-miss query it re-ran `semantic_chunk_stats` (COUNT + MAX(length(vector)) over all chunk vectors ≈ 0.36 ms), `worktree_revision` (MAX over files), and the IVF sidecar mmap+parse peek — all pure functions of index contents.
- **files_touched:** `crates/ast-sgrep-core/src/search/mod.rs` (`stamp_cache`/`stamp_degraded` fields, `cached_stamp_parts`, `take_stamp_degraded`, `semantic_manifest_impl`, snapshot_stamp rewiring)
- **correctness_proof:** golden battery 35/35 byte-identical vs base on same-session index/corpus (g0 vs g4b); `snapshot_generation` tests green (6/6), incl. the stale-sidecar degraded-channel contract — mismatch verdicts are never memoized and unreadable-sidecar notes are drained per response so staleness stays loud. `git_head` deliberately stays uncached (worktree-bound, not generation-bound). Memo keys on full IndexGeneration (external data_version + local counters, br-yp1 semantics); pragma failure falls back to direct recompute (hdwh fail-open-to-recompute).
- **evidence_artifacts_paths:** `/tmp/asgrep-bench/flames_embpop.txt` (worker sample on populated index), `/tmp/asgrep-bench/golden_final/`, interleaved rounds in session log
- **baseline_configuration:** release-perf `8bc467cb` + E1; populated+embed p50 {13.09,12.92,12.33,12.28} ms; warm-distinct no-embed unchanged ~1.9 ms
- **candidate_configuration:** generation-keyed memo of (worktree_revision, semantic_manifest) consulted inside snapshot_stamp
- **measured_result:** KEEP — populated+embed p50 {11.96,12.02,11.48,11.62} then final-binary confirm −9.4%/−6.9%/−10.2% vs base; no-embed warm-distinct unchanged within noise ({1.66–2.12} across builds). Sustained load: 5195 calls/20 s, 0 errors.
- **retry_condition_predicate:** Further stamp reduction is bounded by git_head file reads (kept fresh by design). Reopen ONLY if a profiler shows >=5% of worker time in read_git_head after this memo (would need a product decision on HEAD-freshness semantics) (form 3 + form 8).
- **bead_id:** (none)

### `embed-channel-rescoring-fetch-scale` (CLOSED 2026-08-26 — Door C parked; form 8 blocked on `why` contract)

- **date:** 2026-08-26
- **candidate_name:** `embed-channel-field-fetch-skip-zero-weight` (Door C)
- **target_workload:** populated index (6k chunks), embed ON: IVF engages but adaptive probes take ~90% of clusters → ~5.4k candidate chunks per query; raw-SQL probes measured `semantic_field_vectors_by_ids(5500)` ≈ 8.7 ms and `semantic_chunks_by_ids(5500)` ≈ 2.9 ms per query. The 8.7 ms is **fetch** of 5 field blobs, not decode.
- **files_touched:** `no-source-patch-attempted`
- **correctness_proof:** not-applicable (closed without implementation)
- **evidence_artifacts_paths:** idx_emb/index.db raw-SQL timings in session log; `flames_embpop.txt`; `docs/semantic-search.md` documents `embed_field:<field>=<score>`
- **baseline_configuration:** `why_terms` emits every present field; `rescore_similarity` keeps scores when Literal weights are all zero; fields fetched for ALL ranked candidates before pruning to hit_limit
- **candidate_configuration:** none built. Skipping decode while still selecting five blobs will not clear a −3% keep-gate. Door B (probe percent / top-N rescoring / columnar sidecar) stays parked: `DEFAULT_ADAPTIVE_PROBE_PERCENT = 90` is the published recall@10 ≥ 0.99 gate; top-N rescoring reorders fusion; columnar sidecar is a schema project for a 9.7 ms query tax.
- **measured_result:** CLOSED as a decode-skip candidate. Remaining query path is ~1.77 ms warm / ~11 ms embed; the live product defect was one-file edit → 48–58 s (Door A), not this 8.7 ms fetch.
- **retry_condition_predicate:** Reopen ONLY if the product `why` contract drops zero-weight field scores so those blobs need not be selected at all (form 8). Lowering probes or top-N rescoring requires a separate form-8 sign-off (recall / fusion-order).
- **bead_id:** (none)
