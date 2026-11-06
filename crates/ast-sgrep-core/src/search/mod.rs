pub(crate) mod conjunction;
mod cascade;
pub mod critic;
pub mod field_weight;
mod finish;
mod fusion;
mod ledger;
pub mod passes;
pub(crate) mod planner;
mod scope;
mod types;
use crate::query::{ParsedQuery, QueryMode};
use crate::store::IndexStore;
use crate::Result;
use self::scope::{resolve_path_scope, validate_query_arg};
pub use critic::CriticNote;
pub use field_weight::EmbedFieldScores;
pub use finish::finish_response;
pub(crate) use finish::finish_response_checked;
pub use fusion::dedup_hits;
pub(super) use ledger::{estimate_prevented_reads, record_ledger};
pub(crate) use fusion::NameIntern;
use passes::embed::run_embed_pass_cached;
use passes::lexical::lexical_pass;
use passes::literal::literal_pass;
use passes::regex::regex_pass;
use passes::symbol::{
    anchor_pass, anchor_pass_for_files, exact_symbol_aliases, search_callers, search_defs,
    search_imports, symbol_pass, symbol_pass_for_files,
};
pub use planner::{follow_ups_for_hit, margin_is_decisive, plan_suggested_next};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
pub use types::{
    format_hit_line, hit_why, DegradedChannel, HitKind, HitSignal, QueryExpansion, SearchHit,
    SearchOptions, SearchResponse, SnapshotStamp, SpanHitInput,
};
// Re-exported so existing `crate::search::…` paths keep working untouched;
// several legs are only referenced from inside `cascade` itself.
#[allow(unused_imports)]
pub(crate) use cascade::{
    CASCADE_EXTRA_FILE_RESERVE, CASCADE_PER_TERM_FILE_LIMIT, CASCADE_PREFILTER_FILE_LIMIT,
    FANOUT_CALLER_SCALE, MAX_QUERY_EXPANSIONS, NL_FANOUT_HITS_PER_CHANNEL, NL_FANOUT_SYMBOL_LIMIT,
    cascade_stopword, conceptual_def_terms, conceptual_fanout_pass, discovery_df,
    literal_prefilter_pass, pattern_hits_for_symbol, structural_index_pass,
};

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
    cache.entries.clear();
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
    entries: Vec<(String, SearchResponse)>,
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
    /// Env knobs resolved ONCE at construction ([`crate::config::CoreConfig`]).
    /// Search-time call sites read from here, never from process env.
    config: crate::config::CoreConfig,
    use_field_rescoring: bool,
    /// When false, skip snapshot_stamp + query_expansions. Code Mode capsules
    /// discard both; unique-hybrid p50 paid git/HEAD + lexicon expand + extra
    /// meta reads for JSON fields the model never sees.
    stamp_response: bool,
    /// Generation-keyed LRU of finished responses. Independent of
    /// [`Self::stamp_response`]: Pi Code Mode repeats needles on a sticky
    /// Searcher, so cache hits must stay available when stamps are off.
    cache_responses: bool,
    response_cache: Mutex<ResponseCache>,
    /// When true, this Searcher owns a long-lived `BEGIN DEFERRED` so sticky
    /// unique searches skip per-query BEGIN/COMMIT. Writers bump
    /// `writer_generation` and Code Mode drops the Searcher.
    read_snapshot_held: Mutex<bool>,
    /// `SearchOptions::cache_identity()` is identical for the Searcher
    /// lifetime (options are frozen in `with_store`).
    options_identity: String,
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

/// Count of hybrid queries whose forked embed/fanout channel results were
/// accepted (version-coherent with the primary store). Debug/test surface:
/// proves the parallel path actually served, and that
/// `ASGREP_SERIAL_CHANNELS=1` disables it.
pub fn debug_channel_fork_count() -> u64 {
    CHANNEL_FORK_COUNT.load(std::sync::atomic::Ordering::Relaxed)
}

static CHANNEL_FORK_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// L-Q3 (plan Phase 5): `ASGREP_QEMBED_EXPANDED=1` feeds the repo-PPMI
/// expanded form to the embed lane. Paired frozen-suite measurement
/// switch only — the default stays the query as written (measured:
/// expanded embeds lost a structurally exact target; conformance ledger
/// 2026-10-17 for the armed-run verdict).
pub fn embed_query_expansion_enabled(env: Option<&str>) -> bool {
    env == Some("1")
}

