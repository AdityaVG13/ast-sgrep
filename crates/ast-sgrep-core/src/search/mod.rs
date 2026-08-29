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
use passes::embed::{run_embed_pass, SemanticCache};
use passes::lexical::lexical_pass;
use passes::literal::literal_pass;
use passes::regex::regex_pass;
use passes::symbol::{
    anchor_pass, anchor_pass_for_files, search_callers, search_defs, search_imports, symbol_pass,
    symbol_pass_for_files,
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
/// Cap on reported query expansions (ufk7).
const MAX_QUERY_EXPANSIONS: usize = 5;
const NL_FANOUT_SYMBOL_LIMIT: usize = 4;
const NL_FANOUT_HITS_PER_CHANNEL: usize = 16;

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
    options: SearchOptions,
    use_field_rescoring: bool,
    /// When false, skip snapshot_stamp + query_expansions. Code Mode capsules
    /// discard both; unique-hybrid p50 paid git/HEAD + lexicon expand + extra
    /// meta reads for JSON fields the model never sees.
    stamp_response: bool,
    semantic_cache: Arc<Mutex<Option<SemanticCache>>>,
    lexicon_cache: Mutex<Option<(i64, crate::lexicon::Lexicon)>>,
    response_cache: Mutex<ResponseCache>,
    /// S1: generation-keyed memo for snapshot-stamp parts that are pure
    /// functions of index contents (worktree revision + sidecar fingerprint).
    stamp_cache: Mutex<Option<(IndexGeneration, i64, Option<String>)>>,
    /// S1: drained degraded notes from the latest memoized manifest probe.
    stamp_degraded: Mutex<Vec<DegradedChannel>>,
    /// `.git/HEAD` is independent of index generation. Probe once per Searcher;
    /// index writes reopen via writer_generation.
    git_head_cache: Mutex<Option<Option<String>>>,
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
        Ok(Self::with_store(
            IndexStore::open_readonly(&options.root, options.index_path.as_deref())?,
            options,
        ))
    }
    pub fn with_store(store: IndexStore, mut options: SearchOptions) -> Self {
        // Bind SQL `f.language = ?` to Language::as_str so `--lang ts` matches
        // stored `typescript` (br-5l6). matches_lang already aliases; SQL did not.
        options.lang_filter =
            ast_sgrep_lang::Language::canonical_filter(options.lang_filter.as_deref());
        let options_identity = options.cache_identity();
        store.warm_line_corpus();
        Self {
            store,
            options,
            use_field_rescoring: true,
            stamp_response: true,
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
            git_head_cache: Mutex::new(None),
            options_identity,
        }
    }
    pub fn store(&self) -> &IndexStore {
        &self.store
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
    fn index_gen(&self) -> Option<IndexGeneration> {
        // PRAGMA failure disables caching rather than pinning gen=0 (hdwh).
        let external = self
            .store
            .connection()
            .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
            .ok()?;
        let (local, lexicon) = self.store.search_data_versions().ok()?;
        Some(IndexGeneration {
            external,
            local,
            lexicon,
        })
    }
    /// gauntlet-r5 (S1): generation-keyed memo for the expensive, purely
    /// generation-derived parts of `snapshot_stamp`. The chunk-stats scan
    /// (COUNT + MAX(length(vector)) over every semantic row), the worktree
    /// revision (MAX(mtime_secs) over files), and the sidecar fingerprint are
    /// functions of the index contents alone: any change to them is gated by
    /// a generation counter bump (external data_version or the local
    /// counters — br-yp1 semantics). `git_head` deliberately stays uncached:
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
        let (_, lexicon_generation) = self.store.search_data_versions()?;
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
                let mut guard = lock_clear_on_poison(&self.git_head_cache, |v| *v = None);
                if let Some(cached) = guard.as_ref() {
                    cached.clone()
                } else {
                    let value = read_git_head(&self.options.root);
                    *guard = Some(value.clone());
                    value
                }
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
        if !self.stamp_response {
            // Unique Code Mode never repeats a key; skip PRAGMA/gen probes
            // that cannot admit a hit.
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
            let parsed = ParsedQuery::parse(query_str);
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
                    literal_pass(&self.store, &self.options, &parsed)?
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
                            critic::apply_critic(&parsed, intent, &mut hits);
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
                )
            }
        })
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
                run_embed_pass(
                    &self.store,
                    &options,
                    expanded.as_ref().unwrap_or(&parsed),
                    &self.semantic_cache,
                    self.use_field_rescoring,
                )
            }
        }
    }
    pub fn search_semantic(&self, query_str: &str) -> Result<SearchResponse> {
        validate_query_arg(query_str)?;
        let _perf_run = crate::perf_profile::Run::start("search_semantic");
        self.cached("sem", query_str, || {
            let parsed = ParsedQuery::parse(query_str);
            let expanded = self.repository_expanded_query(&parsed)?;
            finish_response_checked(
                &parsed,
                &self.options,
                run_embed_pass(
                    &self.store,
                    &self.options,
                    expanded.as_ref().unwrap_or(&parsed),
                    &self.semantic_cache,
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
        let intent = crate::intent::classify(parsed);
        // Constraint cascade: each stage receives only files that survived the prior stage.
        let expanded = {
            let _span = crate::perf_profile::Span::start(
                "hybrid_vocab_expand",
                "search",
                "repository_expanded_query",
            );
            self.repository_expanded_query(parsed)?
        };
        let semantic_query = expanded.as_ref().unwrap_or(parsed);
        // Candidate discovery: original 3+ char terms, then repository
        // associations, then offline concept-group tokens (credential ->
        // auth/token/...). 1-2 char tokens stay out of the prefilter.
        let mut discovery = semantic_query.clone();
        if intent == crate::intent::QueryIntent::Conceptual {
            let mut extra = 0usize;
            for tok in ast_sgrep_embed::tokenize(&ast_sgrep_embed::expand_concepts(&parsed.raw)) {
                if extra >= 8 {
                    break;
                }
                if tok.chars().count() >= 3 && !discovery.terms.contains(&tok) {
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
            literal_prefilter_pass(&self.store, &self.options, &discovery)?
        };
        let mut lexical = lexical;
        let lexical_files = lexical
            .iter()
            .map(|hit| hit.file.clone())
            .collect::<HashSet<_>>();
        if lexical_files.is_empty() {
            return Ok(Vec::new());
        }

        // Structural stages keep the user's 3+ char terms (not concept
        // extras). 1-2 char tokens would LIKE '%0%' across symbols/callers.
        let mut stage_query = parsed.clone();
        stage_query.terms.retain(|term| term.chars().count() >= 3);

        // Conceptual NL skips pattern-node matching on generic tokens
        // (`query`, `graph`, `render`) which owned the unique-hybrid p99
        // shortlist. Defs/callers still run so "how does hybrid search work"
        // can rank `search_hybrid` instead of the query string in a bench
        // fixture. Empty structural falls through to lexical + embed (ht1h.3).
        let conceptual = intent == crate::intent::QueryIntent::Conceptual;
        let ast_matches = if conceptual {
            Vec::new()
        } else {
            let _span = crate::perf_profile::Span::start(
                "hybrid_structural_index",
                "search",
                "structural_index_pass",
            );
            structural_index_pass(&self.store, &self.options, &stage_query, &lexical_files)?
        };
        let mut structural = ast_matches;
        structural.extend({
            let _span = crate::perf_profile::Span::start(
                "hybrid_symbol_pass",
                "search",
                "symbol_pass_for_files",
            );
            symbol_pass_for_files(&self.store, &self.options, &stage_query, &lexical_files)?
        });
        // Identifier queries must retrieve the exact definition even when the
        // 100-file lexical cascade is full of substring coincidences.
        if let Some(spelling) = parsed.identifier_spelling() {
            if spelling.chars().count() >= 3 {
                let def_query = ParsedQuery::parse(&format!("defs:{spelling}"));
                structural.extend(search_defs(&self.store, &self.options, &def_query)?);
            }
        }
        structural.extend({
            let _span = crate::perf_profile::Span::start(
                "hybrid_anchor_pass",
                "search",
                "anchor_pass_for_files",
            );
            anchor_pass_for_files(&self.store, &self.options, &stage_query, &lexical_files)?
        });
        // Precision gate: embed only on structurally-confirmed files when
        // structural signals exist. When the structural stage is empty, the
        // lexical survivors ARE the candidate set — the semantic stage must
        // still run on them (ht1h.3 / parity: NL queries surface semantically
        // related symbols, and plain-content files stay findable).
        if conceptual {
            // Precision gating below keeps embed on structural files. Inject
            // defs for expanded identifier tokens (`follow_up`, `planner`)
            // so a leftover English term (`command`/`run`) cannot exclude
            // the module the query actually named.
            structural.extend(conceptual_concept_def_pass(
                &self.store,
                &self.options,
                &parsed.raw,
                &lexical_files,
            )?);
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
        if self.options.use_embed {
            let semantic = {
                let _span = crate::perf_profile::Span::start(
                    "hybrid_embed_pass",
                    "search",
                    "embed_pass_for_files_with_rescoring",
                );
                passes::embed::embed_pass_for_files_with_rescoring(
                    &self.store,
                    &self.options,
                    semantic_query,
                    &working_files,
                    self.use_field_rescoring,
                )?
            };
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


fn conceptual_concept_def_pass(
    store: &IndexStore,
    options: &SearchOptions,
    query: &str,
    allowed_files: &HashSet<String>,
) -> Result<Vec<SearchHit>> {
    let terms: Vec<String> = ast_sgrep_embed::tokenize(&ast_sgrep_embed::expand_concepts(query))
        .into_iter()
        .filter(|tok| {
            tok.chars().count() >= 4
                && (tok.contains('_')
                    || tok == "planner"
                    || tok == "fusion"
                    || tok == "critic"
                    || tok == "embed")
        })
        .collect();
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let parsed = ParsedQuery {
        raw: query.to_string(),
        mode: QueryMode::Hybrid,
        target: None,
        terms,
    };
    symbol_pass_for_files(store, options, &parsed, allowed_files)
}

const FANOUT_CALLER_SCALE: f64 = 0.35;

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
    if ranked.iter().any(|(affinity, _)| *affinity > 0) {
        ranked.retain(|(affinity, _)| *affinity > 0);
        ranked.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1)));
    }
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
        let caller_query = ParsedQuery::parse(&format!("callers:{symbol}"));
        let mut caller_count = 0;
        for mut hit in search_callers(store, &fanout_options, &caller_query)? {
            if hit.kind != HitKind::Caller {
                continue;
            }
            if caller_count >= NL_FANOUT_HITS_PER_CHANNEL {
                continue;
            }
            if critic::is_generic_entrypoint(hit.caller.as_deref().unwrap_or("")) {
                hit.score *= 0.25;
            }
            hit.score *= FANOUT_CALLER_SCALE;
            hits.push(hit);
            caller_count += 1;
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
) -> Result<Vec<SearchHit>> {
    // Trigram MATCH needs 3 chars. Shorter needles use literal_sql LIKE/GLOB
    // with ORDER BY over the whole `lines` table — ~22 ms on a 54k-file
    // corpus for a digit like "0". Cascade file discovery does not need them.
    let terms = parsed
        .terms
        .iter()
        .filter(|term| term.chars().count() >= 3 && !cascade_stopword(term))
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    // Unique-query literal is RAM-cheap, so score every discovery term.
    // Fill the working set from rarest postings first so a hapax in the
    // wrong file ("renewal" only in README) cannot exclude the file that
    // matches a more common sibling term ("credential" in auth.rs), while
    // still reaching rare identifiers (`rrf`, `snapshot`) that first-hit-wins
    // never saw behind English leftovers ("consistent", "across").
    let mut prefilter_options = options.clone();
    prefilter_options.case_insensitive = true;
    prefilter_options.limit = CASCADE_PREFILTER_FILE_LIMIT;
    let mut scored: Vec<Vec<SearchHit>> = Vec::new();
    for term in terms {
        let hits = literal_pass(store, &prefilter_options, &ParsedQuery::literal(term))?;
        if !hits.is_empty() {
            scored.push(hits);
        }
    }
    scored.sort_by_key(|hits| hits.len());
    let mut files = HashSet::new();
    let mut out = Vec::new();
    for hits in scored {
        for hit in hits {
            if files.insert(hit.file.clone()) {
                out.push(hit);
                if files.len() >= CASCADE_PREFILTER_FILE_LIMIT {
                    return Ok(out);
                }
            }
        }
    }
    Ok(out)
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
    for (row, signature) in
        store.pattern_nodes_matching_for_files(&signatures, lang, allowed_files)?
    {
        if !seen.insert((row.path.clone(), row.line_start, row.line_end)) {
            continue;
        }
        let term = sig_to_term.get(&signature).cloned().unwrap_or(signature);
        let excerpt = store.fill_pattern_excerpt(&row)?;
        hits.push(SearchHit::span(SpanHitInput {
            kind: HitKind::Pattern,
            file: row.path,
            line_start: row.line_start,
            line_end: row.line_end,
            score: SCORE_PATTERN * 0.85,
            excerpt,
            symbol: Some(term),
            language: row.language,
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
