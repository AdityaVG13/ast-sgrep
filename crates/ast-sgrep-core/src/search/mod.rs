pub(crate) mod conjunction;
pub(crate) mod critic;
pub(crate) mod field_weight;
mod finish;
mod fusion;
pub mod passes;
pub(crate) mod planner;
mod types;
use crate::query::{ParsedQuery, QueryMode};
use crate::store::IndexStore;
use crate::Result;
pub use critic::CriticNote;
pub use field_weight::EmbedFieldScores;
pub use finish::finish_response;
pub(crate) use finish::finish_response_checked;
pub use fusion::dedup_hits;
use passes::embed::{run_embed_pass_cached, SemanticCache};
use passes::lexical::lexical_pass;
use passes::bmh::asgrep_line_hit;
use passes::literal::literal_pass;
use passes::regex::regex_pass;
use passes::symbol::{
    anchor_pass, anchor_pass_for_files, search_callers, search_defs, search_imports, symbol_pass,
    symbol_pass_for_files_warmed, WarmedSymbolTable,
};
pub use planner::{follow_ups_for_hit, margin_is_decisive, plan_suggested_next};
use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};
pub use types::{
    format_hit_line, hit_why, DegradedChannel, HitKind, HitSignal, QueryExpansion, SearchHit,
    SearchOptions, SearchResponse, SnapshotStamp, SpanHitInput,
};
const CASCADE_PREFILTER_FILE_LIMIT: usize = 100;
/// One cascade term must not spend the whole 100-file budget. `index` otherwise
/// fills CLI helpers before `durability` can add `store/mod.rs`.
const CASCADE_PER_TERM_FILE_LIMIT: usize = 32;
/// Concept extras (`embed` after user `embeddings`) keep slots after common
/// user tokens have already matched files.
const CASCADE_EXTRA_FILE_RESERVE: usize = 32;
/// Cap on reported query expansions (ufk7).
const MAX_QUERY_EXPANSIONS: usize = 5;
const NL_FANOUT_SYMBOL_LIMIT: usize = 4;
const NL_FANOUT_HITS_PER_CHANNEL: usize = 16;
/// Caller-leg fan-out hits rank below defs in fusion (Sep-7 recorded shape).
const FANOUT_CALLER_SCALE: f64 = 0.35;

/// On mutex poison, clear cached state before continuing so a panicked
/// computation cannot leave a half-written entry visible (sxjc).
fn lock_clear_on_poison<T>(mutex: &Mutex<T>, clear: impl FnOnce(&mut T)) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            mutex.clear_poison();
            let mut guard = PoisonError::into_inner(poisoned);
            clear(&mut guard);
            guard
        }
    }
}
fn invalidate_response_cache(cache: &mut ResponseCache) {
    cache.map.clear();
    cache.order.clear();
    cache.enabled = false;
    cache.gen = IndexGeneration {
        external: -1,
        local: -1,
        lexicon: -1,
    };
}
fn lock_response_cache(cache: &Mutex<ResponseCache>) -> MutexGuard<'_, ResponseCache> {
    lock_clear_on_poison(cache, invalidate_response_cache)
}
/// Hybrid search may combine adjacent committed snapshots under concurrent reindex.
/// Semantic cache drops on database generation changes; IVF fingerprint-validates with flat fallback.
#[derive(Clone, Copy, PartialEq, Eq)]
struct IndexGeneration {
    external: i64,
    local: i64,
    lexicon: i64,
}
struct ResponseCache {
    gen: IndexGeneration,
    /// Insertion-order LRU: front = oldest. Cap RESPONSE_CACHE_CAP (fj96).
    map: std::collections::HashMap<String, SearchResponse>,
    order: std::collections::VecDeque<String>,
    /// When false, PRAGMA/gen probe failed — never cache (hdwh).
    enabled: bool,
}
const RESPONSE_CACHE_CAP: usize = 128;
pub struct Searcher {
    store: IndexStore,
    /// True when `store` is the empty in-memory stand-in swapped in for an
    /// index bound to a different project root (the store answers nothing;
    /// the native walk decides). The CLI `--no-auto-index` non-empty-index
    /// gate skips this stand-in — it exists precisely so a passive foreign
    /// db degrades to walk-only instead of erroring.
    inert: bool,
    options: SearchOptions,
    use_field_rescoring: bool,
    /// When false, skip snapshot_stamp + query_expansions. Code Mode capsules
    /// discard both; unique-hybrid p50 paid git/HEAD + lexicon expand + extra
    /// meta reads for JSON fields the model never sees.
    stamp_response: bool,
    /// Generation-keyed LRU of finished responses. Independent of
    /// [`Self::stamp_response`]: Pi Code Mode repeats needles on a sticky
    /// Searcher, so cache hits must stay available when stamps are off.
    cache_responses: bool,
    semantic_cache: Arc<Mutex<Option<SemanticCache>>>,
    lexicon_cache: Mutex<Option<(i64, crate::lexicon::Lexicon)>>,
    response_cache: Mutex<ResponseCache>,
    /// S1: generation-keyed memo for snapshot-stamp parts that are pure
    /// functions of index contents (worktree revision + sidecar fingerprint).
    stamp_cache: Mutex<Option<(IndexGeneration, i64, Option<String>)>>,
    /// S1: drained degraded notes from the latest memoized manifest probe.
    stamp_degraded: Mutex<Vec<DegradedChannel>>,
    /// When true, this Searcher owns a long-lived `BEGIN DEFERRED` so sticky
    /// unique searches skip per-query BEGIN/COMMIT. Writers bump
    /// `writer_generation` and Code Mode drops the Searcher.
    read_snapshot_held: Mutex<bool>,
    /// Snapshot-held `index_gen` is immutable for the Searcher lifetime:
    /// Code Mode does not write on this connection, and other writers are
    /// invisible until the snapshot is released. Unique hybrid was paying
    /// PRAGMA + `search_data_versions` twice per query for a value warmup
    /// already observed.
    index_gen_memo: Mutex<Option<IndexGeneration>>,
    /// Warmed def rows for snapshot unique hybrid. Filled by
    /// [`Self::warm_search_path`]; CLI unique keeps the SQL LIKE fallback.
    symbol_table: Mutex<Option<WarmedSymbolTable>>,
    /// `SearchOptions::cache_identity()` is identical for the Searcher
    /// lifetime (options are frozen in `with_store`).
    options_identity: String,
}
/// Fail closed when callers request optional neural/rerank paths that were
pub fn validate_search_feature_flags(options: &SearchOptions) -> Result<()> {
    if options.use_embed && options.use_neural_embed {
        #[cfg(not(feature = "neural-embed"))]
        {
            return Err(crate::StoreError::Other(
                "--neural-embed / use_neural_embed requested but this binary was built without the `neural-embed` feature; rebuild with --features neural-embed"
                    .into(),
            ));
        }
    }
    if options.use_rerank {
        #[cfg(not(feature = "rerank"))]
        {
            return Err(crate::StoreError::Other(
                "--rerank / use_rerank requested but this binary was built without the `rerank` feature; rebuild with --features rerank"
                    .into(),
            ));
        }
    }
    Ok(())
}

/// True when the store's stamped `meta.root` names a different project root
/// than the query root (trailing slashes ignored). Unstamped stores are
/// never foreign. Shared by `new` (which propagates meta errors) and
/// `with_store` (which treats unreadable bindings as non-foreign).
fn foreign_root_bound(store: &IndexStore, root: &Path) -> Result<bool> {
    let bound = store.get_meta("root")?;
    Ok(bound.is_some_and(|bound| {
        let bound = bound.trim_end_matches('/');
        let here = root.display().to_string();
        bound != here.trim_end_matches('/')
    }))
}