/// Pure selector for the hybrid embed lane's parsed form. Identity-
/// returning on purpose (no clone): the embed path only reads terms.
/// A repo with no expansion falls back to the written form even armed.
pub fn choose_embed_query<'a>(
    parsed: &'a ParsedQuery,
    expanded: Option<&'a ParsedQuery>,
    arm: bool,
) -> &'a ParsedQuery {
    if arm {
        expanded.unwrap_or(parsed)
    } else {
        parsed
    }
}

impl Searcher {
    pub fn new(mut options: SearchOptions) -> Result<Self> {
        // Match Indexer: canonicalize roots so relative/symlink inputs share identity (0fg6/0f7r).
        options.root = crate::paths::canonicalize_path(&options.root, true).map_err(|e| {
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
        let t_sqlite = crate::env_flag::profile_clock();
        let mut store = IndexStore::open_readonly(&options.root, options.index_path.as_deref())?;
        crate::env_flag::emit_profile_stage_since("index_open_sqlite", t_sqlite);
        let t_status = crate::env_flag::profile_clock();
        let foreign_root = foreign_root_bound(&store, &options.root)?;
        crate::env_flag::emit_profile_stage_since("index_open_status", t_status);
        let inert = foreign_root;
        if foreign_root {
            store = IndexStore::open_in_memory(&options.root)?;
        }
        let mut searcher = Self::with_store_and_config(
            store,
            options,
            crate::config::CoreConfig::from_env(),
        );
        searcher.inert = inert;
        Ok(searcher)
    }
    pub fn with_store(store: IndexStore, options: SearchOptions) -> Self {
        Self::with_store_and_config(store, options, crate::config::CoreConfig::from_env())
    }
    /// Construction with an explicitly resolved [`crate::config::CoreConfig`].
    /// [`with_store`](Self::with_store) resolves `CoreConfig::from_env()`
    /// here; prefer this when the config is already in hand so env is read
    /// exactly once per Searcher.
    pub fn with_store_and_config(
        store: IndexStore,
        mut options: SearchOptions,
        config: crate::config::CoreConfig,
    ) -> Self {
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
            config,
            use_field_rescoring: true,
            stamp_response: true,
            cache_responses: true,
            response_cache: Mutex::new(ResponseCache {
                gen: IndexGeneration {
                    external: 0,
                    local: 0,
                    lexicon: 0,
                },
                entries: Vec::new(),
                order: std::collections::VecDeque::new(),
                enabled: true,
            }),
            read_snapshot_held: Mutex::new(false),
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
    /// Env knobs resolved once at construction. Search-time call sites read
    /// from here, never from process env.
    pub fn core_config(&self) -> &crate::config::CoreConfig {
        &self.config
    }
    /// Override the construction-resolved config (lets harnesses pin knobs
    /// without touching process env). Consulted lazily where read per query.
    pub fn with_core_config(mut self, config: crate::config::CoreConfig) -> Self {
        self.config = config;
        self
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
        if self.config.memo_off("warm") {
            return Ok(());
        }
        let _ = self.store.line_corpus()?;
        if self.options.use_embed {
            let parsed = ParsedQuery::parse("warm");
            let _ = run_embed_pass_cached(
                &self.store,
                &self.options,
                &parsed,
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
        conn.execute_batch("BEGIN DEFERRED")
            .map_err(|e| crate::StoreError::Other(format!("failed to pin read snapshot: {e}")))?;
        *held = true;
        Ok(())
    }
    /// Release a snapshot taken by [`Self::hold_read_snapshot`].
    pub fn release_read_snapshot(&self) {
        let mut held = lock_clear_on_poison(&self.read_snapshot_held, |h| *h = false);
        if !*held {
            return;
        }
        *held = false;
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
        lock_response_cache(&self.response_cache).entries.len()
    }
    fn snapshot_is_held(&self) -> bool {
        *lock_clear_on_poison(&self.read_snapshot_held, |_| {})
    }

    fn index_gen(&self) -> Option<IndexGeneration> {
        // A held read snapshot is immutable for the Searcher lifetime. The
        // bundle header is the generation for that snapshot; no per-query
        // PRAGMA/meta probes are needed. The SQL control arm has no header and
        // pays the probes (it is the migration arm, not the product path).
        if self.snapshot_is_held() {
            if let Ok(Some(bundle)) = self.store.read_bundle(None) {
                let generation = bundle.generation();
                return Some(IndexGeneration {
                    external: 0,
                    local: generation,
                    lexicon: generation,
                });
            }
        }
        // PRAGMA failure disables caching rather than pinning gen=0 (hdwh).
        // `data_version` only moves for *other* connections; same-connection
        // writes are visible via `search_data_versions` (index/lexicon meta).
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
        let mut fence_wall = std::time::Duration::ZERO;
        // Pin one read snapshot for multi-pass search under concurrent reindex.
        // Nested/active transactions mean an outer scope already owns the snapshot.
        // When we *should* own one (autocommit) but BEGIN fails (busy/IO), fail
        // closed rather than run unfenced and risk a silently mixed generation.
        let owns_snapshot = if conn.is_autocommit() {
            let t_begin = crate::env_flag::profile_clock();
            let owned = match conn.execute_batch("BEGIN DEFERRED") {
                Ok(()) => true,
                Err(e) => {
                    return Err(crate::StoreError::Other(format!(
                        "failed to open read snapshot for search: {e}"
                    )));
                }
            };
            if let Some(t) = t_begin {
                fence_wall += t.elapsed();
            }
            owned
        } else {
            false
        };
        let result = (|| {
            if !self.stamp_response {
                return compute();
            }
            let t_pre = crate::env_flag::profile_clock();
            let (generation_before, lexicon_generation_before) =
                self.store.search_data_versions()?;
            if let Some(t) = t_pre {
                fence_wall += t.elapsed();
            }
            let mut response = compute()?;
            let t_post = crate::env_flag::profile_clock();
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
            if let Some(t) = t_post {
                fence_wall += t.elapsed();
            }
            Ok(response)
        })();

        let t_close = crate::env_flag::profile_clock();
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
        if let Some(t) = t_close {
            fence_wall += t.elapsed();
        }
        if crate::env_flag::profile_stages_enabled() {
            crate::env_flag::emit_profile_stage("search_fence", fence_wall);
        }
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
    /// Bundle-header variant of [`Self::semantic_manifest`]. The expected
    /// fingerprint is baked at bundle build time, so the per-query path reads
    /// only the sidecar header instead of scanning `semantic_chunks`.
    fn semantic_manifest_for_stamp(
        &self,
        generation: i64,
        degraded: &mut Vec<DegradedChannel>,
    ) -> Result<Option<String>> {
        let Some(bundle) = self.store.read_bundle(Some(generation))? else {
            return Ok(self.semantic_manifest(generation, degraded));
        };
        let path = crate::semantic_ivf::semantic_ivf_path(self.store.db_path());
        if !path.exists() {
            return Ok(None);
        }
        let Some(stored) = crate::semantic_ivf::peek_semantic_ivf_fingerprint(&path) else {
            degraded.push(DegradedChannel {
                channel: "semantic".to_owned(),
                reason: "sidecar_unreadable".to_owned(),
            });
            return Ok(None);
        };
        if bundle
            .semantic_expected()
            .is_some_and(|expected| expected != stored)
        {
            degraded.push(DegradedChannel {
                channel: "semantic".to_owned(),
                reason: "sidecar_generation_mismatch".to_owned(),
            });
        }
        Ok(Some(hex32(&stored)))
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
        let _ = lexicon_generation;
        let lexicon =
            crate::lexicon::load_lexicon_for_terms(&self.store, terms).unwrap_or_default();
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
        // The bundle header carries the generation-derived worktree revision
        // and the expected ANN fingerprint. The SQL control arm falls back to
        // direct probes; neither path memoizes a stamp.
        let worktree_revision = match self.store.read_bundle(Some(generation))? {
            Some(bundle) => bundle.worktree_revision(),
            None => self.store.worktree_revision()?,
        };
        let semantic_manifest =
            self.semantic_manifest_for_stamp(generation, &mut degraded_channels)?;
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

    fn cached(
        &self,
        kind: &str,
        query: &str,
        compute: impl FnOnce() -> Result<SearchResponse>,
    ) -> Result<SearchResponse> {
        if !self.cache_responses || self.config.memo_off("response") {
            return self.fenced(compute);
        }
        let t_gen = crate::env_flag::profile_clock();
        let gen_opt = self.index_gen();
        crate::env_flag::emit_profile_stage_since("search_gen", t_gen);
        let Some(gen) = gen_opt else {
            return self.fenced(compute);
        };
        let key = self.cache_key(kind, query);
        {
            let guard = lock_response_cache(&self.response_cache);
            if guard.enabled && guard.gen == gen {
                if let Some((_, hit)) = guard.entries.iter().find(|(k, _)| k == &key) {
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
            guard.entries.clear();
            guard.order.clear();
            guard.gen = gen;
        }
        if let Some((_, cached)) = guard.entries.iter_mut().find(|(k, _)| k == &key) {
            *cached = response.clone();
        } else {
            while guard.entries.len() >= RESPONSE_CACHE_CAP {
                if let Some(old) = guard.order.pop_front() {
                    if let Some(i) = guard.entries.iter().position(|(k, _)| k == &old) {
                        guard.entries.swap_remove(i);
                    }
                } else {
                    break;
                }
            }
            guard.order.push_back(key.clone());
            guard.entries.push((key, response.clone()));
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
                self.config.ledger_path.as_deref(),
            )
        })
    }
    pub fn search_symbol_pass(&self, query_str: &str) -> Result<SearchResponse> {
        validate_query_arg(query_str)?;
        self.cached("sym", query_str, || {
            let parsed = ParsedQuery::parse(query_str);
            let mut hits = symbol_pass(&self.store, &self.options, &parsed)?;
            hits.extend(anchor_pass(&self.store, &self.options, &parsed)?);
            finish_response_checked(
                &parsed,
                &self.options,
                hits,
                true,
                self.config.ledger_path.as_deref(),
            )
        })
    }
    pub fn search(&self, query_str: &str) -> Result<SearchResponse> {
        validate_query_arg(query_str)?;
        // br-f5q: channel ATTEMPTS (alias / ≤2-edit near-miss prefixes) refuse
        // with a teaching error instead of degrading to semantic free-text
        // noise; ordinary colon text (URLs, key:value, mod::path) passes.
        if let Some(teaching) = ParsedQuery::unknown_channel_teaching(query_str) {
            return Err(crate::StoreError::Other(teaching));
        }
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
                let mut response_query = conjunction::response_query(query_str, &conj);
                resolve_path_scope(&self.options.root, &mut response_query)?;
                return finish_response_checked(
                    &response_query,
                    &self.options,
                    hits,
                    true,
                    self.config.ledger_path.as_deref(),
                );
            }
            let mut parsed = ParsedQuery::parse(query_str);
            resolve_path_scope(&self.options.root, &mut parsed)?;
            let t_vocab = crate::env_flag::profile_clock();
            let critic_vocabulary = self.critic_vocabulary(&parsed);
            crate::env_flag::emit_profile_stage_since("search_vocab", t_vocab);
            let hits = match parsed.mode {
                QueryMode::Callers => search_callers(&self.store, &self.options, &parsed)?,
                QueryMode::Defs => search_defs(&self.store, &self.options, &parsed)?,
                QueryMode::Imports => search_imports(&self.store, &self.options, &parsed)?,
                QueryMode::Pattern => crate::pattern::search_pattern_with_config(
                    parsed.terms.first().map(|s| s.as_str()).unwrap_or(""),
                    &self.store,
                    &self.options.root,
                    self.options.lang_filter.as_deref(),
                    self.options.retained_limit(),
                    &self.config,
                )?,
                QueryMode::Literal | QueryMode::Word => {
                    let mut hits = literal_pass(&self.store, &self.options, &parsed)?;
                    // A single identifier needle means "where is it defined".
                    // The line lane alone ranks incidental mentions above the
                    // declaration (measured: word:SnapshotStamp put the struct's
                    // own file 6th, behind three other files that merely use it),
                    // and the hybrid lane already merges def hits for identifier
                    // needles, so this lane does too. One row per line (§41.3):
                    // def rows lead and literal rows fill lines the defs lane
                    // missed — a bare extend doubled every def line.
                    if let Some(spelling) = parsed.identifier_spelling() {
                        if spelling.chars().count() >= 3 {
                            let def_query = ParsedQuery::parse(&format!("defs:{spelling}"));
                            let mut defs = search_defs(&self.store, &self.options, &def_query)?;
                            // Def lookup folds case independently of the line
                            // lane. Ranking enrichment must not widen a
                            // case-sensitive literal/word query's membership.
                            if !self.options.case_insensitive {
                                // Exact spelling keeps `word:Searcher` from
                                // admitting `searcher` (case-only fold). Token-
                                // equal aliases must still pass: alias lookup
                                // just admitted `auth_refresh` for
                                // `word:authRefresh`, and `== spelling` drops it.
                                let aliases = exact_symbol_aliases(spelling);
                                defs.retain(|hit| {
                                    hit.symbol.as_deref().is_some_and(|sym| {
                                        aliases.iter().any(|alias| alias == sym)
                                    })
                                });
                            }
                            if !defs.is_empty() {
                                let mut seen: std::collections::HashSet<(String, u32)> = defs
                                    .iter()
                                    .map(|h| (h.file.clone(), h.line_start))
                                    .collect();
                                let mut merged = defs;
                                for hit in hits {
                                    if seen.insert((hit.file.clone(), hit.line_start)) {
                                        merged.push(hit);
                                    }
                                }
                                hits = merged;
                            }
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
                        let phrase = crate::query::strip_wrapping_quotes(&parsed.raw);
                        literal_pass(&self.store, &self.options, &ParsedQuery::literal(phrase))?
                    } else {
                        let mut hits = self.search_hybrid(&parsed)?;
                        let t_fusion = crate::env_flag::profile_clock();
                        crate::intent::route_hits_opts(&parsed, &mut hits, !self.options.use_embed);
                        let intent = crate::intent::classify(&parsed);
                        let weights = crate::intent::weights_for(intent);
                        {
                            let _span = crate::perf_profile::Span::start(
                                "hybrid_fusion_critic",
                                "search",
                                "weighted RRF + critic",
                            );
                            let magnitudes =
                                crate::fusion::apply_weighted_rrf(&mut hits, &weights, &parsed.raw);
                            // Pre-critic fused scores: the leader clamp must
                            // distinguish boost-manufactured overrides (fusion
                            // had the leader first) from earned ones.
                            let fused: Vec<f64> = hits.iter().map(|hit| hit.score).collect();
                            critic::apply_critic(
                                &parsed,
                                intent,
                                &mut hits,
                                critic_vocabulary.as_ref(),
                            );
                            // Fusion already dropped extra votes below the
                            // decisive margin. The critic then reverses that
                            // order. On a description, put it back.
                            critic::clamp_far_cosine_on_description(
                                &parsed,
                                &mut hits,
                                &magnitudes,
                            );
                            // The same reversal happens off-description when
                            // boosts carry a corroborated hit over the embed
                            // channel's own decisively-leading answer.
                            critic::clamp_boost_overrides_of_semantic_leader(
                                &mut hits,
                                &magnitudes,
                                &fused,
                            );
                        }
                        crate::env_flag::emit_profile_stage_since("search_fusion", t_fusion);
                        hits
                    }
                }
            };
            let t_finish = crate::env_flag::profile_clock();
            let response = {
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
                    !self.stamp_response,
                    self.config.ledger_path.as_deref(),
                )
            };
            crate::env_flag::emit_profile_stage_since("search_finish", t_finish);
            response
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
                    // A merged envelope is truncated when ANY constituent
                    // pattern's answer set was cut (br-tvv).
                    acc.truncated |= response.truncated;
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
        let mut parsed = match channel {
            conjunction::ChannelQuery::Mode(parsed) => parsed.clone(),
            conjunction::ChannelQuery::Semantic(query) => ParsedQuery::parse(query),
        };
        resolve_path_scope(&self.options.root, &mut parsed)?;
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
        options.ann_probes = Some(usize::MAX);
        let mut hits = match channel {
            conjunction::ChannelQuery::Mode(_) => match parsed.mode {
                QueryMode::Callers => search_callers(&self.store, &options, &parsed),
                QueryMode::Defs => search_defs(&self.store, &options, &parsed),
                QueryMode::Imports => search_imports(&self.store, &options, &parsed),
                QueryMode::Pattern => crate::pattern::search_pattern_with_config(
                    parsed.terms.first().map(|s| s.as_str()).unwrap_or(""),
                    &self.store,
                    &self.options.root,
                    self.options.lang_filter.as_deref(),
                    options.limit,
                    &self.config,
                ),
                QueryMode::Literal | QueryMode::Word => {
                    literal_pass(&self.store, &options, &parsed)
                }
                QueryMode::Regex => regex_pass(&self.store, &options, &parsed),
                // ChannelQuery::parse never yields Hybrid; stay total anyway.
                QueryMode::Hybrid => Ok(Vec::new()),
            },
            conjunction::ChannelQuery::Semantic(_) => {
                let expanded = self.repository_expanded_query(&parsed)?;
                run_embed_pass_cached(
                    &self.store,
                    &options,
                    expanded.as_ref().unwrap_or(&parsed),
                    self.snapshot_is_held(),
                    self.use_field_rescoring,
                )
            }
        }?;
        finish::filter_hit_paths(&parsed, &options, &mut hits)?;
        Ok(hits)
    }
    pub fn search_semantic(&self, query_str: &str) -> Result<SearchResponse> {
        validate_query_arg(query_str)?;
        // br-f5q twin of the `search` guard (the semantic_only path parses
        // the same way; a typo'd channel must refuse here too).
        if let Some(teaching) = ParsedQuery::unknown_channel_teaching(query_str) {
            return Err(crate::StoreError::Other(teaching));
        }
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
                    self.snapshot_is_held(),
                    self.use_field_rescoring,
                )?,
                false,
                self.config.ledger_path.as_deref(),
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
                self.config.ledger_path.as_deref(),
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
                self.config.ledger_path.as_deref(),
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
                self.config.ledger_path.as_deref(),
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
        if let (crate::intent::QueryIntent::Conceptual, [term]) = (intent, parsed.terms.as_slice())
        {
            if term.chars().count() >= 3 && self.store.has_symbol_named(term)? {
                intent = crate::intent::QueryIntent::Symbol;
            }
        }
        // Constraint cascade: each stage receives only files that survived the prior stage.
        let t_lexical = crate::env_flag::profile_clock();
        let t_lex_vocab = crate::env_flag::profile_clock();
        let expanded = {
            let _span = crate::perf_profile::Span::start(
                "hybrid_vocab_expand",
                "search",
                "repository_expanded_query",
            );
            self.repository_expanded_query(parsed)?
        };
        // The static engine embeds the query as written; repository-vocab
        // expansion feeds lexical discovery only. Measured under the
        // expansion: `derive the next command ...` lost its target from
        // the page even after the cascade and lexical channels were cleaned.
        let semantic_query = choose_embed_query(
            parsed,
            expanded.as_ref(),
            self.config.qembed_expanded,
        );
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
        // Verb-alias enrichment (same static table + co-occurrence gate as
        // the def lane, see `user_verb_aliases`): a verb query's gold file
        // often has no literal foothold on the query's own terms
        // (`check a file against the type filter` → `types.rs::matched`),
        // so the alias tokens must join lexical FILE discovery — and as
        // USER evidence, or the cascade constrains structural stages to
        // files the un-aliased terms found and the def lane never sees the
        // target. Filtered by the same predicate the prefilter applies to
        // terms (generic/stopword tokens are dead weight there); embed
        // query stays untouched (measured regression when polluted).
        let verb_aliases = if intent == crate::intent::QueryIntent::Conceptual {
            passes::symbol::user_verb_aliases(&self.store, &parsed.terms)?
                .into_iter()
                .filter(|alias| {
                    alias.chars().count() >= 3
                        && !cascade_stopword(alias)
                        && !critic::is_generic_concept_token(alias)
                        && !discovery.terms.contains(alias)
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for alias in verb_aliases.iter() {
            discovery.terms.push(alias.clone());
        }
        // User evidence is the query's own terms. Learned borrows (and the
        // concept-group extras added below) are "extra" terms in the prefilter,
        // which reserves budget for them instead of letting them spend the
        // user's. Reading this after the borrow made learned terms look like
        // user evidence: with embeddings on, three borrowed terms consumed the
        // 100-file cascade budget and a structurally exact target fell out of
        // the cascade entirely (measured: embed-on gold MRR 0.850 vs 1.000).
        // Verb aliases ARE user evidence: they are deterministic surface
        // forms of the user's own verb, not learned associations.
        let user_discovery: HashSet<String> = if intent == crate::intent::QueryIntent::Conceptual {
            parsed
                .terms
                .iter()
                .cloned()
                .chain(verb_aliases.iter().cloned())
                .collect()
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
        // Channel fork (c1 L3): on conceptual NL queries with the static
        // backend and no held snapshot, the embed+fanout channel is independent
        // of the lexical->symbol cascade, so it runs concurrently on a sibling
        // read-only store. The primary connection is a single rusqlite handle
        // (!Sync), so the worker opens its own; WAL readers never block each
        // other and the CLI search path is already read-only. Coherence: the
        // worker records its `index_data_version`; a mismatch against the
        // primary (a concurrent writer committed mid-query) discards the
        // worker results and reruns the channel serially, so a response can
        // never mix index generations. Write guards inside the embed path
        // already skip `set_meta` on read-only stores, so the sibling persists
        // nothing.
        crate::env_flag::emit_profile_stage_since("lex_vocab", t_lex_vocab);
        let t_prefilter = crate::env_flag::profile_clock();
        let mut embed_worker = if intent == crate::intent::QueryIntent::Conceptual
            && self.options.use_embed
            && !self.inert
            && !self.snapshot_is_held()
            && self.config.parallel_channels
            && self.store.get_meta("embed_backend")?.as_deref() == Some("static")
        {
            let root = self.store.root().to_path_buf();
            let db = self.store.db_path().to_path_buf();
            let opts = self.options.clone();
            let sem_q = semantic_query.clone();
            let raw = parsed.raw.clone();
            let field = self.use_field_rescoring;
            let run_id = crate::perf_profile::current_run_id();
            Some(std::thread::spawn(move || {
                let _prev_run = crate::perf_profile::enter_run(run_id);
                let _span = crate::perf_profile::Span::start(
                    "hybrid_embed_channel_fork",
                    "search",
                    "sibling-store embed+fanout channel",
                );
                let sibling = IndexStore::open_readonly(&root, Some(&db))?;
                let version = sibling.index_data_version()?;
                let semantic = passes::embed::run_embed_pass_cached(
                    &sibling, &opts, &sem_q, false, field,
                )?;
                let fanout = conceptual_fanout_pass(&sibling, &opts, &raw, &semantic)?;
                Ok::<_, crate::StoreError>((version, semantic, fanout))
            }))
        } else {
            None
        };
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
        // Symbol-nominated alias files join the USER file set: their
        // evidence is the symbol surface (the def lane will rank it), not
        // lexical content, so they must bypass the lexical retain below.
        let mut user_files = user_files;
        if !verb_aliases.is_empty() {
            let nominated = passes::symbol::alias_nominated_files(&self.store, &verb_aliases, 64)?;
            user_files.extend(nominated);
        }
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
            crate::env_flag::emit_profile_stage_since("lex_vocab", t_lex_vocab);
            crate::env_flag::emit_profile_stage_since("lex_prefilter", t_prefilter);
            crate::env_flag::emit_profile_stage_since("search_lexical", t_lexical);
            if let Some(handle) = embed_worker.take() {
                // The escape path serves from the primary store exactly as
                // before; drain the fork so its thread ends before we return.
                let _ = handle.join();
            }
            if intent == crate::intent::QueryIntent::Conceptual && self.options.use_embed {
                let t_escape_embed = crate::env_flag::profile_clock();
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
                        self.snapshot_is_held(),
                        self.use_field_rescoring,
                    )?
                };
                crate::env_flag::emit_profile_stage_since("search_embed", t_escape_embed);
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
        // Generic tokens (`command`, `next`, `run`) are dropped only when
        // specific siblings carry the query: genericness is relative, not
        // absolute. When fewer than two specific terms survive, the generics
        // ARE the evidence — dropping them reduced "derive the next command
        // to run" to "derive" alone and the feature named `suggested_next`
        // became unreachable in every channel.
        let drop_generics = conceptual
            && parsed
                .terms
                .iter()
                .filter(|term| term.chars().count() >= 3 && !critic::is_generic_concept_token(term))
                .count()
                >= 2;
        let mut stage_query = parsed.clone();
        stage_query.terms.retain(|term| {
            term.chars().count() >= 3 && !(drop_generics && critic::is_generic_concept_token(term))
        });
        if conceptual {
            for tok in conceptual_def_terms(&parsed.raw) {
                if !stage_query.terms.contains(&tok) {
                    stage_query.terms.push(tok);
                }
            }
        }
        crate::env_flag::emit_profile_stage_since("lex_prefilter", t_prefilter);
        let t_structural = crate::env_flag::profile_clock();
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
            // Callers stay in the conceptual pool: rule 5 (critic) penalizes
            // `<module>`/`main` caller hits there, which requires them to
            // reach fusion at all.
            symbol_pass_for_files(
                &self.store,
                &self.options,
                &stage_query,
                &cascade_files,
                true,
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
        // Without embeddings the cascade has no semantic budget to protect:
        // its per-term file caps plus flat prefilter scores strand lexical
        // evidence past the page (bm25-first gold ties break by path and
        // truncate out). Merge the full FTS lexical pass so no-embed hybrid
        // is a superset of the lexical channel, never a subset. Same-line
        // duplicates merge by max score in finish dedup; route, weighted
        // RRF, and the critic below run unchanged. Embed-on path untouched.
        if !self.options.use_embed {
            hits.extend(lexical_pass(&self.store, &self.options, parsed)?);
        }
        // 2D ablation (thin-page condition): semantic evidence runs on
        // every hybrid query when embeddings are enabled. Conceptual
        // queries use it as an unconstrained candidate generator (below);
        // identifier/literal/structural intents keep the known-files-only
        // restricted pass, but no longer skip it just because the
        // structural page is already full.
        let learned_backend = self.store.get_meta("embed_backend")?.as_deref() == Some("static");
        crate::env_flag::emit_profile_stage_since("lex_structural", t_structural);
        crate::env_flag::emit_profile_stage_since("search_lexical", t_lexical);
        let t_embed = crate::env_flag::profile_clock();
        let embed_as_generator = conceptual && self.options.use_embed && learned_backend;
        if self.options.use_embed {
            // Forked channel: accept the worker's results only when its
            // `index_data_version` matches the primary's (no writer committed
            // between fork and join). Any mismatch, worker error, or worker
            // panic falls through to the serial rerun on the primary store,
            // which is byte-for-byte today's path.
            let mut forked: Option<(Vec<SearchHit>, Vec<SearchHit>)> = None;
            if let Some(handle) = embed_worker.take() {
                match handle.join() {
                    Ok(Ok((version, semantic, fanout))) => {
                        if version == self.store.index_data_version()? {
                            CHANNEL_FORK_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            forked = Some((semantic, fanout));
                        }
                    }
                    Ok(Err(_)) | Err(_) => {
                        // Worker store error or panic: serial rerun below is
                        // the authoritative path.
                    }
                }
            }
            if let Some((semantic, fanout)) = forked {
                // Same assembly order as the serial path: fan-out extends
                // before the semantic hits (fusion reads this order).
                if intent == crate::intent::QueryIntent::Conceptual {
                    hits.extend(fanout);
                }
                hits.extend(semantic);
            } else {
                let semantic = {
                    let _span = crate::perf_profile::Span::start(
                        "hybrid_embed_pass",
                        "search",
                        "embed_pass_for_files_cached",
                    );
                    let trust_snapshot = self.snapshot_is_held();
                    if embed_as_generator {
                        run_embed_pass_cached(
                            &self.store,
                            &self.options,
                            semantic_query,
                            trust_snapshot,
                            self.use_field_rescoring,
                        )?
                    } else {
                        passes::embed::embed_pass_for_files_cached(
                            &self.store,
                            &self.options,
                            semantic_query,
                            &working_files,
                            trust_snapshot,
                            self.use_field_rescoring,
                        )?
                    }
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
        }
        crate::env_flag::emit_profile_stage_since("search_embed", t_embed);
        Ok(hits)
    }
}

impl Drop for Searcher {
    fn drop(&mut self) {
        self.release_read_snapshot();
    }
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
