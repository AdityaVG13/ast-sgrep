pub mod passes;
mod types;
use crate::query::{ParsedQuery, QueryMode};
use crate::semantic_ann::flatten_vectors_for_search;
use crate::store::IndexStore;
use crate::Result;
use ast_sgrep_embed::SemanticChunkRow;
use passes::embed::{
    embed_pass_for_files, embed_pass_lazy_ivf, embed_pass_with_context, EmbedContext,
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
    format_hit_line, HitKind, HitSignal, SearchHit, SearchOptions, SearchResponse, SpanHitInput,
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
struct SemanticCache {
    lang_filter: Option<String>,
    max_id: i64,
    index_data_version: i64,
    semantic_data_version: i64,
    embed_backend: String,
    chunks: Arc<Vec<SemanticChunkRow>>,
    flat_vectors: Arc<Vec<f32>>,
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
    map: std::collections::HashMap<String, SearchResponse>,
}
pub struct Searcher {
    store: IndexStore,
    options: SearchOptions,
    semantic_cache: Arc<Mutex<Option<SemanticCache>>>,
    response_cache: Mutex<ResponseCache>,
}
impl Searcher {
    pub fn new(mut options: SearchOptions) -> Result<Self> {
        // Match Indexer: canonicalize roots so relative/symlink inputs share identity (0f7r).
        options.root = options.root.canonicalize().unwrap_or(options.root.clone());
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
            }),
        }
    }
    pub fn store(&self) -> &IndexStore {
        &self.store
    }
    pub fn options(&self) -> &SearchOptions {
        &self.options
    }
    fn index_gen(&self) -> IndexGeneration {
        IndexGeneration {
            external: self
                .store
                .connection()
                .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
                .unwrap_or(0),
            local: self.store.index_data_version().unwrap_or(0),
        }
    }
    fn cached(
        &self,
        kind: &str,
        query: &str,
        compute: impl FnOnce() -> Result<SearchResponse>,
    ) -> Result<SearchResponse> {
        let gen = self.index_gen();
        let key = format!("{kind}\0{query}");
        {
            let guard = lock_clear_on_poison(&self.response_cache, |cache| {
                cache.map.clear();
                cache.gen = IndexGeneration {
                    external: -1,
                    local: -1,
                };
            });
            if guard.gen == gen {
                if let Some(hit) = guard.map.get(&key) {
                    return Ok(hit.clone());
                }
            }
        }
        let response = compute()?;
        let mut guard = lock_clear_on_poison(&self.response_cache, |cache| {
            cache.map.clear();
            cache.gen = IndexGeneration {
                external: -1,
                local: -1,
            };
        });
        if guard.gen != gen {
            guard.map.clear();
            guard.gen = gen;
        }
        if guard.map.len() < 128 {
            guard.map.insert(key, response.clone());
        }
        Ok(response)
    }
    pub fn search_lexical(&self, query_str: &str) -> Result<SearchResponse> {
        self.cached("lex", query_str, || {
            let parsed = ParsedQuery::parse(query_str);
            Ok(finish_response(
                &parsed,
                &self.options,
                lexical_pass(&self.store, &self.options, &parsed)?,
                true,
            ))
        })
    }
    pub fn search_symbol_pass(&self, query_str: &str) -> Result<SearchResponse> {
        self.cached("sym", query_str, || {
            let parsed = ParsedQuery::parse(query_str);
            let mut hits = symbol_pass(&self.store, &self.options, &parsed)?;
            hits.extend(anchor_pass(&self.store, &self.options, &parsed)?);
            Ok(finish_response(&parsed, &self.options, hits, true))
        })
    }
    pub fn search(&self, query_str: &str) -> Result<SearchResponse> {
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
                    let mut hits = self.search_hybrid(&parsed)?;
                    crate::intent::route_hits(&parsed, &mut hits);
                    let weights = crate::intent::weights_for(crate::intent::classify(&parsed));
                    crate::fusion::apply_weighted_rrf(&mut hits, &weights);
                    hits
                }
            };
            Ok(finish_response(&parsed, &self.options, hits, true))
        })
    }
    pub fn search_semantic(&self, query_str: &str) -> Result<SearchResponse> {
        self.cached("sem", query_str, || {
            let parsed = ParsedQuery::parse(query_str);
            Ok(finish_response(
                &parsed,
                &self.options,
                run_embed_pass(&self.store, &self.options, &parsed, &self.semantic_cache)?,
                false,
            ))
        })
    }
    pub fn search_literal(&self, query: &str) -> Result<SearchResponse> {
        self.cached("lit", query, || {
            let parsed = ParsedQuery::literal(query);
            Ok(finish_response(
                &parsed,
                &self.options,
                literal_pass(&self.store, &self.options, &parsed)?,
                true,
            ))
        })
    }
    pub fn search_regex(&self, query: &str) -> Result<SearchResponse> {
        self.cached("re", query, || {
            let parsed = ParsedQuery::regex(query);
            Ok(finish_response(
                &parsed,
                &self.options,
                regex_pass(&self.store, &self.options, &parsed)?,
                true,
            ))
        })
    }
    pub fn search_word(&self, query: &str) -> Result<SearchResponse> {
        self.cached("word", query, || {
            let parsed = ParsedQuery::word(query);
            Ok(finish_response(
                &parsed,
                &self.options,
                literal_pass(&self.store, &self.options, &parsed)?,
                true,
            ))
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
        if structural_files.is_empty() {
            return Ok(Vec::new());
        }

        lexical.retain(|hit| structural_files.contains(&hit.file));
        let mut hits = lexical;
        hits.extend(structural);
        if self.options.use_embed {
            hits.extend(embed_pass_for_files(
                &self.store,
                &self.options,
                parsed,
                &structural_files,
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

fn run_embed_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    cache: &Mutex<Option<SemanticCache>>,
) -> Result<Vec<SearchHit>> {
    if let Some(hits) = embed_pass_lazy_ivf(store, options, parsed)? {
        return Ok(hits);
    }
    match load_semantic_context(store, options, cache)? {
        Some(ctx) => embed_pass_with_context(store, options, parsed, Some(ctx)),
        None => Ok(vec![]),
    }
}
fn load_semantic_context(
    store: &IndexStore,
    options: &SearchOptions,
    cache: &Mutex<Option<SemanticCache>>,
) -> Result<Option<EmbedContext>> {
    if !options.use_embed {
        return Ok(None);
    }
    let lang_filter = options.lang_filter.clone();
    let max_id = store.semantic_chunk_max_id()?.unwrap_or(0);
    let index_data_version = store.index_data_version()?;
    let semantic_data_version = store.semantic_data_version()?;
    let embed_backend = store
        .get_meta("embed_backend")?
        .unwrap_or_else(|| "semantic".into());
    {
        let guard = lock_clear_on_poison(cache, |slot| {
            *slot = None;
        });
        if let Some(c) = guard.as_ref() {
            if c.lang_filter == lang_filter
                && c.max_id == max_id
                && c.index_data_version == index_data_version
                && c.semantic_data_version == semantic_data_version
                && c.embed_backend == embed_backend
            {
                return Ok(Some(EmbedContext {
                    chunks: Arc::clone(&c.chunks),
                    flat_vectors: Arc::clone(&c.flat_vectors),
                }));
            }
        }
    }
    let chunks = store.all_semantic_chunks(lang_filter.as_deref())?;
    if chunks.is_empty() {
        return Ok(None);
    }
    let flat_vectors = flatten_vectors_for_search(&chunks, chunks[0].5.len())?;
    let entry = SemanticCache {
        lang_filter,
        max_id,
        index_data_version,
        semantic_data_version,
        embed_backend,
        chunks: Arc::new(chunks),
        flat_vectors: Arc::new(flat_vectors),
    };
    let ctx = EmbedContext {
        chunks: Arc::clone(&entry.chunks),
        flat_vectors: Arc::clone(&entry.flat_vectors),
    };
    *lock_clear_on_poison(cache, |slot| {
        *slot = None;
    }) = Some(entry);
    Ok(Some(ctx))
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
        let signatures = [
            format!("call-name:{term}"),
            format!("call:{term}"),
            format!("decl:fn:{term}"),
            format!("decl:def:{term}"),
            format!("decl:function:{term}"),
            term.clone(),
        ];
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

pub fn finish_response(
    parsed: &ParsedQuery,
    options: &SearchOptions,
    mut hits: Vec<SearchHit>,
    dedup: bool,
) -> SearchResponse {
    if dedup {
        hits = dedup_hits(hits);
    }
    if let Some(ref filter) = options.file_filter {
        if let Ok(re) = compile_glob(filter) {
            hits.retain(|h| re.is_match(&h.file));
        }
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
        };
        record_ledger_from_env(&response);
        return response;
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
    if hits.len() > keep.saturating_mul(4).max(keep.saturating_add(32)) {
        hits.select_nth_unstable_by(keep, |a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.file.cmp(&b.file))
                .then_with(|| a.line_start.cmp(&b.line_start))
        });
        hits.truncate(keep);
    }
    let mut keyed: Vec<(u32, SearchHit)> = hits
        .into_iter()
        .map(|h| (excerpt_term_coverage(&parsed.terms, &h), h))
        .collect();
    let mut compare = |(ca, a): &(u32, SearchHit), (cb, b): &(u32, SearchHit)| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| cb.cmp(ca))
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line_start.cmp(&b.line_start))
    };
    if keyed.len() > keep {
        keyed.select_nth_unstable_by(keep, &mut compare);
        keyed.truncate(keep);
    }
    keyed.sort_unstable_by(compare);
    let mut hits: Vec<_> = keyed.into_iter().map(|(_, h)| h).collect();
    if let Some(definition) = best_definition {
        let retained = hits.iter().any(|hit| {
            hit.kind == HitKind::Def
                && hit.file == definition.file
                && hit.line_start == definition.line_start
                && hit.symbol == definition.symbol
        });
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
    };
    record_ledger_from_env(&response);
    response
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
                if let Some(index) = hits.iter().position(|hit| {
                    hit.kind == HitKind::Def
                        && hit.file == definition.file
                        && hit.line_start == definition.line_start
                        && hit.symbol == definition.symbol
                }) {
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
fn compile_glob(pattern: &str) -> std::result::Result<regex::Regex, regex::Error> {
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
    regex::Regex::new(&result)
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
    let text = hit.excerpt.to_lowercase();
    terms
        .iter()
        .filter(|term| contains_term_token(&text, term))
        .count() as u32
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