impl Searcher {
    pub fn new(mut options: SearchOptions) -> Result<Self> {
        validate_search_feature_flags(&options)?;
        // Match Indexer: canonicalize roots so relative/symlink inputs share identity (0fg6/0f7r).
        options.root = options.root.canonicalize().map_err(|e| {
            crate::StoreError::Other(format!(
                "project root does not exist or is not a directory: {}: {e}",
                options.root.display()
            ))
        })?;
        if !options.root.is_dir() {
            return Err(crate::StoreError::Other(format!(
                "project root is not a directory: {}",
                options.root.display()
            )));
        }
        // Cross-surface input bounds: 0/`ASGREP_LIMIT=0` remaps; oversize clamps (CLI docs + LSP).
        options.limit = crate::limits::clamp_output_limit(Some(options.limit), 16);
        options.rerank_top_k = options
            .rerank_top_k
            .clamp(1, crate::limits::MAX_OUTPUT_RESULTS);
        options.context_before = options.context_before.min(crate::limits::MAX_EXCERPT_LINES);
        options.context_after = options.context_after.min(crate::limits::MAX_EXCERPT_LINES);
        if let Some(ref filter) = options.file_filter {
            if filter.chars().count() > crate::limits::MAX_FILE_FILTER_CHARS {
                return Err(crate::StoreError::Other(format!(
                    "file_filter exceeds maximum of {} characters",
                    crate::limits::MAX_FILE_FILTER_CHARS
                )));
            }
        }
        // Read-side root binding. `Indexer` stamps the canonical indexed
        // root into `meta.root` and cross-root REINDEX is the designed
        // prune-replace, so the last-indexed root owns the db. Answering a
        // query rooted elsewhere against those rows silently returns
        // wrong-tree paths, languages, and line corpus (both false-empty
        // and phantom hits). The foreign db is therefore swapped for an
        // EMPTY in-memory store: zero serving, zero candidate narrowing,
        // the native walk decides every hit from the query root alone.
        // (Refusing the search outright over-refuses — cases where the walk
        // answers correctly against a passive db — so the guard degrades to
        // inert instead.) A fresh db with no binding keeps today's behavior.
        let mut store = IndexStore::open_readonly(&options.root, options.index_path.as_deref())?;
        let foreign_root = foreign_root_bound(&store, &options.root)?;
        let inert = foreign_root;
        if foreign_root {
            store = IndexStore::open_in_memory(&options.root)?;
        }
        let mut searcher = Self::with_store(store, options);
        searcher.inert = inert;
        Ok(searcher)
    }
    pub fn with_store(store: IndexStore, mut options: SearchOptions) -> Self {
        // Bind SQL `f.language = ?` to Language::as_str so `--lang ts` matches
        // stored `typescript`. matches_lang already aliases; SQL did not.
        options.lang_filter =
            ast_sgrep_lang::Language::canonical_filter(options.lang_filter.as_deref());
        let options_identity = options.cache_identity();
        // The read-side root-binding predicate is bound HERE, not only in
        // `new` — a direct `with_store` caller can no longer hand a
        // foreign-root db to a searcher that would serve it as its own. Same
        // rule as `new`: a db stamped for another root starts inert; `new`
        // additionally swaps in the empty in-memory store before this runs.
        // Fresh/unreadable bindings keep the historical behavior.
        let foreign_root = foreign_root_bound(&store, &options.root).unwrap_or(false);
        // Line corpus loads only via explicit `warm_search_path`: its
        // consumers (`literal_pass`, cascade prefilter) use the cached-only
        // probe and fall back to trigram/SQL. Loading on first use
        // materialized the whole `lines ⋈ files` table plus the token index
        // on EVERY one-shot search — flamegraph 2026-09-20 put ~95% of
        // one-shot literal wall there (3/3 captures).
        Self {
            store,
            inert: foreign_root,
            options,
            use_field_rescoring: true,
            stamp_response: true,
            cache_responses: true,
            semantic_cache: Arc::new(Mutex::new(None)),
            lexicon_cache: Mutex::new(None),
            response_cache: Mutex::new(ResponseCache {
                gen: IndexGeneration {
                    external: 0,
                    local: 0,
                    lexicon: 0,
                },
                map: std::collections::HashMap::new(),
                order: std::collections::VecDeque::new(),
                enabled: true,
            }),
            stamp_cache: Mutex::new(None),
            stamp_degraded: Mutex::new(Vec::new()),
            read_snapshot_held: Mutex::new(false),
            index_gen_memo: Mutex::new(None),
            symbol_table: Mutex::new(None),
            options_identity,
        }
    }
    pub fn store(&self) -> &IndexStore {
        &self.store
    }
    /// `store` is the empty in-memory stand-in for a foreign-root index
    /// (nothing serves; the walk decides). See the `inert` field.
    pub fn store_is_inert(&self) -> bool {
        self.inert
    }
    pub fn options(&self) -> &SearchOptions {
        &self.options
    }
    /// Select concatenated-vector scoring (`false`) or the default per-field
    /// semantic rescoring (`true`). This eval-oriented setting lives on the
    /// searcher so adding it does not break exhaustive `SearchOptions` literals.
    pub fn with_field_rescoring(mut self, enabled: bool) -> Self {
        self.use_field_rescoring = enabled;
        self
    }
    pub fn with_response_stamp(mut self, enabled: bool) -> Self {
        self.stamp_response = enabled;
        self
    }
    pub fn with_response_cache(mut self, enabled: bool) -> Self {
        self.cache_responses = enabled;
        lock_response_cache(&self.response_cache).enabled = enabled;
        self
    }
    /// Pin RAM line corpus, trigram df vocab, and semantic vectors so the
    /// first unique sticky search is not the session-open tax.
    pub fn warm_search_path(&self) -> Result<()> {
        let _ = self.store.line_corpus()?;
        self.store.trigram_df().arm(&self.store);
        if let Ok(table) = WarmedSymbolTable::load(&self.store) {
            *lock_clear_on_poison(&self.symbol_table, |slot| *slot = None) = Some(table);
        }
        if self.options.use_embed {
            let parsed = ParsedQuery::parse("warm");
            let _ = run_embed_pass_cached(
                &self.store,
                &self.options,
                &parsed,
                &self.semantic_cache,
                self.snapshot_is_held(),
                self.use_field_rescoring,
            )?;
        }
        let _ = self.index_gen();
        Ok(())
    }
    /// Pin one read snapshot for the Searcher lifetime (Pi sticky unique path).
    pub fn hold_read_snapshot(&self) -> Result<()> {
        let mut held = lock_clear_on_poison(&self.read_snapshot_held, |h| *h = false);
        if *held {
            return Ok(());
        }
        let conn = self.store.connection();
        if !conn.is_autocommit() {
            return Ok(());
        }
        conn.execute_batch("BEGIN DEFERRED").map_err(|e| {
            crate::StoreError::Other(format!("failed to pin read snapshot: {e}"))
        })?;
        *held = true;
        *lock_clear_on_poison(&self.index_gen_memo, |memo| *memo = None) = None;
        Ok(())
    }
    /// Release a snapshot taken by [`Self::hold_read_snapshot`].
    pub fn release_read_snapshot(&self) {
        let mut held = lock_clear_on_poison(&self.read_snapshot_held, |h| *h = false);
        if !*held {
            return;
        }
        *held = false;
        *lock_clear_on_poison(&self.index_gen_memo, |memo| *memo = None) = None;
        let conn = self.store.connection();
        if conn.is_autocommit() {
            return;
        }
        if conn.execute_batch("COMMIT").is_err() {
            let _ = conn.execute_batch("ROLLBACK");
        }
    }
    /// Occupancy of the generation-keyed response LRU (Pi/session diagnostics).
    pub fn cached_response_count(&self) -> usize {
        lock_response_cache(&self.response_cache).map.len()
    }
    fn snapshot_is_held(&self) -> bool {
        *lock_clear_on_poison(&self.read_snapshot_held, |_| {})
    }

    fn index_gen(&self) -> Option<IndexGeneration> {
        if self.snapshot_is_held() {
            let memo = lock_clear_on_poison(&self.index_gen_memo, |slot| *slot = None);
            if let Some(gen) = *memo {
                return Some(gen);
            }
        }
        // PRAGMA failure disables caching rather than pinning gen=0 (hdwh).
        // `data_version` only moves for *other* connections; same-connection
        // writes are visible via `search_data_versions` (index/lexicon meta).
        // A held read snapshot freezes both, so later unique searches reuse
        // the first probe (Pi sticky path).
        let external = self
            .store
            .connection()
            .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
            .ok()?;
        let (local, lexicon) = self.store.search_data_versions().ok()?;
        let gen = IndexGeneration {
            external,
            local,
            lexicon,
        };
        if self.snapshot_is_held() {
            *lock_clear_on_poison(&self.index_gen_memo, |slot| *slot = None) = Some(gen);
        }
        Some(gen)
    }
    /// gauntlet-r5 (S1): generation-keyed memo for the expensive, purely
    /// generation-derived parts of `snapshot_stamp`. The chunk-stats scan
    /// (COUNT + MAX(length(vector)) over every semantic row), the worktree
    /// revision (MAX(mtime_secs) over files), and the sidecar fingerprint are
    /// functions of the index contents alone: any change to them is gated by
    /// a generation counter bump (external data_version or the local
    /// counters). `git_head` deliberately stays uncached:
    /// it reads the worktree's HEAD file and can move without any index
    /// write. Memo validity therefore keys on IndexGeneration; on any pragma
    /// failure we skip the memo entirely (fail-open to recompute, hdwh).
    fn cached_stamp_parts(&self, gen: IndexGeneration) -> Option<(i64, Option<String>)> {
        {
            let guard = lock_clear_on_poison(&self.stamp_cache, |_| {});
            if let Some((_, rev, manifest)) = guard.as_ref().filter(|(g, _, _)| *g == gen) {
                return Some((*rev, manifest.clone()));
            }
        }
        let worktree_revision = self.store.worktree_revision().ok()?;
        let mut degraded = Vec::new();
        let semantic_manifest = self.semantic_manifest_impl(&mut degraded);
        // A mismatched-sidecar verdict depends on the stored sidecar vs the
        // live stats comparison and must stay loud per query; only the
        // memo-safe parts are cached here. Unreadable-sidecar notes are
        // drained by the caller so each response reports its own probe.
        {
            let mut guard =
                lock_clear_on_poison(&self.stamp_degraded, |v: &mut Vec<DegradedChannel>| {
                    *v = Vec::new()
                });
            *guard = degraded;
        }
        {
            let mut guard = lock_clear_on_poison(&self.stamp_cache, |_| {});
            *guard = Some((gen, worktree_revision, semantic_manifest.clone()));
        }
        Some((worktree_revision, semantic_manifest))
    }
    fn cache_key(&self, kind: &str, query: &str) -> String {
        // Full SearchOptions identity (nyui).
        format!(
            "{kind}\0{query}\0{}\0fr={}",
            self.options_identity, self.use_field_rescoring
        )
    }
    /// Run one multi-pass search inside a single read snapshot and stamp the
    fn fenced(&self, compute: impl FnOnce() -> Result<SearchResponse>) -> Result<SearchResponse> {
        let conn = self.store.connection();
        // Pin one read snapshot for multi-pass search under concurrent reindex.
        // Nested/active transactions mean an outer scope already owns the snapshot.
        // When we *should* own one (autocommit) but BEGIN fails (busy/IO), fail
        // closed rather than run unfenced and risk a silently mixed generation.
        let owns_snapshot = if conn.is_autocommit() {
            match conn.execute_batch("BEGIN DEFERRED") {
                Ok(()) => true,
                Err(e) => {
                    return Err(crate::StoreError::Other(format!(
                        "failed to open read snapshot for search: {e}"
                    )));
                }
            }
        } else {
            false
        };
        let result = (|| {
            if !self.stamp_response {
                return compute();
            }
            let (generation_before, lexicon_generation_before) =
                self.store.search_data_versions()?;
            let mut response = compute()?;
            if owns_snapshot {
                let (generation_after, lexicon_generation_after) =
                    self.store.search_data_versions()?;
                if generation_after != generation_before
                    || lexicon_generation_after != lexicon_generation_before
                {
                    return Err(crate::StoreError::Other(format!(
                        "index generation changed during search \
                         (index {generation_before} -> {generation_after}, \
                          lexicon {lexicon_generation_before} -> {lexicon_generation_after}); \
                         retry for a single-generation response"
                    )));
                }
            }

            response.snapshot = self.snapshot_stamp(generation_before)?;
            response.query_expansions =
                self.query_expansions(&response.query, lexicon_generation_before);
            Ok(response)
        })();

        let close_result = if owns_snapshot {
            // A read snapshot is released either way; COMMIT is the cheap path.
            // If COMMIT fails, ROLLBACK unsticks the connection for later searches.
            if let Err(commit_error) = conn.execute_batch("COMMIT") {
                if let Err(rollback_error) = conn.execute_batch("ROLLBACK") {
                    Err(crate::StoreError::Other(format!(
                        "failed to close search snapshot: COMMIT failed: {commit_error}; \
                         cleanup ROLLBACK failed: {rollback_error}"
                    )))
                } else {
                    Err(crate::StoreError::Other(format!(
                        "failed to close search snapshot: {commit_error}"
                    )))
                }
            } else {
                Ok(())
            }
        } else {
            Ok(())
        };

        // A close failure can leave the connection unusable and therefore takes
        // precedence over a compute/read failure. Otherwise return the search result.
        close_result?;
        result
    }

