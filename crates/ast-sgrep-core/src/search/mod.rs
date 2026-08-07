pub mod passes;
mod types;
use crate::query::{ParsedQuery, QueryMode};
use crate::store::IndexStore;
use crate::Result;
use passes::embed::{
    embed_pass_for_files, run_embed_pass, SemanticCache,
};
use passes::lexical::lexical_pass;
use passes::literal::literal_pass;
use passes::regex::regex_pass;
use passes::symbol::{
    anchor_pass, anchor_pass_for_files, search_callers, search_defs, search_imports, symbol_pass,
    symbol_pass_for_files,
};
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};
use types::{assign_signal_margins, dedup_hits};
pub use types::{
    format_hit_line, DegradedChannel, HitKind, HitSignal, SearchHit, SearchOptions, SearchResponse,
    SnapshotStamp, SpanHitInput,
};
const CASCADE_PREFILTER_FILE_LIMIT: usize = 100;
const MAX_HITS_PER_FILE: usize = 3;

/// On mutex poison, clear cached state before continuing so a panicked
/// computation cannot leave a half-written entry visible (sxjc).
fn lock_clear_on_poison<T>(
    mutex: &Mutex<T>,
    clear: impl FnOnce(&mut T),
) -> MutexGuard<'_, T> {
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
    semantic_cache: Arc<Mutex<Option<SemanticCache>>>,
    response_cache: Mutex<ResponseCache>,
}
/// Fail closed when callers request optional neural/rerank paths that were
/// not compiled in (parity contract: silently ignoring the flags would make
/// searches appear to use neural/rerank when they do not).
pub fn validate_search_feature_flags(options: &SearchOptions) -> Result<()> {
    if options.use_neural_embed {
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
        Ok(Self::with_store(
            IndexStore::open(&options.root, options.index_path.as_deref())?,
            options,
        ))
    }
    pub fn with_store(store: IndexStore, options: SearchOptions) -> Self {
        Self {
            store,
            options,
            semantic_cache: Arc::new(Mutex::new(None)),
            response_cache: Mutex::new(ResponseCache {
                gen: IndexGeneration {
                    external: 0,
                    local: 0,
                },
                map: std::collections::HashMap::new(),
                order: std::collections::VecDeque::new(),
                enabled: true,
            }),
        }
    }
    pub fn store(&self) -> &IndexStore {
        &self.store
    }
    pub fn options(&self) -> &SearchOptions {
        &self.options
    }
    fn index_gen(&self) -> Option<IndexGeneration> {
        // PRAGMA failure disables caching rather than pinning gen=0 (hdwh).
        let external = self
            .store
            .connection()
            .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
            .ok()?;
        let local = self.store.index_data_version().ok()?;
        Some(IndexGeneration { external, local })
    }
    fn cache_key(&self, kind: &str, query: &str) -> String {
        // Full SearchOptions identity (nyui).
        format!("{kind}\0{query}\0{}", self.options.cache_identity())
    }
    /// Run one multi-pass search inside a single read snapshot and stamp the
    /// resulting response with that snapshot's identity (d3l5).
    ///
    /// Every pass shares one connection, and in autocommit each statement is
    /// its own implicit transaction -- so a writer committing mid-search could
    /// otherwise let one response mix generations. `BEGIN DEFERRED` pins one
    /// read snapshot for the whole search; SQLite's WAL snapshot isolation then
    /// guarantees every pass observes the same committed state.
    ///
    /// The generation is re-read before COMMIT and compared. Equality is the
    /// evidence that the response is single-generation; a mismatch is reported
    /// rather than returned as if it were coherent.
    fn fenced(
        &self,
        compute: impl FnOnce() -> Result<SearchResponse>,
    ) -> Result<SearchResponse> {
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
        let generation_before = self.store.index_generation().unwrap_or_default();

        let computed = compute();

        let generation_after = self.store.index_generation().unwrap_or_default();
        if owns_snapshot {
            // A read snapshot is released either way; COMMIT is the cheap path.
            // If COMMIT fails, ROLLBACK unsticks the connection for later searches.
            if conn.execute_batch("COMMIT").is_err() {
                let _ = conn.execute_batch("ROLLBACK");
            }
        }
        let mut response = computed?;

        if owns_snapshot && generation_after != generation_before {
            return Err(crate::StoreError::Other(format!(
                "index generation changed during search ({generation_before} -> {generation_after}); \
                 retry for a single-generation response"
            )));
        }

        response.snapshot = self.snapshot_stamp(generation_before);
        Ok(response)
    }

    /// Fingerprint of the semantic sidecar, and whether it matches this
    /// generation (d3l5).
    ///
    /// `load_semantic_ivf` returns `Ok(None)` on a fingerprint mismatch, which
    /// makes a stale sidecar indistinguishable from no sidecar at all: search
    /// quietly falls back to brute force and the response looks healthy. Peek
    /// at the stored fingerprint so the mismatch is reported instead.
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

    /// Fingerprint the sidecar should carry for the current snapshot (d3l5).
    /// `None` when the inputs cannot be read, so an unknown state is never
    /// reported as a mismatch.
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

    /// Describe the snapshot a response was read from (d3l5).
    fn snapshot_stamp(&self, generation: i64) -> SnapshotStamp {
        let mut degraded_channels = Vec::new();
        let semantic_manifest = self.semantic_manifest(generation, &mut degraded_channels);
        SnapshotStamp {
            generation,
            schema_version: self.store.schema_version(),
            worktree_revision: self.store.worktree_revision().unwrap_or_default(),
            git_head: read_git_head(&self.options.root),
            semantic_manifest,
            degraded_channels,
        }
    }

    fn cached(
        &self,
        kind: &str,
        query: &str,
        compute: impl FnOnce() -> Result<SearchResponse>,
    ) -> Result<SearchResponse> {
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
        self.cached("lex", query_str, || {
            let parsed = ParsedQuery::parse(query_str);
            Ok(finish_response_checked(
                &parsed,
                &self.options,
                lexical_pass(&self.store, &self.options, &parsed)?,
                true,
            )?)
        })
    }
    pub fn search_symbol_pass(&self, query_str: &str) -> Result<SearchResponse> {
        self.cached("sym", query_str, || {
            let parsed = ParsedQuery::parse(query_str);
            let mut hits = symbol_pass(&self.store, &self.options, &parsed)?;
            hits.extend(anchor_pass(&self.store, &self.options, &parsed)?);
            Ok(finish_response_checked(&parsed, &self.options, hits, true)?)
        })
    }
    pub fn search(&self, query_str: &str) -> Result<SearchResponse> {
        let _perf_run = crate::perf_profile::Run::start("search_query");
        let _span = crate::perf_profile::Span::start(
            "search_query",
            "search",
            "Searcher::search (mode dispatch + finish)",
        );
        self.cached("search", query_str, || {
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
                )?,
                QueryMode::Literal | QueryMode::Word => {
                    literal_pass(&self.store, &self.options, &parsed)?
                }
                QueryMode::Regex => regex_pass(&self.store, &self.options, &parsed)?,
                QueryMode::Hybrid => {
                    // Quoted → Literal intent must run phrase literal_pass (50hx).
                    if crate::intent::classify(&parsed) == crate::intent::QueryIntent::Literal {
                        let phrase = strip_wrapping_quotes(&parsed.raw);
                        literal_pass(
                            &self.store,
                            &self.options,
                            &ParsedQuery::literal(phrase),
                        )?
                    } else {
                        let mut hits = self.search_hybrid(&parsed)?;
                        crate::intent::route_hits(&parsed, &mut hits);
                        let weights =
                            crate::intent::weights_for(crate::intent::classify(&parsed));
                        crate::fusion::apply_weighted_rrf(&mut hits, &weights);
                        hits
                    }
                }
            };
            Ok(finish_response_checked(&parsed, &self.options, hits, true)?)
        })
    }
    pub fn search_semantic(&self, query_str: &str) -> Result<SearchResponse> {
        self.cached("sem", query_str, || {
            let parsed = ParsedQuery::parse(query_str);
            Ok(finish_response_checked(
                &parsed,
                &self.options,
                run_embed_pass(&self.store, &self.options, &parsed, &self.semantic_cache)?,
                false,
            )?)
        })
    }
    pub fn search_literal(&self, query: &str) -> Result<SearchResponse> {
        self.cached("lit", query, || {
            let parsed = ParsedQuery::literal(query);
            Ok(finish_response_checked(
                &parsed,
                &self.options,
                literal_pass(&self.store, &self.options, &parsed)?,
                true,
            )?)
        })
    }
    pub fn search_regex(&self, query: &str) -> Result<SearchResponse> {
        self.cached("re", query, || {
            let parsed = ParsedQuery::regex(query);
            Ok(finish_response_checked(
                &parsed,
                &self.options,
                regex_pass(&self.store, &self.options, &parsed)?,
                true,
            )?)
        })
    }
    pub fn search_word(&self, query: &str) -> Result<SearchResponse> {
        self.cached("word", query, || {
            let parsed = ParsedQuery::word(query);
            Ok(finish_response_checked(
                &parsed,
                &self.options,
                literal_pass(&self.store, &self.options, &parsed)?,
                true,
            )?)
        })
    }
    fn search_hybrid(&self, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
        // Constraint cascade: each stage receives only files that survived the prior stage.
        let mut lexical = literal_prefilter_pass(&self.store, &self.options, parsed)?;
        let lexical_files = lexical
            .iter()
            .map(|hit| hit.file.clone())
            .collect::<HashSet<_>>();
        if lexical_files.is_empty() {
            return Ok(Vec::new());
        }

        let ast_matches =
            structural_index_pass(&self.store, &self.options, parsed, &lexical_files)?;
        let mut structural =
            symbol_pass_for_files(&self.store, &self.options, parsed, &lexical_files)?;
        structural.extend(anchor_pass_for_files(
            &self.store,
            &self.options,
            parsed,
            &lexical_files,
        )?);
        structural.extend(ast_matches);
        let structural_files = structural
            .iter()
            .map(|hit| hit.file.clone())
            .collect::<HashSet<_>>();
        // Precision gate: embed only on structurally-confirmed files when
        // structural signals exist. When the structural stage is empty, the
        // lexical survivors ARE the candidate set — the semantic stage must
        // still run on them (ht1h.3 / parity: NL queries surface semantically
        // related symbols, and plain-content files stay findable).
        let working_files = if structural_files.is_empty() {
            lexical_files
        } else {
            structural_files
        };

        lexical.retain(|hit| working_files.contains(&hit.file));
        let mut hits = lexical;
        hits.extend(structural);
        if self.options.use_embed {
            hits.extend(embed_pass_for_files(
                &self.store,
                &self.options,
                parsed,
                &working_files,
            )?);
        }
        Ok(hits)
    }
}
fn literal_prefilter_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let mut terms = parsed
        .terms
        .iter()
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>();
    terms.sort_by_key(|term| std::cmp::Reverse(term.chars().count()));
    let mut prefilter_options = options.clone();
    prefilter_options.case_insensitive = true;
    prefilter_options.limit = CASCADE_PREFILTER_FILE_LIMIT;
    let mut hits = Vec::new();
    let mut file_scores = std::collections::HashMap::<String, f64>::new();
    for term in terms {
        for hit in literal_pass(store, &prefilter_options, &ParsedQuery::literal(term))? {
            *file_scores.entry(hit.file.clone()).or_default() +=
                hit.score * term.chars().count() as f64;
            hits.push(hit);
        }
    }
    let mut ranked_files = file_scores.into_iter().collect::<Vec<_>>();
    ranked_files.sort_by(|(file_a, score_a), (file_b, score_b)| {
        score_b.total_cmp(score_a).then_with(|| file_a.cmp(file_b))
    });
    let allowed_files = ranked_files
        .into_iter()
        .take(CASCADE_PREFILTER_FILE_LIMIT)
        .map(|(file, _)| file)
        .collect::<HashSet<_>>();
    hits.retain(|hit| allowed_files.contains(&hit.file));
    Ok(hits)
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
    let mut hits = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for term in &parsed.terms {
        if term.len() < 3 || !term.chars().all(|c| c == '_' || c.is_alphanumeric()) {
            continue;
        }
        let signatures = ast_sgrep_lang::structural_term_signatures(term);
        for sig in &signatures {
            for row in store.pattern_nodes_matching(sig, lang)? {
                if !allowed_files.contains(&row.path)
                    || !seen.insert((row.path.clone(), row.line_start, row.line_end))
                {
                    continue;
                }
                hits.push(SearchHit::span(SpanHitInput {
                    kind: HitKind::Pattern,
                    file: row.path,
                    line_start: row.line_start,
                    line_end: row.line_end,
                    score: SCORE_PATTERN * 0.85,
                    excerpt: row.excerpt,
                    symbol: Some(term.clone()),
                    language: row.language,
                }));
            }
        }
    }
    Ok(hits)
}
fn identifier_tokens(symbol: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut previous_lower_or_digit = false;
    for ch in symbol.chars() {
        if !ch.is_alphanumeric() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            previous_lower_or_digit = false;
            continue;
        }
        if ch.is_uppercase() && previous_lower_or_digit && !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
        current.extend(ch.to_lowercase());
        previous_lower_or_digit = ch.is_lowercase() || ch.is_ascii_digit();
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn definition_query_affinity(parsed: &ParsedQuery, hit: &SearchHit) -> u8 {
    let Some(symbol) = hit.symbol.as_deref() else {
        return 0;
    };
    let symbol_tokens = identifier_tokens(symbol);
    if symbol_tokens.is_empty() || symbol_tokens.len() > parsed.terms.len() {
        return 0;
    }
    let matches_boundary = parsed.terms.windows(symbol_tokens.len()).any(|window| {
        window
            .iter()
            .map(String::as_str)
            .eq(symbol_tokens.iter().map(String::as_str))
    });
    if !matches_boundary {
        return 0;
    }
    let snake_spelling = symbol_tokens.join("_");
    if symbol.to_lowercase() == snake_spelling {
        3
    } else {
        2
    }
}

/// Shared ranking key for pre-truncate prune and final sort in `finish_response`.
/// Multi-term queries prefer coverage so high-coverage lower-score evidence is retained (8mb8).
fn cmp_ranked_hits(
    a: &SearchHit,
    coverage_a: u32,
    b: &SearchHit,
    coverage_b: u32,
    multi_term: bool,
) -> std::cmp::Ordering {
    let score_ord = b
        .score
        .partial_cmp(&a.score)
        .unwrap_or(std::cmp::Ordering::Equal);
    let coverage_ord = coverage_b.cmp(&coverage_a);
    let primary = if multi_term {
        coverage_ord.then(score_ord)
    } else {
        score_ord.then(coverage_ord)
    };
    primary
        .then_with(|| a.file.cmp(&b.file))
        .then_with(|| a.line_start.cmp(&b.line_start))
}

fn same_definition_locus(hit: &SearchHit, definition: &SearchHit) -> bool {
    hit.kind == HitKind::Def
        && hit.file == definition.file
        && hit.line_start == definition.line_start
        && hit.symbol == definition.symbol
}

/// Preserve the pre-1.3 non-fallible response API. Invalid globs keep the
/// legacy behavior and are ignored; internal search paths use the checked API.
pub fn finish_response(
    parsed: &ParsedQuery,
    options: &SearchOptions,
    hits: Vec<SearchHit>,
    dedup: bool,
) -> SearchResponse {
    let mut compatibility_options = options.clone();
    if compatibility_options
        .file_filter
        .as_ref()
        .is_some_and(|filter| compile_glob(filter).is_err())
    {
        compatibility_options.file_filter = None;
    }
    finish_response_checked(parsed, &compatibility_options, hits, dedup)
        .expect("compatibility options were validated")
}

pub(crate) fn finish_response_checked(
    parsed: &ParsedQuery,
    options: &SearchOptions,
    mut hits: Vec<SearchHit>,
    dedup: bool,
) -> Result<SearchResponse> {
    if dedup {
        hits = dedup_hits(hits);
    }
    if let Some(ref filter) = options.file_filter {
        // iva9.2: invalid globs error — never silently skip the filter.
        let re = compile_glob(filter).map_err(|e| {
            crate::StoreError::Other(format!("invalid file_filter glob '{filter}': {e}"))
        })?;
        hits.retain(|h| re.is_match(&h.file));
    }
    assign_signal_margins(&mut hits);
    if options.count_only {
        let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for hit in &hits {
            *counts.entry(hit.file.clone()).or_default() += 1;
        }
        let mut counts: Vec<_> = counts.into_iter().collect();
        counts.sort_by(|a, b| a.0.cmp(&b.0));
        let response = SearchResponse {
            query: parsed.raw.clone(),
            limit: options.limit,
            hits: vec![],
            counts,
            read_bytes_estimate: 0,
            returned_excerpt_bytes: 0,
            prevented_read_bytes: 0,
            // Stamped by the Searcher, which owns the snapshot (d3l5).
            snapshot: SnapshotStamp::default(),
        };
        record_ledger_from_env(&response);
        return Ok(response);
    }
    let gate_limit = rerank_candidate_limit(options);
    let hybrid = parsed.mode == QueryMode::Hybrid;
    let best_definition = if hybrid {
        hits.iter()
            .filter(|hit| hit.kind == HitKind::Def)
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        definition_query_affinity(parsed, a)
                            .cmp(&definition_query_affinity(parsed, b))
                    })
                    .then_with(|| b.file.cmp(&a.file))
            })
            .cloned()
    } else {
        None
    };
    let keep = if hybrid {
        gate_limit.saturating_mul(MAX_HITS_PER_FILE).max(gate_limit)
    } else {
        gate_limit
    };
    let prune_keep = keep.saturating_mul(4).max(keep.saturating_add(32));
    let multi_term = parsed.terms.len() > 1;
    if hits.len() > prune_keep {
        // Keep coverage in the pre-truncate sort key so high-coverage lower-score
        // hits survive the keep*4 prune (8mb8).
        hits.select_nth_unstable_by(prune_keep, |a, b| {
            cmp_ranked_hits(
                a,
                excerpt_term_coverage(&parsed.terms, a),
                b,
                excerpt_term_coverage(&parsed.terms, b),
                multi_term,
            )
        });
        hits.truncate(prune_keep);
    }
    let mut keyed: Vec<(u32, SearchHit)> = hits
        .into_iter()
        .map(|h| (excerpt_term_coverage(&parsed.terms, &h), h))
        .collect();
    let mut compare = |(ca, a): &(u32, SearchHit), (cb, b): &(u32, SearchHit)| {
        cmp_ranked_hits(a, *ca, b, *cb, multi_term)
    };
    if keyed.len() > keep {
        keyed.select_nth_unstable_by(keep, &mut compare);
        keyed.truncate(keep);
    }
    keyed.sort_unstable_by(compare);
    let mut hits: Vec<_> = keyed.into_iter().map(|(_, h)| h).collect();
    if let Some(definition) = best_definition {
        let retained = hits
            .iter()
            .any(|hit| same_definition_locus(hit, &definition));
        if !retained {
            hits.push(definition);
        }
    }
    hits = enforce_result_gates(hits, hybrid, gate_limit);
    if options.use_rerank {
        hits = maybe_rerank(&parsed.raw, hits, options.rerank_top_k);
        hits = enforce_result_gates(hits, parsed.mode == QueryMode::Hybrid, options.limit);
    }
    let (read_bytes_estimate, returned_excerpt_bytes, prevented_read_bytes) =
        estimate_prevented_reads(&options.root, &hits);
    let response = SearchResponse {
        query: parsed.raw.clone(),
        limit: options.limit,
        hits,
        counts: vec![],
        read_bytes_estimate,
        returned_excerpt_bytes,
        prevented_read_bytes,
        // Stamped by the Searcher, which owns the snapshot (d3l5).
        snapshot: SnapshotStamp::default(),
    };
    record_ledger_from_env(&response);
    Ok(response)
}
fn rerank_candidate_limit(options: &SearchOptions) -> usize {
    if options.use_rerank {
        options.limit.max(options.rerank_top_k)
    } else {
        options.limit
    }
}
fn maybe_rerank(query: &str, hits: Vec<SearchHit>, top_k: usize) -> Vec<SearchHit> {
    if hits.is_empty() {
        return hits;
    }
    let k = top_k.max(1).min(hits.len());
    let docs: Vec<String> = hits
        .iter()
        .take(k)
        .map(|h| {
            format!(
                "{}:{} {}",
                h.file,
                h.line_start,
                h.excerpt.lines().next().unwrap_or("")
            )
        })
        .collect();
    #[cfg(feature = "rerank")]
    {
        match ast_sgrep_embed::rerank(query, &docs) {
            Ok(scores) => {
                return apply_rerank_order(hits, k, scores.into_iter().map(|s| (s.index, s.score)))
            }
            Err(e) => eprintln!("[asgrep] rerank skipped: {e}"),
        }
    }
    #[cfg(not(feature = "rerank"))]
    {
        let _ = (query, &docs);
    }
    hits
}
#[cfg(any(feature = "rerank", test))]
fn apply_rerank_order(
    mut hits: Vec<SearchHit>,
    top_k: usize,
    scores: impl IntoIterator<Item = (usize, f32)>,
) -> Vec<SearchHit> {
    let k = top_k.min(hits.len());
    let mut prefix: Vec<Option<SearchHit>> = hits.drain(..k).map(Some).collect();
    let mut seen = vec![false; k];
    let mut ranked: Vec<(f32, usize)> = scores
        .into_iter()
        .filter(|(index, score)| {
            let valid =
                *index < k && score.is_finite() && !seen.get(*index).copied().unwrap_or(true);
            if valid {
                seen[*index] = true;
            }
            valid
        })
        .map(|(index, score)| (score, index))
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out = Vec::with_capacity(prefix.len() + hits.len());
    out.extend(
        ranked
            .into_iter()
            .filter_map(|(_, index)| prefix[index].take()),
    );
    out.extend(prefix.into_iter().flatten());
    out.append(&mut hits);
    out
}
fn enforce_result_gates(mut hits: Vec<SearchHit>, hybrid: bool, limit: usize) -> Vec<SearchHit> {
    if hybrid {
        let preferred_definition = hits.iter().find(|hit| hit.kind == HitKind::Def).cloned();
        hits = cap_per_file(hits);
        let head = limit.min(hits.len());
        if head > 0 && !hits[..head].iter().any(|hit| hit.kind == HitKind::Def) {
            if let Some(definition) = preferred_definition {
                if let Some(index) = hits
                    .iter()
                    .position(|hit| same_definition_locus(hit, &definition))
                {
                    hits.remove(index);
                }
                hits.insert(head - 1, definition);
            }
        }
    }
    hits.truncate(limit);
    hits
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
                    matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_))
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
        let _ = append_ledger_entry(path, response);
        return;
    };
    if !parent_canon.starts_with(&cwd) {
        eprintln!("[asgrep] ignoring ASGREP_LEDGER_PATH: outside process cwd");
        return;
    }
    let _ = append_ledger_entry(path, response);
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
fn cap_per_file(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut counts = std::collections::HashMap::new();
    let mut kept = Vec::with_capacity(hits.len());
    let mut overflow = Vec::new();
    for hit in hits {
        let c = counts.entry(hit.file.clone()).or_insert(0);
        if *c < MAX_HITS_PER_FILE {
            *c += 1;
            kept.push(hit);
        } else {
            overflow.push(hit);
        }
    }
    kept.extend(overflow);
    kept
}
fn compile_glob(pattern: &str) -> std::result::Result<regex::Regex, String> {
    if pattern.is_empty() {
        return Err("file_filter must be non-empty".into());
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
fn contains_term_token(text: &str, term: &str) -> bool {
    !term.is_empty()
        && text.match_indices(term).any(|(start, matched)| {
            let before = text[..start].chars().next_back();
            let after = text[start + matched.len()..].chars().next();
            before.is_none_or(|ch| !ch.is_alphanumeric() && ch != '_')
                && after.is_none_or(|ch| !ch.is_alphanumeric() && ch != '_')
        })
}
fn excerpt_term_coverage(terms: &[String], hit: &SearchHit) -> u32 {
    terms
        .iter()
        .filter(|term| {
            // Match term casing: mixed/upper terms stay case-sensitive (hhca).
            if term.chars().any(|c| c.is_uppercase()) {
                contains_term_token(&hit.excerpt, term)
            } else {
                contains_term_token(&hit.excerpt.to_lowercase(), &term.to_lowercase())
            }
        })
        .count() as u32
}
fn strip_wrapping_quotes(raw: &str) -> &str {
    let t = raw.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        &t[1..t.len() - 1]
    } else {
        t
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn hit(file: &str, line: u32, score: f64) -> SearchHit {
        SearchHit {
            kind: HitKind::Asgrep,
            file: file.to_owned(),
            line_start: line,
            line_end: line,
            symbol: None,
            caller: None,
            callee: None,
            language: None,
            score,
            signal: HitSignal::Exact,
            contributors: vec![HitKind::Asgrep],
            margin: 0.0,
            excerpt: String::new(),
        }
    }
    #[test]
    fn excerpt_coverage_respects_term_casing() {
        let mut h = hit("a.rs", 1, 1.0);
        h.excerpt = "AuthRefresh token".into();
        assert_eq!(excerpt_term_coverage(&["AuthRefresh".into()], &h), 1);
        // Lowercase terms are case-insensitive and match the lowered excerpt.
        assert_eq!(excerpt_term_coverage(&["authrefresh".into()], &h), 1);
        // Mixed/upper terms stay case-sensitive and miss wrong casing.
        assert_eq!(excerpt_term_coverage(&["AUTHREFRESH".into()], &h), 0);
        assert_eq!(excerpt_term_coverage(&["token".into()], &h), 1);
    }

    #[test]
    fn pretruncate_keeps_high_coverage_lower_score() {
        let parsed = ParsedQuery::parse("alpha beta gamma");
        let mut low = hit("low.rs", 1, 0.1);
        low.excerpt = "alpha beta gamma present".into();
        let mut highs: Vec<_> = (0..40)
            .map(|i| {
                let mut h = hit(&format!("high-{i}.rs"), 1, 1.0);
                h.excerpt = "alpha only".into();
                h
            })
            .collect();
        highs.push(low);
        let options = SearchOptions {
            limit: 5,
            ..SearchOptions::default()
        };
        let response = finish_response(&parsed, &options, highs, false);
        assert!(
            response.hits.iter().any(|h| h.file == "low.rs"),
            "high-coverage lower-score hit must survive pre-truncate"
        );
    }

    #[test]
    fn definition_affinity_prefers_phrase_boundary_spelling() {
        let parsed = ParsedQuery::parse("how does auth refresh work");
        let mut snake = hit("snake.rs", 1, 1.0);
        snake.kind = HitKind::Def;
        snake.symbol = Some("auth_refresh".into());
        let mut camel = hit("camel.rs", 1, 1.0);
        camel.kind = HitKind::Def;
        camel.symbol = Some("authRefresh".into());
        assert!(
            definition_query_affinity(&parsed, &snake) > definition_query_affinity(&parsed, &camel)
        );

        let unrelated = ParsedQuery::parse("authorization workflow");
        let mut short = hit("short.rs", 1, 1.0);
        short.kind = HitKind::Def;
        short.symbol = Some("auth".into());
        assert_eq!(definition_query_affinity(&unrelated, &short), 0);

        let suffix = ParsedQuery::parse("refreshable token");
        short.symbol = Some("refresh".into());
        assert_eq!(definition_query_affinity(&suffix, &short), 0);
    }

    #[test]
    fn hybrid_window_retains_definition_evidence() {
        let mut hits = vec![
            hit("embed-a.rs", 1, 1.0),
            hit("embed-b.rs", 1, 0.9),
            hit("def.rs", 1, 0.2),
        ];
        hits[0].kind = HitKind::Embed;
        hits[1].kind = HitKind::Embed;
        hits[2].kind = HitKind::Def;
        let gated = enforce_result_gates(hits, true, 2);
        assert_eq!(gated.len(), 2);
        assert_eq!(gated[0].kind, HitKind::Embed);
        assert_eq!(gated[1].kind, HitKind::Def);
    }

    #[test]
    fn rerank_can_promote_candidate_beyond_final_limit() {
        let options = SearchOptions {
            limit: 16,
            use_rerank: true,
            rerank_top_k: 20,
            ..SearchOptions::default()
        };
        let hits: Vec<_> = (0..20)
            .map(|i| {
                hit(
                    &format!("candidate-{i}.rs"),
                    i + 1,
                    1.0 - f64::from(i) / 100.0,
                )
            })
            .collect();
        let candidates = enforce_result_gates(hits, false, rerank_candidate_limit(&options));
        assert_eq!(candidates.len(), 20);
        let reranked = apply_rerank_order(candidates, options.rerank_top_k, [(16, 1.0)]);
        let final_hits = enforce_result_gates(reranked, false, options.limit);
        assert_eq!(final_hits.len(), options.limit);
        assert_eq!(final_hits[0].file, "candidate-16.rs");
    }
    #[test]
    fn rerank_reorders_prefix_without_overwriting_fused_scores() {
        let hits = vec![
            hit("a.rs", 1, 0.9),
            hit("b.rs", 2, 0.8),
            hit("c.rs", 3, 0.7),
            hit("tail.rs", 4, 0.6),
        ];
        let reranked = apply_rerank_order(
            hits,
            3,
            [(2, 0.99), (0, 0.5), (7, 1.0), (2, 0.2), (1, f32::NAN)],
        );
        let identity: Vec<_> = reranked
            .iter()
            .map(|h| (h.file.as_str(), h.score))
            .collect();
        assert_eq!(
            identity,
            vec![
                ("c.rs", 0.7),
                ("a.rs", 0.9),
                ("b.rs", 0.8),
                ("tail.rs", 0.6)
            ]
        );
    }
    #[test]
    fn literal_prefilter_handles_trigram_casefold_short_terms_and_bounds() {
        use crate::store::UpsertFileInput;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let store = IndexStore::open(temp.path(), None).unwrap();
        let mut lines = (1..=1_000)
            .map(|line| (line, format!("filler line {line}")))
            .collect::<Vec<_>>();
        lines.push((1_001, "NeedleCase id".to_string()));
        store
            .upsert_file(UpsertFileInput {
                rel_path: "large.rs",
                language: Some("rust"),
                mtime_secs: 1,
                mtime_nanos: 0,
                content_hash: "large",
                lines: &lines,
                eol: "\n",
                symbols: &[],
                callers: &[],
                imports: &[],
                pattern_nodes: &[],
                semantic_chunks: &[],
                embed_semantic: false,
                embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
            })
            .unwrap();
        let options = SearchOptions {
            root: temp.path().to_path_buf(),
            ..SearchOptions::default()
        };
        let hits =
            literal_prefilter_pass(&store, &options, &ParsedQuery::parse("needlecase id")).unwrap();
        assert!(hits.iter().any(|hit| hit.excerpt == "NeedleCase id"));

        for index in 0..120 {
            let path = format!("bound-{index:03}.rs");
            let term = if index < 60 {
                "alphauniqueterm"
            } else {
                "betauniqueterm"
            };
            let bound_lines = [(1, term.to_string())];
            store
                .upsert_file(UpsertFileInput {
                    rel_path: &path,
                    language: Some("rust"),
                    mtime_secs: 1,
                    mtime_nanos: 0,
                    content_hash: &path,
                    lines: &bound_lines,
                    eol: "\n",
                    symbols: &[],
                    callers: &[],
                    imports: &[],
                    pattern_nodes: &[],
                    semantic_chunks: &[],
                    embed_semantic: false,
                    embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
                })
                .unwrap();
        }
        let bounded = literal_prefilter_pass(
            &store,
            &options,
            &ParsedQuery::parse("alphauniqueterm betauniqueterm"),
        )
        .unwrap();
        let files = bounded
            .iter()
            .map(|hit| hit.file.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(files.len(), CASCADE_PREFILTER_FILE_LIMIT);
    }

    #[test]
    fn hybrid_cap_and_limit_are_reapplied_after_rerank() {
        let hits = vec![
            hit("a.rs", 1, 0.9),
            hit("a.rs", 2, 0.8),
            hit("a.rs", 3, 0.7),
            hit("a.rs", 4, 0.6),
            hit("b.rs", 1, 0.5),
        ];
        let reranked =
            apply_rerank_order(hits, 5, [(3, 1.0), (2, 0.9), (1, 0.8), (0, 0.7), (4, 0.1)]);
        let gated = enforce_result_gates(reranked, true, 4);
        let identity: Vec<_> = gated
            .iter()
            .map(|h| (h.file.as_str(), h.line_start, h.score))
            .collect();
        assert_eq!(
            identity,
            vec![
                ("a.rs", 4, 0.6),
                ("a.rs", 3, 0.7),
                ("a.rs", 2, 0.8),
                ("b.rs", 1, 0.5)
            ]
        );
    }

    #[test]
    fn lock_clear_on_poison_resets_state() {
        let mutex = Mutex::new(vec![1, 2, 3]);
        let _ = std::panic::catch_unwind(|| {
            let _guard = mutex.lock().unwrap();
            panic!("inject poison");
        });
        assert!(mutex.is_poisoned());
        let guard = lock_clear_on_poison(&mutex, |v| v.clear());
        assert!(guard.is_empty());
        assert!(!mutex.is_poisoned());
    }
}

/// Resolve `.git/HEAD` to a commit id without spawning git (d3l5).
///
/// Returns `None` outside a git worktree, or when HEAD cannot be resolved --
/// an unknown source revision is reported as unknown, never guessed.
fn read_git_head(root: &std::path::Path) -> Option<String> {
    let git_dir = root.join(".git");
    // A worktree or submodule uses a `gitdir:` pointer file instead of a dir.
    let git_dir = if git_dir.is_file() {
        let pointer = std::fs::read_to_string(&git_dir).ok()?;
        let target = pointer.strip_prefix("gitdir:")?.trim();
        root.join(target)
    } else {
        git_dir
    };
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref:") {
        Some(reference) => {
            let reference = reference.trim();
            if let Ok(direct) = std::fs::read_to_string(git_dir.join(reference)) {
                return Some(direct.trim().to_owned());
            }
            // Packed refs: the loose file may not exist.
            let packed = std::fs::read_to_string(git_dir.join("packed-refs")).ok()?;
            packed.lines().find_map(|line| {
                let (id, name) = line.split_once(' ')?;
                (name.trim() == reference).then(|| id.trim().to_owned())
            })
        }
        // Detached HEAD already holds the id.
        None => (!head.is_empty()).then(|| head.to_owned()),
    }
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