    /// Fingerprint of the semantic sidecar, and whether it matches this
    fn semantic_manifest(
        &self,
        generation: i64,
        degraded: &mut Vec<DegradedChannel>,
    ) -> Option<String> {
        let path = crate::semantic_ivf::semantic_ivf_path(self.store.db_path());
        if !path.exists() {
            return None;
        }
        let Some(stored) = crate::semantic_ivf::peek_semantic_ivf_fingerprint(&path) else {
            degraded.push(DegradedChannel {
                channel: "semantic".to_owned(),
                reason: "sidecar_unreadable".to_owned(),
            });
            return None;
        };
        let expected = self.expected_semantic_fingerprint(generation);
        if expected.is_some_and(|expected| expected != stored) {
            degraded.push(DegradedChannel {
                channel: "semantic".to_owned(),
                reason: "sidecar_generation_mismatch".to_owned(),
            });
        }
        Some(hex32(&stored))
    }
    /// S1 helper: manifest probe without the generation parameter. The
    /// generation enters only through `expected_semantic_fingerprint`, which
    /// reads generation-gated stats; callers that already hold a fresh
    /// `IndexGeneration` use this variant together with `cached_stamp_parts`.
    fn semantic_manifest_impl(&self, degraded: &mut Vec<DegradedChannel>) -> Option<String> {
        let generation = self
            .store
            .search_data_versions()
            .map(|(local, _)| local)
            .unwrap_or_default();
        self.semantic_manifest(generation, degraded)
    }

    /// Fingerprint the sidecar should carry for the current snapshot (d3l5).
    fn expected_semantic_fingerprint(&self, generation: i64) -> Option<[u8; 32]> {
        // The sidecar is built over the whole corpus, so compare against
        // unfiltered stats regardless of any per-query language filter.
        let stats = self.store.semantic_chunk_stats(None).ok()?;
        if stats.count == 0 || stats.dim == 0 {
            return None;
        }
        let backend = self.store.get_meta("embed_backend").ok()?;
        Some(crate::semantic_ivf::compute_ann_fingerprint(
            stats.count,
            stats.max_id,
            stats.dim,
            backend.as_deref(),
            generation,
        ))
    }

    fn repository_associations(
        &self,
        terms: &[String],
        lexicon_generation: i64,
    ) -> Vec<crate::lexicon::Association> {
        if terms.is_empty() {
            return Vec::new();
        }
        let mut cache = lock_clear_on_poison(&self.lexicon_cache, |cached| *cached = None);
        if cache
            .as_ref()
            .is_none_or(|(cached_generation, _)| *cached_generation != lexicon_generation)
        {
            // A corrupt externally modified lexicon fails closed once per data
            // generation rather than decoding the bounded maximum on every query.
            let lexicon = crate::lexicon::load_lexicon(&self.store).unwrap_or_default();
            *cache = Some((lexicon_generation, lexicon));
        }
        let Some((_, lexicon)) = cache.as_ref() else {
            return Vec::new();
        };
        if lexicon.is_empty() {
            return Vec::new();
        }
        lexicon.expand(terms, MAX_QUERY_EXPANSIONS)
    }

    /// Repository vocabulary for the critic: the learned associations the
    /// retrieval passes already expand the query with.
    ///
    /// The critic's concept affinity read only the static concept groups, so a
    /// symbol covering a learned association (compact -> budget, 3
    /// co-occurrences) counted as single-concept while a one-token symbol
    /// counted the same -- adjudication could not see what retrieval saw. The
    /// lexicon is cached per data generation, so this shares one expansion.
    fn critic_vocabulary(&self, parsed: &ParsedQuery) -> Option<std::collections::HashSet<String>> {
        if !self.options.use_repository_vocabulary
            || crate::intent::classify(parsed) != crate::intent::QueryIntent::Conceptual
        {
            return None;
        }
        let generation = match self.index_gen() {
            Some(gen) => gen.lexicon,
            None => self.store.search_data_versions().ok()?.1,
        };
        let terms = crate::lexicon::prose_terms(&parsed.raw);
        let related: std::collections::HashSet<String> = self
            .repository_associations(&terms, generation)
            .into_iter()
            .map(|association| association.related)
            .collect();
        (!related.is_empty()).then_some(related)
    }

    /// Repository associations that apply to this query (ufk7).
    fn query_expansions(&self, query: &str, lexicon_generation: i64) -> Vec<QueryExpansion> {
        let parsed = ParsedQuery::parse(query);
        if crate::intent::classify(&parsed) == crate::intent::QueryIntent::Symbol {
            return Vec::new();
        }
        let terms = crate::lexicon::prose_terms(query);
        self.repository_associations(&terms, lexicon_generation)
            .into_iter()
            .map(|association| QueryExpansion {
                because: crate::lexicon::explain(&association),
                term: association.term,
                related: association.related,
                support: association.support,
            })
            .collect()
    }

    /// Add bounded repository vocabulary to conceptual candidate discovery and
    /// semantic scoring. The original query still owns returned lexical hits,
    /// structural matching, final scoring, and the response text.
    fn repository_expanded_query(&self, parsed: &ParsedQuery) -> Result<Option<ParsedQuery>> {
        if !self.options.use_embed
            || !self.options.use_repository_vocabulary
            || crate::intent::classify(parsed) != crate::intent::QueryIntent::Conceptual
        {
            return Ok(None);
        }
        let lexicon_generation = match self.index_gen() {
            Some(gen) => gen.lexicon,
            None => self.store.search_data_versions()?.1,
        };
        let terms = crate::lexicon::prose_terms(&parsed.raw);
        let associations = self.repository_associations(&terms, lexicon_generation);
        let mut expanded = parsed.clone();
        for association in associations {
            if !expanded.terms.contains(&association.related) {
                expanded.terms.push(association.related);
            }
        }
        if expanded.terms.len() == parsed.terms.len() {
            Ok(None)
        } else {
            Ok(Some(expanded))
        }
    }

    /// Describe the snapshot a response was read from (d3l5).
    fn snapshot_stamp(&self, generation: i64) -> Result<SnapshotStamp> {
        let mut degraded_channels = Vec::new();
        // S1: the generation-derived parts (worktree revision, sidecar
        // fingerprint via the stats scan) are memoized per IndexGeneration.
        // Fall back to the direct computation whenever the memo cannot be
        // consulted (pragma failure) so behavior only ever gets slower, never
        // different.
        let (worktree_revision, semantic_manifest) = match self.index_gen() {
            Some(gen) => self.cached_stamp_parts(gen).unwrap_or_else(|| {
                let mut degraded = Vec::new();
                (
                    self.store.worktree_revision().unwrap_or_default(),
                    self.semantic_manifest(generation, &mut degraded),
                )
            }),
            None => {
                let mut degraded = Vec::new();
                (
                    self.store.worktree_revision()?,
                    self.semantic_manifest(generation, &mut degraded),
                )
            }
        };
        degraded_channels.extend(self.take_stamp_degraded());
        Ok(SnapshotStamp {
            generation,
            schema_version: self.store.schema_version(),
            worktree_revision,
            git_head: {
                // Uncached by design: `.git/HEAD` (and the ref file it names)
                // can move without any index write, and a probe-once memo
                // reported a stale branch head for the Searcher lifetime.
                read_git_head(&self.options.root)
            },
            semantic_manifest,
            degraded_channels,
        })
    }
    /// S1: degraded-channel notes produced by the most recent memoized
    /// manifest probe (`sidecar_unreadable` only — a mismatch verdict is never
    /// memoized, see `cached_stamp_parts`). Empty when the stamp was built
    /// without the memo. The notes are drained once so each response reports
    /// exactly what its own probe observed.
    fn take_stamp_degraded(&self) -> Vec<DegradedChannel> {
        let mut guard =
            lock_clear_on_poison(&self.stamp_degraded, |v: &mut Vec<DegradedChannel>| {
                *v = Vec::new()
            });
        std::mem::take(&mut *guard)
    }

    fn cached(
        &self,
        kind: &str,
        query: &str,
        compute: impl FnOnce() -> Result<SearchResponse>,
    ) -> Result<SearchResponse> {
        if !self.cache_responses {
            return self.fenced(compute);
        }
        let Some(gen) = self.index_gen() else {
            return self.fenced(compute);
        };
        let key = self.cache_key(kind, query);
        {
            let guard = lock_response_cache(&self.response_cache);
            if guard.enabled && guard.gen == gen {
                if let Some(hit) = guard.map.get(&key) {
                    return Ok(hit.clone());
                }
            }
        }
        let response = self.fenced(compute)?;
        // Re-check generation after compute so concurrent reindex cannot poison wrong-gen (hdwh).
        let Some(gen_after) = self.index_gen() else {
            return Ok(response);
        };
        if gen_after != gen {
            return Ok(response);
        }
        let mut guard = lock_response_cache(&self.response_cache);
        if !guard.enabled {
            return Ok(response);
        }
        if guard.gen != gen {
            guard.map.clear();
            guard.order.clear();
            guard.gen = gen;
        }
        if guard.map.contains_key(&key) {
            guard.map.insert(key, response.clone());
        } else {
            while guard.map.len() >= RESPONSE_CACHE_CAP {
                if let Some(old) = guard.order.pop_front() {
                    guard.map.remove(&old);
                } else {
                    break;
                }
            }
            guard.order.push_back(key.clone());
            guard.map.insert(key, response.clone());
        }
        Ok(response)
    }
    pub fn search_lexical(&self, query_str: &str) -> Result<SearchResponse> {
        validate_query_arg(query_str)?;
        self.cached("lex", query_str, || {
            let parsed = ParsedQuery::parse(query_str);
            finish_response_checked(
                &parsed,
                &self.options,
                lexical_pass(&self.store, &self.options, &parsed)?,
                true,
            )
        })
    }
    pub fn search_symbol_pass(&self, query_str: &str) -> Result<SearchResponse> {
        validate_query_arg(query_str)?;
        self.cached("sym", query_str, || {
            let parsed = ParsedQuery::parse(query_str);
            let mut hits = symbol_pass(&self.store, &self.options, &parsed)?;
            hits.extend(anchor_pass(&self.store, &self.options, &parsed)?);
            finish_response_checked(&parsed, &self.options, hits, true)
        })
    }
    pub fn search(&self, query_str: &str) -> Result<SearchResponse> {
        validate_query_arg(query_str)?;
        let _perf_run = crate::perf_profile::Run::start("search_query");
        let _span = crate::perf_profile::Span::start(
            "search_query",
            "search",
            "Searcher::search (mode dispatch + finish)",
        );
        self.cached("search", query_str, || {
            // Two-channel conjunction (P0 channel-conjunction). Detected on
            // the raw query because a left prefix such as `callers:` would
            // otherwise claim the whole string as its target.
            if let Some(conj) = conjunction::parse(query_str) {
                let hits = conjunction::run(self, &conj)?;
                let response_query = conjunction::response_query(query_str, &conj);
                return finish_response_checked(&response_query, &self.options, hits, true);
            }
            let mut parsed = ParsedQuery::parse(query_str);
            resolve_path_scope(&self.options.root, &mut parsed)?;
            let critic_vocabulary = self.critic_vocabulary(&parsed);
            let hits = match parsed.mode {
                QueryMode::Callers => search_callers(&self.store, &self.options, &parsed)?,
                QueryMode::Defs => search_defs(&self.store, &self.options, &parsed)?,
                QueryMode::Imports => search_imports(&self.store, &self.options, &parsed)?,
                QueryMode::Pattern => crate::pattern::search_pattern(
                    parsed.terms.first().map(|s| s.as_str()).unwrap_or(""),
                    &self.store,
                    &self.options.root,
                    self.options.lang_filter.as_deref(),
                    self.options.limit,
                )?,
                QueryMode::Literal | QueryMode::Word => {
                    let mut hits = literal_pass(&self.store, &self.options, &parsed)?;
                    // A single identifier needle means "where is it defined".
                    // The line lane alone ranks incidental mentions above the
                    // declaration (measured: word:SnapshotStamp put the struct's
                    // own file 6th, behind three other files that merely use it),
                    // and the hybrid lane already merges def hits for identifier
                    // needles, so this lane does too.
                    if let Some(spelling) = parsed.identifier_spelling() {
                        if spelling.chars().count() >= 3 {
                            let def_query = ParsedQuery::parse(&format!("defs:{spelling}"));
                            hits.extend(search_defs(&self.store, &self.options, &def_query)?);
                        }
                    }
                    // The critic ran only on the hybrid lane, so every context
                    // penalty (prose/data paths, identifier collisions, folded
                    // spellings) was inert for literal/word shortlists: a JSON
                    // fixture quoting the needle and doc-comment mentions
                    // outranked the code that implements it. No fusion here --
                    // one channel, so rank order is the lane's own.
                    crate::intent::route_hits(&parsed, &mut hits);
                    let intent = crate::intent::classify(&parsed);
                    critic::apply_critic(&parsed, intent, &mut hits, critic_vocabulary.as_ref());
                    hits
                }
                QueryMode::Regex => regex_pass(&self.store, &self.options, &parsed)?,
                QueryMode::Hybrid => {
                    // Quoted → Literal intent must run phrase literal_pass (50hx).
                    if crate::intent::classify(&parsed) == crate::intent::QueryIntent::Literal {
                        let phrase = strip_wrapping_quotes(&parsed.raw);
                        literal_pass(&self.store, &self.options, &ParsedQuery::literal(phrase))?
                    } else {
                        let mut hits = self.search_hybrid(&parsed)?;
                        crate::intent::route_hits(&parsed, &mut hits);
                        let intent = crate::intent::classify(&parsed);
                        let weights = crate::intent::weights_for(intent);
                        {
                            let _span = crate::perf_profile::Span::start(
                                "hybrid_fusion_critic",
                                "search",
                                "weighted RRF + critic",
                            );
                            crate::fusion::apply_weighted_rrf(&mut hits, &weights);
                            critic::apply_critic(&parsed, intent, &mut hits, critic_vocabulary.as_ref());
                        }
                        hits
                    }
                }
            };
            {
                let _span = crate::perf_profile::Span::start(
                    "search_finish_response",
                    "search",
                    "finish_response_checked_lazy",
                );
                finish::finish_response_checked_lazy(
                    &parsed,
                    &self.options,
                    hits,
                    true,
                    Some(&self.store),
                    true,
                    !self.stamp_response,
                )
            }
        })
    }
    /// Multi-pattern ingress. Runs each pattern through the EXACT
    /// single-pattern ingress (`Searcher::search` on `pattern:<text>`),
    /// amortizing the per-process fixed cost (index open + supervisor floor,
    /// phase-13: ~19-23 ms per invocation) across N patterns, then merges the
    /// finished responses into ONE envelope: hits are grouped by pattern in
    /// argument order (per-pattern identity is `SearchHit::symbol`), byte/read
    /// estimates are summed, and `query` is the space-joined `pattern:` token
    /// list (D-03 multi-token shape). `limit` stays per-pattern — the merged
    /// hit count can reach N x limit, which is exactly what N sequential
    /// invocations return. All-or-nothing on per-pattern errors (fail-closed):
    /// a pattern the single invocation would reject rejects the whole batch.
    pub fn search_multi_pattern(&self, patterns: &[String]) -> Result<SearchResponse> {
        // Negative result: a concurrent fan-out (scoped threads, one
        // `Searcher` per worker, merge in pattern order) measured 1.02x
        // SLOWER than this sequential loop on the registered 3-pattern
        // batch with a byte-identical envelope. Each per-pattern pipeline
        // already saturates the global file-match pool, so the fan-out only
        // adds pool contention plus N extra index opens. Recorded as
        // `multi-pattern-concurrent-fanout` in PERF_NEGATIVE_RESULTS.md.
        let mut merged: Option<SearchResponse> = None;
        for raw in patterns {
            // One optional `pattern:` token per value is tolerated (D-03 shape).
            let token = raw.strip_prefix("pattern:").unwrap_or(raw);
            let response = self.search(&format!("pattern:{token}"))?;
            match &mut merged {
                None => merged = Some(response),
                Some(acc) => {
                    acc.hits.extend(response.hits);
                    acc.read_bytes_estimate = acc
                        .read_bytes_estimate
                        .saturating_add(response.read_bytes_estimate);
                    acc.returned_excerpt_bytes = acc
                        .returned_excerpt_bytes
                        .saturating_add(response.returned_excerpt_bytes);
                    acc.prevented_read_bytes = acc
                        .prevented_read_bytes
                        .saturating_add(response.prevented_read_bytes);
                    acc.query.push(' ');
                    acc.query.push_str("pattern:");
                    acc.query.push_str(token.trim());
                }
            }
        }
        merged.ok_or_else(|| crate::StoreError::Other("no patterns supplied".into()))
    }
    /// Raw hits for one side of a conjunction (P0 channel-conjunction).
    /// Dispatches exactly like `search` does for the same prefix; the
    /// semantic channel runs the embedding-only pass.
    fn channel_hits(&self, channel: &conjunction::ChannelQuery) -> Result<Vec<SearchHit>> {
        let status = self.store.status()?;
        let exhaustive_limit = status
            .line_count
            .saturating_add(status.symbol_count)
            .saturating_add(status.caller_count)
            .saturating_add(status.import_count)
            .saturating_add(status.semantic_chunk_count)
            .max(1);
        let mut options = self.options.clone();
        options.limit = exhaustive_limit;
        options.use_rerank = false;
        options.rerank_top_k = exhaustive_limit;
        options.ann_probes = Some(usize::MAX);
        match channel {
            conjunction::ChannelQuery::Mode(parsed) => match parsed.mode {
                QueryMode::Callers => search_callers(&self.store, &options, parsed),
                QueryMode::Defs => search_defs(&self.store, &options, parsed),
                QueryMode::Imports => search_imports(&self.store, &options, parsed),
                QueryMode::Pattern => crate::pattern::search_pattern(
                    parsed.terms.first().map(|s| s.as_str()).unwrap_or(""),
                    &self.store,
                    &self.options.root,
                    self.options.lang_filter.as_deref(),
                    options.limit,
                ),
                QueryMode::Literal | QueryMode::Word => literal_pass(&self.store, &options, parsed),
                QueryMode::Regex => regex_pass(&self.store, &options, parsed),
                // ChannelQuery::parse never yields Hybrid; stay total anyway.
                QueryMode::Hybrid => Ok(Vec::new()),
            },
            conjunction::ChannelQuery::Semantic(query) => {
                let parsed = ParsedQuery::parse(query);
                let expanded = self.repository_expanded_query(&parsed)?;
                run_embed_pass_cached(
                    &self.store,
                    &options,
                    expanded.as_ref().unwrap_or(&parsed),
                    &self.semantic_cache,
                    self.snapshot_is_held(),
                    self.use_field_rescoring,
                )
            }
        }
    }
    pub fn search_semantic(&self, query_str: &str) -> Result<SearchResponse> {
        validate_query_arg(query_str)?;
        let _perf_run = crate::perf_profile::Run::start("search_semantic");
        self.cached("sem", query_str, || {
            let mut parsed = ParsedQuery::parse(query_str);
            resolve_path_scope(&self.options.root, &mut parsed)?;
            let expanded = self.repository_expanded_query(&parsed)?;
            finish_response_checked(
                &parsed,
                &self.options,
                run_embed_pass_cached(
                    &self.store,
                    &self.options,
                    expanded.as_ref().unwrap_or(&parsed),
                    &self.semantic_cache,
                    self.snapshot_is_held(),
                    self.use_field_rescoring,
                )?,
                false,
            )
        })
    }
    pub fn search_literal(&self, query: &str) -> Result<SearchResponse> {
        validate_query_arg(query)?;
        self.cached("lit", query, || {
            let parsed = ParsedQuery::literal(query);
            finish_response_checked(
                &parsed,
                &self.options,
                literal_pass(&self.store, &self.options, &parsed)?,
                true,
            )
        })
    }
    pub fn search_regex(&self, query: &str) -> Result<SearchResponse> {
        validate_query_arg(query)?;
        self.cached("re", query, || {
            let parsed = ParsedQuery::regex(query);
            finish_response_checked(
                &parsed,
                &self.options,
                regex_pass(&self.store, &self.options, &parsed)?,
                true,
            )
        })
    }
    pub fn search_word(&self, query: &str) -> Result<SearchResponse> {
        validate_query_arg(query)?;
        self.cached("word", query, || {
            let parsed = ParsedQuery::word(query);
            finish_response_checked(
                &parsed,
                &self.options,
                literal_pass(&self.store, &self.options, &parsed)?,
                true,
            )
        })
    }
    fn search_hybrid(&self, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
        let mut intent = crate::intent::classify(parsed);
        // Chain seed contract (resolve_module regression): a single-term
        // query whose term names an indexed symbol exactly is an
        // identifier query even when classify() reads Conceptual — bare
        // dictionary words ("run", "test", "search") live in the
        // generic-concept vocabulary and every conceptual stage strips them,
        // collapsing hybrid to embed-only with the def channel never
        // consulted. Chain seeding then finds zero entries and emits zero
        // edges (`chain_imports_edge_resolves_for_typescript`). The symbol
        // table is ground truth; NL queries are multi-token and never take
        // this upgrade, so the unique-hybrid p99 NL class is untouched.
        if let (crate::intent::QueryIntent::Conceptual, [term]) =
            (intent, parsed.terms.as_slice())
        {
            if term.chars().count() >= 3 && self.store.has_symbol_named(term)? {
                intent = crate::intent::QueryIntent::Symbol;
            }
        }
        // Constraint cascade: each stage receives only files that survived the prior stage.
        let expanded = {
            let _span = crate::perf_profile::Span::start(
                "hybrid_vocab_expand",
                "search",
                "repository_expanded_query",
            );
            self.repository_expanded_query(parsed)?
        };
        // The local hashed backend re-encodes tokens, so feeding it a learned
        // expansion makes it encode something the user did not ask for. Ask the
        // derived backend for the query as written; a neural backend keeps the
        // expansion (it is genuinely semantic). Measured with the expansion in
        // the derived path: `derive the next command ...` lost its target from
        // the page even after the cascade and lexical channels were cleaned.
        let semantic_query = if self.options.use_neural_embed {
            expanded.as_ref().unwrap_or(parsed)
        } else {
            parsed
        };
        // Candidate discovery: original 3+ char terms, then a few concept
        // extras for zero-overlap paraphrases (throttle -> rate/limit).
        // Lexicon-expanded semantic terms stay on the embed path; stuffing
        // them into literal_prefilter was unique-hybrid p100 on NL queries.
        let mut discovery = if intent == crate::intent::QueryIntent::Conceptual {
            let mut discovery = parsed.clone();
            // Repository-learned vocabulary must reach the lexical prefilter:
            // for a conceptual query it is the only channel that admits a
            // file whose content matches just the learned identifier term
            // (repository_vocabulary_closes_a_real_cli_lexical_gap). Borrowing
            // the top-ranked associations keeps the bound that motivated
            // dropping them wholesale (a full lexicon dump here was
            // unique-hybrid p100 on NL queries), and the prefilter's df
            // guards still skip common borrows once a rare foothold exists.
            if let Some(expanded) = expanded.as_ref() {
                let mut borrowed = 0usize;
                for term in expanded.terms.iter() {
                    if borrowed >= 3 {
                        break;
                    }
                    if term.chars().count() >= 3 && !discovery.terms.contains(term) {
                        discovery.terms.push(term.clone());
                        borrowed += 1;
                    }
                }
            }
            discovery
        } else {
            semantic_query.clone()
        };
        // User evidence is the query's own terms. Learned borrows (and the
        // concept-group extras added below) are "extra" terms in the prefilter,
        // which reserves budget for them instead of letting them spend the
        // user's. Reading this after the borrow made learned terms look like
        // user evidence: with embeddings on, three borrowed terms consumed the
        // 100-file cascade budget and a structurally exact target fell out of
        // the cascade entirely (measured: embed-on gold MRR 0.850 vs 1.000).
        let user_discovery: HashSet<String> = if intent == crate::intent::QueryIntent::Conceptual {
            parsed.terms.iter().cloned().collect()
        } else {
            discovery.terms.iter().cloned().collect()
        };
        if intent == crate::intent::QueryIntent::Conceptual {
            let mut extra = discovery.terms.len().saturating_sub(parsed.terms.len());
            for tok in ast_sgrep_embed::tokenize(&ast_sgrep_embed::expand_concepts(&parsed.raw)) {
                if extra >= 3 {
                    break;
                }
                if tok.chars().count() >= 3
                    && !cascade_stopword(&tok)
                    && !critic::is_generic_concept_token(&tok)
                    && !discovery.terms.contains(&tok)
                {
                    discovery.terms.push(tok);
                    extra += 1;
                }
            }
        }
        let lexical = {
            let _span = crate::perf_profile::Span::start(
                "hybrid_lexical_prefilter",
                "search",
                "literal_prefilter_pass",
            );
            literal_prefilter_pass(
                &self.store,
                &self.options,
                &discovery,
                intent == crate::intent::QueryIntent::Conceptual,
                if intent == crate::intent::QueryIntent::Conceptual {
                    Some(&user_discovery)
                } else {
                    None
                },
            )?
        };
        let (lexical, user_files) = lexical;
        let mut lexical = lexical;
        let lexical_files = lexical
            .iter()
            .map(|hit| hit.file.clone())
            .collect::<HashSet<_>>();
        // Learned borrows may not outrank the query's own evidence inside the
        // lexical channel either. Their hits keep the channel honest about
        // rank: a borrowed term that ranks first pushes every user-term hit
        // down, and RRF reads ranks (measured: with borrows in the lexical
        // channel, `derive the next command ...` kept its target out of the
        // page; with them dropped the target returns to rank 1). Borrowing
        // still applies when the query's terms find nothing -- the
        // zero-overlap case the vocabulary exists for.
        if !user_files.is_empty() {
            lexical.retain(|hit| user_files.contains(&hit.file));
        }
        let cascade_files = if user_files.is_empty() {
            lexical_files.clone()
        } else {
            user_files
        };
        // Invent-path escape: conceptual NL with no lexical foothold still runs
        // unconstrained semantic (same path as `search_semantic`), then fan-out.
        // Identifier / literal intents stay fail-closed on empty discovery.
        if lexical_files.is_empty() {
            if intent == crate::intent::QueryIntent::Conceptual && self.options.use_embed {
                let mut hits = {
                    let _span = crate::perf_profile::Span::start(
                        "hybrid_conceptual_semantic_escape",
                        "search",
                        "run_embed_pass",
                    );
                    run_embed_pass_cached(
                        &self.store,
                        &self.options,
                        semantic_query,
                        &self.semantic_cache,
                        self.snapshot_is_held(),
                        self.use_field_rescoring,
                    )?
                };
                let _span = crate::perf_profile::Span::start(
                    "hybrid_conceptual_fanout_escape",
                    "search",
                    "conceptual_fanout_pass",
                );
                hits.extend(conceptual_fanout_pass(
                    &self.store,
                    &self.options,
                    &parsed.raw,
                    &hits,
                )?);
                return Ok(hits);
            }
            return Ok(Vec::new());
        }

        // Structural stages keep the user's 3+ char terms (not concept
        // extras). 1-2 char tokens would LIKE '%0%' across symbols/callers.
        // Conceptual NL skips pattern-node matching on generic tokens
        // (`query`, `graph`, `render`) which owned the unique-hybrid p99
        // shortlist. Defs/callers still run so "how does hybrid search work"
        // can rank `search_hybrid` instead of the query string in a bench
        // fixture. Empty structural falls through to lexical + embed (ht1h.3).
        let conceptual = intent == crate::intent::QueryIntent::Conceptual;
        let mut stage_query = parsed.clone();
        stage_query.terms.retain(|term| {
            term.chars().count() >= 3 && !(conceptual && critic::is_generic_concept_token(term))
        });
        if conceptual {
            for tok in conceptual_def_terms(&parsed.raw) {
                if !stage_query.terms.contains(&tok) {
                    stage_query.terms.push(tok);
                }
            }
        }
        let ast_matches = if conceptual {
            Vec::new()
        } else {
            let _span = crate::perf_profile::Span::start(
                "hybrid_structural_index",
                "search",
                "structural_index_pass",
            );
            structural_index_pass(&self.store, &self.options, &stage_query, &cascade_files)?
        };
        let mut structural = ast_matches;
        structural.extend({
            let _span = crate::perf_profile::Span::start(
                "hybrid_symbol_pass",
                "search",
                "symbol_pass_for_files",
            );
            let warmed = lock_clear_on_poison(&self.symbol_table, |slot| *slot = None).clone();
            // Callers stay in the conceptual pool: rule 5 (critic) penalizes
            // `<module>`/`main` caller hits there, which requires them to
            // reach fusion at all.
            symbol_pass_for_files_warmed(
                &self.store,
                &self.options,
                &stage_query,
                &cascade_files,
                true,
                warmed.as_ref(),
            )?
        });
        // Identifier queries must retrieve the exact definition even when the
        // 100-file lexical cascade is full of substring coincidences.
        if let Some(spelling) = parsed.identifier_spelling() {
            if spelling.chars().count() >= 3 {
                let def_query = ParsedQuery::parse(&format!("defs:{spelling}"));
                structural.extend(search_defs(&self.store, &self.options, &def_query)?);
                // Whole-word occurrences of the exact spelling, but only when
                // the query IS the identifier: SCREAMING/snake_case tokens
                // fragment into noisy terms that match unrelated comments
                // (VT_LMHEAD_FP8 vs VT-MATMUL-FP8-BLOCK-*). Multi-word natural
                // queries keep their expansion/fusion path or literal hits
                // crowd out the semantic lane (invent_path gold).
                if parsed.raw.trim() == spelling {
                    let word_query = ParsedQuery::parse(&format!("word:{spelling}"));
                    structural.extend(literal_pass(&self.store, &self.options, &word_query)?);
                }
            }
        }
        if !conceptual {
            structural.extend({
                let _span = crate::perf_profile::Span::start(
                    "hybrid_anchor_pass",
                    "search",
                    "anchor_pass_for_files",
                );
                anchor_pass_for_files(&self.store, &self.options, &stage_query, &cascade_files)?
            });
        }
        let structural_files = structural
            .iter()
            .map(|hit| hit.file.clone())
            .collect::<HashSet<_>>();
        let working_files = if structural_files.is_empty() {
            lexical_files
        } else {
            structural_files
        };

        lexical.retain(|hit| working_files.contains(&hit.file));
        let mut hits = lexical;
        hits.extend(structural);
        // Semantic evidence is a fallback, not a co-equal voter. The local
        // hashed backend re-encodes the same text, so it cannot outvote
        // structural evidence, and its *presence* measurably displaced
        // structurally exact hits: embed-on gold MRR 0.850 vs 1.000 with the
        // channel off, unchanged at semantic weight 0.001 (presence, not
        // weight, was the lever) and unchanged when the vocabulary left the
        // cascade. It still runs when structural + lexical evidence leaves the
        // page thin -- the zero-overlap case it exists for.
        let structural_evidence_is_thin = hits.len() < self.options.limit;
        if self.options.use_embed && structural_evidence_is_thin {
            let semantic = {
                let _span = crate::perf_profile::Span::start(
                    "hybrid_embed_pass",
                    "search",
                    "embed_pass_for_files_with_rescoring",
                );
                let trust_snapshot = self.snapshot_is_held();
                passes::embed::embed_pass_for_files_cached(
                    &self.store,
                    &self.options,
                    semantic_query,
                    &working_files,
                    Some(&self.semantic_cache),
                    trust_snapshot,
                    self.use_field_rescoring,
                )?
            };
            // Fan-out is unconditional for conceptual queries: it is the sole
            // source of caller/graph/pattern contributor evidence, so skipping
            // it when a def hit exists silently strips call-site provenance
            // from the ranked response (cli_smoke conceptual_query_fans_out).
            if intent == crate::intent::QueryIntent::Conceptual {
                let _span = crate::perf_profile::Span::start(
                    "hybrid_conceptual_fanout",
                    "search",
                    "conceptual_fanout_pass",
                );
                hits.extend(conceptual_fanout_pass(
                    &self.store,
                    &self.options,
                    &parsed.raw,
                    &semantic,
                )?);
            }
            hits.extend(semantic);
        }
        Ok(hits)
    }
}

impl Drop for Searcher {
    fn drop(&mut self) {
        self.release_read_snapshot();
    }
}

fn conceptual_def_terms(query: &str) -> Vec<String> {
    // Snake_case from expansion can name a def (`intern_paths`). Other tokens
    // must appear in the user query: dumping a whole CONCEPT_GROUPS list into
    // symbol LIKE was unique-hybrid p100 (auth NL) and a rarity cap on that
    // dump ranked auth_refresh for "durable session write".
    let user: HashSet<String> = ast_sgrep_embed::tokenize(query).into_iter().collect();
    let mut terms: Vec<String> = ast_sgrep_embed::tokenize(&ast_sgrep_embed::expand_concepts(query))
        .into_iter()
        .filter(|tok| {
            tok.chars().count() >= 4
                && !critic::is_generic_concept_token(tok)
                && (tok.contains('_')
                    || user.contains(tok)
                    || user.iter().any(|u| u.contains(tok.as_str())))
        })
        .collect();
    terms.sort();
    terms.dedup();
    terms.truncate(8);
    terms
}

fn conceptual_fanout_pass(
    store: &IndexStore,
    options: &SearchOptions,
    query: &str,
    semantic: &[SearchHit],
) -> Result<Vec<SearchHit>> {
    let query_tokens: HashSet<String> =
        ast_sgrep_embed::tokenize(&ast_sgrep_embed::expand_concepts(query))
            .into_iter()
            .collect();
    let mut seen_symbols = HashSet::new();
    let mut ranked = Vec::new();
    for hit in semantic {
        let Some(symbol) = hit.symbol.as_deref() else {
            continue;
        };
        if symbol.is_empty() || critic::is_generic_entrypoint(symbol) {
            continue;
        }
        if !seen_symbols.insert(symbol.to_lowercase()) {
            continue;
        }
        let affinity = critic::identifier_tokens(symbol)
            .into_iter()
            .filter(|token| query_tokens.contains(token))
            .count();
        ranked.push((affinity, symbol));
    }
    if !ranked.iter().any(|(affinity, _)| *affinity > 0) {
        // Zero-affinity defs (auth_refresh for "combine two search channels"
        // on a tiny fixture) were unique-hybrid p100: two extra search_defs
        // SQL round-trips that cannot help ranking.
        return Ok(Vec::new());
    }
    ranked.retain(|(affinity, _)| *affinity > 0);
    ranked.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1)));
    let symbols = ranked
        .into_iter()
        .map(|(_, symbol)| symbol)
        .take(NL_FANOUT_SYMBOL_LIMIT);
    let mut hits = Vec::new();
    let mut fanout_options = options.clone();
    fanout_options.limit = NL_FANOUT_HITS_PER_CHANNEL;
    for symbol in symbols {
        let def_query = ParsedQuery::parse(&format!("defs:{symbol}"));
        hits.extend(search_defs(store, &fanout_options, &def_query)?);
        // Caller/graph leg: the pinned conceptual contract (cli_smoke
        // conceptual_query_fans_out) requires call-site contributors — a
        // callee name is unreachable by term match from prose, so this
        // affinity-driven leg is the only source of that evidence. The
        // callers query materializes the direct Caller row and its Graph
        // edge side by side; both must survive so fusion can list the full
        // contributor set on the merged same-line hit.
        let caller_query = ParsedQuery::parse(&format!("callers:{symbol}"));
        let mut caller_count = 0usize;
        let mut graph_count = 0usize;
        for mut hit in search_callers(store, &fanout_options, &caller_query)? {
            let count = match hit.kind {
                HitKind::Caller => &mut caller_count,
                HitKind::Graph => &mut graph_count,
                _ => continue,
            };
            if *count >= NL_FANOUT_HITS_PER_CHANNEL {
                continue;
            }
            if critic::is_generic_entrypoint(hit.caller.as_deref().unwrap_or("")) {
                hit.score *= 0.25;
            }
            hit.score *= FANOUT_CALLER_SCALE;
            hits.push(hit);
            *count += 1;
        }
        hits.extend(pattern_hits_for_symbol(
            store,
            options.lang_filter.as_deref(),
            &symbol,
        )?);
    }
    Ok(hits)
}

/// Indexed pattern nodes matching the fan-out symbol's structural signatures —
/// the `pattern` contributor leg of the conceptual fan-out (bounded per
/// channel; deduped by location).
fn pattern_hits_for_symbol(
    store: &IndexStore,
    lang_filter: Option<&str>,
    symbol: &str,
) -> Result<Vec<SearchHit>> {
    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    for signature in ast_sgrep_lang::structural_term_signatures(symbol) {
        let remaining = NL_FANOUT_HITS_PER_CHANNEL.saturating_sub(hits.len());
        if remaining == 0 {
            break;
        }
        for row in store.pattern_nodes_matching_limited(&signature, lang_filter, remaining)? {
            if !seen.insert((row.path.clone(), row.line_start, row.line_end)) {
                continue;
            }
            let excerpt = store.fill_pattern_excerpt(&row)?;
            hits.push(SearchHit::span(SpanHitInput {
                kind: HitKind::Pattern,
                file: row.path,
                line_start: row.line_start,
                line_end: row.line_end,
                score: crate::rank::SCORE_PATTERN * 0.85,
                excerpt,
                symbol: Some(symbol.to_owned()),
                language: row.language,
                byte_span: None,
            }));
        }
    }
    Ok(hits)
}


fn cascade_stopword(term: &str) -> bool {
    // English function words that match too many files and stall discovery
    // on the first 3+ character token ("how does hybrid search work").
    matches!(
        term.to_ascii_lowercase().as_str(),
        "how"
            | "does"
            | "the"
            | "and"
            | "for"
            | "with"
            | "from"
            | "that"
            | "this"
            | "what"
            | "where"
            | "when"
            | "why"
            | "who"
            | "are"
            | "was"
            | "were"
            | "into"
            | "about"
            | "than"
            | "then"
            | "them"
            | "they"
            | "have"
            | "has"
            | "had"
            | "but"
            | "can"
            | "could"
            | "would"
            | "should"
            | "will"
            | "also"
    )
}

fn literal_prefilter_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    drop_common: bool,
    user_first: Option<&HashSet<String>>,
) -> Result<(Vec<SearchHit>, HashSet<String>)> {
    // Trigram MATCH needs 3 chars. Shorter needles use literal_sql LIKE/GLOB
    // with ORDER BY over the whole `lines` table — ~22 ms on a 54k-file
    // corpus for a digit like "0". Cascade file discovery does not need them.
    let mut terms = parsed
        .terms
        .iter()
        .filter(|term| {
            term.chars().count() >= 3
                && !cascade_stopword(term)
                && !(drop_common && critic::is_generic_concept_token(term))
        })
        .collect::<Vec<_>>();
    if drop_common {
        // High-df English (encode/payload/search) owned unique-hybrid p90:
        // each term still ran a full literal_pass. Rare siblings keep the
        // cascade; all-common conceptual queries go empty so semantic escape
        // can run (sub-1ms IVF) instead of a 100-file junk shortlist.
        const CASCADE_COMMON_DF: i64 = 2048;
        let rare: Vec<_> = terms
            .iter()
            .copied()
            .filter(|term| match store.trigram_df().min_df(store, term) {
                Some(df) if df > CASCADE_COMMON_DF => false,
                _ => true,
            })
            .collect();
        if rare.is_empty() {
            return Ok((Vec::new(), HashSet::new()));
        }
        terms = rare;
    }
    if terms.is_empty() {
        return Ok((Vec::new(), HashSet::new()));
    }
    // Rarest-trigram-df first, then stop at the file cap. Scanning every
    // leftover English term and *then* merging is equivalent for the 100
    // files kept, and it was the unique-hybrid p90: common terms still paid
    // a full literal_pass after the shortlist was already full.
    terms.sort_by(|left, right| {
        let left_extra = user_first.is_some_and(|user| !user.contains(*left));
        let right_extra = user_first.is_some_and(|user| !user.contains(*right));
        left_extra
            .cmp(&right_extra)
            .then_with(|| {
                discovery_df(store, left).cmp(&discovery_df(store, right))
            })
            .then_with(|| left.cmp(right))
    });
    // Cached-only (one-shot literal discipline): the prefilter already
    // handles `None` via the per-term `literal_pass` fallback below.
    let corpus = store.line_corpus_if_cached()?;
    let file_cap = match corpus.as_ref() {
        Some(corpus) => CASCADE_PREFILTER_FILE_LIMIT.min(corpus.file_count().max(1)),
        None => CASCADE_PREFILTER_FILE_LIMIT,
    };
    let mut files = HashSet::new();
    // Files the query's own terms found. Learned borrows may add lexical
    // candidates, but the structural passes constrain to this set when it is
    // non-empty: a borrowed term must not spend the cascade budget that the
    // user's evidence needs (measured: embed-on gold MRR 0.902 -> 0.873 and one
    // structurally exact target fell out of the def pool when borrows widened
    // the constraint).
    let mut user_files = HashSet::new();
    let mut out = Vec::new();
    let extra_reserve = if user_first.is_some() && file_cap > CASCADE_PER_TERM_FILE_LIMIT {
        CASCADE_EXTRA_FILE_RESERVE
            .min(file_cap / 4)
            .min(file_cap.saturating_sub(CASCADE_PER_TERM_FILE_LIMIT))
    } else {
        0
    };
    let user_budget = file_cap.saturating_sub(extra_reserve);
    if let Some(corpus) = corpus.as_ref() {
        for term in terms.iter().copied().filter(|term| term.is_ascii()) {
            let extra = user_first.is_some_and(|user| !user.contains(term));
            let budget = if extra { file_cap } else { user_budget };
            let remaining = budget.saturating_sub(files.len());
            if remaining == 0 {
                continue;
            }
            // After a rare foothold, common leftovers only add junk files and
            // memchr the rest of the packed corpus. Invent-path extras stay
            // rare (df well under this on the mini fixture).
            const CASCADE_FOOTHOLD_DF: i64 = 128;
            // Missing df sorts last, but must still scan: invent-path "session"
            // after a README "durable" foothold has no trigram row.
            if !files.is_empty()
                && !extra
                && store
                    .trigram_df()
                    .min_df(store, term)
                    .is_some_and(|df| df > CASCADE_FOOTHOLD_DF)
            {
                continue;
            }
            let term_cap = remaining.min(CASCADE_PER_TERM_FILE_LIMIT);
            for row in corpus.scan_distinct_files_cs(term, term_cap, &files) {
                if files.insert(row.path.to_string()) {
                    if !extra {
                        user_files.insert(row.path.to_string());
                    }
                    out.push(asgrep_line_hit(
                        row.path.to_string(),
                        row.language.map(str::to_string),
                        row.line_no,
                        row.content.to_string(),
                        1.0,
                    ));
                    if files.len() >= file_cap {
                        break;
                    }
                }
            }
        }
        if files.len() >= file_cap || terms.iter().all(|term| term.is_ascii()) {
            return Ok((out, user_files));
        }
    }
    let mut prefilter_options = options.clone();
    prefilter_options.case_insensitive = true;
    prefilter_options.limit = file_cap;
    for term in terms {
        let extra = user_first.is_some_and(|user| !user.contains(term.as_str()));
        let hits = literal_pass(store, &prefilter_options, &ParsedQuery::literal(term))?;
        for mut hit in hits {
            // Presence semantics, matching the corpus branch above (score
            // 1.0 per distinct file): this funnel feeds fusion, and a
            // rank-decay here would score identical file sets differently
            // on cold (this fallback) vs warmed (corpus) sessions.
            hit.score = 1.0;
            if files.insert(hit.file.clone()) {
                if !extra {
                    user_files.insert(hit.file.clone());
                }
                out.push(hit);
                if files.len() >= file_cap {
                    return Ok((out, user_files));
                }
            }
        }
    }
    Ok((out, user_files))
}

fn discovery_df(store: &IndexStore, term: &str) -> i64 {
    // Missing df must sort LAST (common), never first. unwrap_or(0) made
    // unknown terms look rarest and fill the 100-file cap before "durability".
    store.trigram_df().min_df(store, term).unwrap_or(i64::MAX)
}

/// Boost hybrid recall with pre-indexed pattern_nodes (decls/calls extracted at index time).
fn structural_index_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    allowed_files: &HashSet<String>,
) -> Result<Vec<SearchHit>> {
    use crate::rank::SCORE_PATTERN;
    use crate::search::types::{HitKind, SpanHitInput};
    let lang = options.lang_filter.as_deref();
    let mut sig_to_term = HashMap::<String, String>::new();
    for term in &parsed.terms {
        if term.len() < 3 || !term.chars().all(|c| c == '_' || c.is_alphanumeric()) {
            continue;
        }
        for sig in ast_sgrep_lang::structural_term_signatures(term) {
            sig_to_term.entry(sig).or_insert_with(|| term.clone());
        }
    }
    if sig_to_term.is_empty() {
        return Ok(Vec::new());
    }
    let signatures: Vec<String> = sig_to_term.keys().cloned().collect();
    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    let rows = {
        let _span = crate::perf_profile::Span::start(
            "hybrid_structural_sql",
            "search",
            "pattern_nodes_matching_for_files",
        );
        // Same row budget as the symbol/caller/anchor passes: enough to score
        // inside the 100-file cascade, never the whole node population.
        let pattern_row_budget = crate::search::passes::bmh::retained_limit(options).max(32).min(500);
        store.pattern_nodes_matching_for_files(&signatures, lang, allowed_files, pattern_row_budget)?
    };
    for (row, signature) in rows {
        if !seen.insert((row.path.clone(), row.line_start, row.line_end)) {
            continue;
        }
        let term = sig_to_term.get(&signature).cloned().unwrap_or(signature);
        // Excerpts attach lazily in finish (attach_indexed_excerpts_if_empty) for
        // the survivors only. Fetching one indexed excerpt per pattern node here
        // ran a SQL query for rows fusion then discarded — measured p50 244us /
        // p90 691us inside the search. Pattern nodes store an empty excerpt, so a
        // survivor gets the identical string from the same call, once.
        let excerpt = row.excerpt.clone();
        hits.push(SearchHit::span(SpanHitInput {
            kind: HitKind::Pattern,
            file: row.path,
            line_start: row.line_start,
            line_end: row.line_end,
            score: SCORE_PATTERN * 0.85,
            excerpt,
            symbol: Some(term),
            language: row.language,
            byte_span: None,
        }));
    }
    Ok(hits)
}
fn estimate_prevented_reads(root: &Path, hits: &[SearchHit]) -> (u64, u64, u64) {
    use std::path::Component;
    use std::sync::OnceLock;
    const META_CACHE_CAP: usize = 4_096;
    static META_CACHE: OnceLock<Mutex<std::collections::HashMap<String, u64>>> = OnceLock::new();
    let cache = META_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut files = HashSet::new();
    let mut read_bytes_estimate = 0u64;
    {
        let mut guard = lock_clear_on_poison(cache, |map| map.clear());
        for h in hits {
            if !files.insert(h.file.as_str()) {
                continue;
            }
            // Sanitize: reject absolute escapes and parent-directory joins (89er).
            let hit_path = Path::new(&h.file);
            if hit_path.is_absolute()
                || hit_path.components().any(|c| {
                    matches!(
                        c,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                })
            {
                continue;
            }
            let key = root.join(hit_path).to_string_lossy().into_owned();
            let len = if let Some(&n) = guard.get(&key) {
                n
            } else {
                let n = std::fs::metadata(&key).ok().map(|m| m.len()).unwrap_or(0);
                if guard.len() >= META_CACHE_CAP {
                    // Bound growth: drop arbitrary entry when full.
                    if let Some(evict) = guard.keys().next().cloned() {
                        guard.remove(&evict);
                    }
                }
                guard.insert(key, n);
                n
            };
            read_bytes_estimate += len;
        }
    }
    let returned_excerpt_bytes = hits.iter().map(|h| h.excerpt.len() as u64).sum();
    (
        read_bytes_estimate,
        returned_excerpt_bytes,
        read_bytes_estimate.saturating_sub(returned_excerpt_bytes),
    )
}
fn record_ledger_from_env(response: &SearchResponse) {
    let Some(path) = std::env::var_os("ASGREP_LEDGER_PATH") else {
        return;
    };
    let path = Path::new(&path);
    // Constrain ledger writes: absolute path required; no `..`; must stay under cwd (5xf2).
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        eprintln!("[asgrep] ignoring ASGREP_LEDGER_PATH: must be an absolute path without '..'");
        return;
    }
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    let Ok(cwd) = cwd.canonicalize() else {
        return;
    };
    let parent = path.parent().unwrap_or(path);
    let Ok(parent_canon) = parent.canonicalize() else {
        // Parent may not exist yet; require it under cwd by prefix check on the raw absolute path.
        if !path.starts_with(&cwd) {
            eprintln!("[asgrep] ignoring ASGREP_LEDGER_PATH: outside process cwd");
            return;
        }
        try_append_ledger(path, response);
        return;
    };
    if !parent_canon.starts_with(&cwd) {
        eprintln!("[asgrep] ignoring ASGREP_LEDGER_PATH: outside process cwd");
        return;
    }
    try_append_ledger(path, response);
}
/// Best-effort ledger append: search must not fail, but write errors are visible.
fn try_append_ledger(path: &Path, response: &SearchResponse) {
    if let Err(e) = append_ledger_entry(path, response) {
        eprintln!(
            "[asgrep] warning: failed to write ASGREP_LEDGER_PATH {}: {e}",
            path.display()
        );
    }
}
fn append_ledger_entry(path: &Path, response: &SearchResponse) -> std::io::Result<()> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut line = serde_json::to_vec(&serde_json::json!({
        "ts": ts, "query": response.query, "hits": response.hits.len(), "bytes": {
            "read_bytes_estimate": response.read_bytes_estimate, "returned_excerpt_bytes": response.returned_excerpt_bytes,
            "prevented_read_bytes": response.prevented_read_bytes, },
    })).map_err(std::io::Error::other)?;
    line.push(b'\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(&line)
}
/// Resolve an `in:` scope against the index root BEFORE any walk. A
/// parse-time scope error refuses loudly (the
/// historic silent drop ran the query unscoped); a scope that exists on disk
/// as a FILE pins the exact file (`path_scope_exact`), a directory keeps the
/// `dir/**` glob, and a path matching nothing under the root refuses with a
/// diagnostic instead of answering silent-empty. Wildcard scopes bypass disk
/// resolution (their glob is the filter, existence of individual matches is
/// the walk's own business).
fn resolve_path_scope(root: &std::path::Path, parsed: &mut ParsedQuery) -> Result<()> {
    if let Some(err) = parsed.path_scope_error.as_deref() {
        return Err(crate::StoreError::Other(err.to_string()));
    }
    let Some(scope) = parsed.path_scope.as_deref() else {
        return Ok(());
    };
    if scope.contains('*') || scope.contains('?') {
        return Ok(());
    }
    let target = root.join(scope);
    if target.is_file() {
        parsed.path_scope_exact = true;
    } else if !target.is_dir() {
        return Err(crate::StoreError::Other(format!(
            "in: scope '{scope}' matches no file or directory under {}",
            root.display()
        )));
    }
    Ok(())
}

fn compile_glob(pattern: &str) -> std::result::Result<regex::Regex, String> {
    if pattern.is_empty() {
        return Err("file_filter must be non-empty".into());
    }
    if pattern.chars().count() > crate::limits::MAX_FILE_FILTER_CHARS {
        return Err(format!(
            "file_filter exceeds maximum of {} characters",
            crate::limits::MAX_FILE_FILTER_CHARS
        ));
    }
    if pattern
        .chars()
        .any(|c| c == '\0' || (c.is_control() && c != '\t'))
    {
        return Err("file_filter contains invalid control characters".into());
    }
    let mut result = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    chars.next();
                    result.push_str("(?:.*/)?");
                } else {
                    result.push_str(".*");
                }
            }
            '*' => result.push_str(".*"),
            '?' => result.push('.'),
            c if "\\.+()|[]{}^$".contains(c) => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }
    result.push('$');
    regex::Regex::new(&result).map_err(|e| e.to_string())
}
fn strip_wrapping_quotes(raw: &str) -> &str {
    let t = raw.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        &t[1..t.len() - 1]
    } else {
        t
    }
}

fn validate_query_arg(query: &str) -> Result<()> {
    crate::limits::validate_query_len(query).map_err(crate::StoreError::Other)
}

/// Resolve `.git/HEAD` to a commit id without spawning git (d3l5).
fn read_git_head(root: &std::path::Path) -> Option<String> {
    let git_dir = root.join(".git");
    // Only a real in-workspace .git directory is consulted. Following a
    // worktree `gitdir:` pointer would let untrusted workspace content nominate
    // arbitrary ambient files for inclusion in the search response.
    let git = crate::io_bounds::RootDir::open(&git_dir).ok()?;
    let head = git.read_text_capped(Path::new("HEAD"), 4 * 1024).ok()?;
    let head = head.text.trim();
    match head.strip_prefix("ref:") {
        Some(reference) => {
            let reference = reference.trim();
            let path = Path::new(reference);
            if !reference.starts_with("refs/")
                || path
                    .components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_)))
            {
                return None;
            }
            let direct = git.read_text_capped(path, 4 * 1024).ok()?;
            valid_git_object_id(direct.text.trim()).then(|| direct.text.trim().to_ascii_lowercase())
        }
        // Detached HEAD already holds the id.
        None => valid_git_object_id(head).then(|| head.to_ascii_lowercase()),
    }
}

fn valid_git_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Lowercase hex for a 32-byte digest (d3l5).
fn hex32(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}
