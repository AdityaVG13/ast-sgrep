mod hits; pub mod passes; mod types; pub use types::{format_hit_line, HitKind, SearchHit, SearchOptions, SearchResponse, SpanHitInput}; use std::collections::HashSet; use std::fs::OpenOptions; use std::io::Write; use std::path::Path;
use std::sync::{Arc, Mutex}; use std::thread; use std::time::{SystemTime, UNIX_EPOCH}; use ast_sgrep_embed::SemanticChunkRow; use crate::query::{ParsedQuery, QueryMode}; use crate::semantic_ann::flatten_vectors_for_search;
use crate::store::IndexStore; use crate::Result; use hits::dedup_hits; use passes::embed::{embed_pass_lazy_ivf, embed_pass_with_context, EmbedContext}; use passes::lexical::lexical_pass; use passes::literal::literal_pass;
use passes::regex::regex_pass; use passes::symbol::{anchor_pass, search_callers, search_defs, search_imports, symbol_pass}; const PARALLEL_PASS_FILE_THRESHOLD: usize = 128; const MAX_HITS_PER_FILE: usize = 3; struct SemanticCache {
    lang_filter: Option<String>, max_id: i64, embed_backend: String, chunks: Arc<Vec<SemanticChunkRow>>, flat_vectors: Arc<Vec<f32>>,
}
/// Searches use SQLite statement snapshots, so each channel is internally consistent,
/// but a hybrid search concurrent with reindex may combine adjacent committed snapshots.
/// Semantic cache entries are discarded when the chunk maximum id changes; persisted IVF
/// is fingerprint-validated and falls back to flat search when stale. Public methods return
/// structured errors rather than panicking if a concurrent reindex changes vector shape.
struct ResponseCache {
    gen: u64,
    // key = "kind\0query"
    map: std::collections::HashMap<String, SearchResponse>,
}
pub struct Searcher {
    store: IndexStore,
    options: SearchOptions,
    semantic_cache: Arc<Mutex<Option<SemanticCache>>>,
    response_cache: Mutex<ResponseCache>,
}
impl Searcher {
    pub fn new(options: SearchOptions) -> Result<Self> {
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
                gen: 0,
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

    fn index_gen(&self) -> u64 {
        // SQLite data_version changes on any committed write — one cheap pragma.
        self.store
            .connection()
            .query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
            .map(|v| v as u64)
            .unwrap_or(0)
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
            let guard = self.response_cache.lock().unwrap_or_else(|e| e.into_inner());
            if guard.gen == gen {
                if let Some(hit) = guard.map.get(&key) {
                    return Ok(hit.clone());
                }
            }
        }
        let response = compute()?;
        let mut guard = self.response_cache.lock().unwrap_or_else(|e| e.into_inner());
        if guard.gen != gen {
            guard.map.clear();
            guard.gen = gen;
        }
        if guard.map.len() < 128 {
            guard.map.insert(key, response.clone());
        }
        Ok(response)
    }

    /// Lexical / FTS pass only.
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

    /// Symbol + anchor graph passes only.
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
        let parallel = self
            .store .status() .map(|s| s.file_count >= PARALLEL_PASS_FILE_THRESHOLD) .unwrap_or(false);
        if parallel {
            self.search_hybrid_large(parsed)
        } else {
            let mut hits = run_serial_passes(&self.store, &self.options, parsed)?; if self.options.use_embed {
                if let Some(ctx) = load_semantic_context(&self.store, &self.options, &self.semantic_cache)? {
                    hits.extend(embed_pass_with_context(&self.store, &self.options, parsed, Some(ctx))?);
                }
            } Ok(hits)
        }
    }

    fn search_hybrid_large(&self, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
        let root = self.options.root.clone(); let index_path = self.options.index_path.clone(); let options = self.options.clone(); let parsed_c = parsed.clone(); let parsed_sql = parsed.clone();
        thread::scope(|scope| -> Result<Vec<SearchHit>> {
            let embed = self.options.use_embed.then(|| {
                let cache = Arc::clone(&self.semantic_cache); let (root_e, index_path_e, options_e, parsed_e) =
                    (root.clone(), index_path.clone(), options.clone(), parsed_c.clone());
                scope.spawn(move || {
                    run_embed_pass(&IndexStore::open(&root_e, index_path_e.as_deref())?, &options_e, &parsed_e, &cache)
                })
            }); let sql = scope.spawn(move || run_parallel_passes(&root, index_path.as_deref(), &options, &parsed_sql)); let mut hits = join_worker(sql.join())?;
            if let Some(embed) = embed { hits.extend(join_worker(embed.join())?); } Ok(hits)
        })
    }
} fn run_embed_pass(
    store: &IndexStore, options: &SearchOptions, parsed: &ParsedQuery, cache: &Mutex<Option<SemanticCache>>,
) -> Result<Vec<SearchHit>> {
    if let Some(hits) = embed_pass_lazy_ivf(store, options, parsed)? { return Ok(hits); } match load_semantic_context(store, options, cache)? {
        Some(ctx) => embed_pass_with_context(store, options, parsed, Some(ctx)), None => Ok(vec![]),
    }
} fn load_semantic_context(
    store: &IndexStore, options: &SearchOptions, cache: &Mutex<Option<SemanticCache>>,
) -> Result<Option<EmbedContext>> {
    if !options.use_embed { return Ok(None); } let lang_filter = options.lang_filter.clone(); let max_id = store.semantic_chunk_max_id()?.unwrap_or(0);
    let embed_backend = store.get_meta("embed_backend")?.unwrap_or_else(|| "semantic".into()); {
        let guard = cache.lock().unwrap_or_else(|e| e.into_inner()); if let Some(c) = guard.as_ref() {
            if c.lang_filter == lang_filter && c.max_id == max_id && c.embed_backend == embed_backend { return Ok(Some(EmbedContext {
                    chunks: Arc::clone(&c.chunks), flat_vectors: Arc::clone(&c.flat_vectors),
                })); }
        }
    } let chunks = store.all_semantic_chunks(lang_filter.as_deref())?; if chunks.is_empty() { return Ok(None); } let flat_vectors = flatten_vectors_for_search(&chunks, chunks[0].5.len())?; let entry = SemanticCache {
        lang_filter, max_id, embed_backend, chunks: Arc::new(chunks), flat_vectors: Arc::new(flat_vectors),
    }; let ctx = EmbedContext { chunks: Arc::clone(&entry.chunks), flat_vectors: Arc::clone(&entry.flat_vectors) }; *cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(entry); Ok(Some(ctx))
} fn run_serial_passes(store: &IndexStore, options: &SearchOptions, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
    let mut hits = Vec::with_capacity(64); hits.extend(lexical_pass(store, options, parsed)?); hits.extend(symbol_pass(store, options, parsed)?); hits.extend(anchor_pass(store, options, parsed)?); Ok(hits)
} fn run_parallel_passes(
    root: &Path, index_path: Option<&Path>, options: &SearchOptions, parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let (parsed, options, root, index_path) = (
        parsed.clone(), options.clone(), root.to_path_buf(), index_path.map(|p| p.to_path_buf()),
    ); thread::scope(|scope| {
        let lex = scope.spawn(|| {
            let store = IndexStore::open(&root, index_path.as_deref())?; lexical_pass(&store, &options, &parsed)
        }); let sym = scope.spawn(|| {
            let store = IndexStore::open(&root, index_path.as_deref())?; symbol_pass(&store, &options, &parsed)
        }); let anchor = scope.spawn(|| {
            let store = IndexStore::open(&root, index_path.as_deref())?; anchor_pass(&store, &options, &parsed)
        }); let mut hits = join_worker(lex.join())?; hits.extend(join_worker(sym.join())?); hits.extend(join_worker(anchor.join())?); Ok(hits)
    })
} fn join_worker<T>(join: thread::Result<Result<T>>) -> Result<T> {
    join.map_err(|e| crate::StoreError::Other(format!("search worker panicked: {e:?}")))?
}
/// Rank, dedup, and cap multi-pass candidates into a [`SearchResponse`].
pub fn finish_response(
    parsed: &ParsedQuery, options: &SearchOptions, mut hits: Vec<SearchHit>, dedup: bool,
) -> SearchResponse {
    if dedup { hits = dedup_hits(hits); } if let Some(ref filter) = options.file_filter {
        if let Ok(re) = compile_glob(filter) { hits.retain(|h| re.is_match(&h.file)); }
    } if options.count_only {
        let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for hit in &hits { *counts.entry(hit.file.clone()).or_default() += 1; } let mut counts: Vec<_> = counts.into_iter().collect(); counts.sort_by(|a, b| a.0.cmp(&b.0)); let response = SearchResponse {
            query: parsed.raw.clone(), limit: options.limit, hits: vec![], counts, read_bytes_estimate: 0, returned_excerpt_bytes: 0, prevented_read_bytes: 0,
        }; record_ledger_from_env(&response); return response;
    } let gate_limit = rerank_candidate_limit(options);
    // Pre-trim before coverage/sort when far over the gate limit (hybrid included).
    let keep = if parsed.mode == QueryMode::Hybrid {
        gate_limit.saturating_mul(MAX_HITS_PER_FILE).max(gate_limit)
    } else {
        gate_limit
    }; if hits.len() > keep.saturating_mul(4).max(keep + 32) {
        hits.select_nth_unstable_by(keep, |a, b| {
            b.score
                .partial_cmp(&a.score) .unwrap_or(std::cmp::Ordering::Equal) .then_with(|| a.file.cmp(&b.file)) .then_with(|| a.line_start.cmp(&b.line_start))
        }); hits.truncate(keep);
    } let mut keyed: Vec<(u32, SearchHit)> = hits
        .into_iter() .map(|h| (excerpt_term_coverage(&parsed.terms, &h), h)) .collect();
    let mut compare = |(ca, a): &(u32, SearchHit), (cb, b): &(u32, SearchHit)| {
        b.score
            .partial_cmp(&a.score) .unwrap_or(std::cmp::Ordering::Equal) .then_with(|| cb.cmp(ca)) .then_with(|| a.file.cmp(&b.file)) .then_with(|| a.line_start.cmp(&b.line_start))
    }; if keyed.len() > keep { keyed.select_nth_unstable_by(keep, &mut compare); keyed.truncate(keep); } keyed.sort_unstable_by(compare);
    let mut hits: Vec<_> = keyed.into_iter().map(|(_, h)| h).collect(); hits = enforce_result_gates(hits, parsed.mode == QueryMode::Hybrid, gate_limit); if options.use_rerank {
        hits = maybe_rerank(&parsed.raw, hits, options.rerank_top_k); hits = enforce_result_gates(hits, parsed.mode == QueryMode::Hybrid, options.limit);
    } let (read_bytes_estimate, returned_excerpt_bytes, prevented_read_bytes) =
        estimate_prevented_reads(&options.root, &hits);
    let response = SearchResponse {
        query: parsed.raw.clone(), limit: options.limit, hits, counts: vec![], read_bytes_estimate, returned_excerpt_bytes, prevented_read_bytes,
    }; record_ledger_from_env(&response); response
} fn rerank_candidate_limit(options: &SearchOptions) -> usize {
    if options.use_rerank {
        options.limit.max(options.rerank_top_k)
    } else {
        options.limit
    }
}

fn maybe_rerank(query: &str, hits: Vec<SearchHit>, top_k: usize) -> Vec<SearchHit> {
    if hits.is_empty() { return hits; } let k = top_k.max(1).min(hits.len()); let docs: Vec<String> = hits
        .iter() .take(k) .map(|h| {
            format!(
                "{}:{} {}", h.file, h.line_start, h.excerpt.lines().next().unwrap_or("")
            )
        }) .collect();
    #[cfg(feature = "rerank")] {
        match ast_sgrep_embed::rerank(query, &docs) {
            Ok(scores) => {
                return apply_rerank_order(
                    hits, k, scores.into_iter().map(|score| (score.index, score.score)),
                );
            } Err(e) => eprintln!("[asgrep] rerank skipped: {e}"),
        }
    } #[cfg(not(feature = "rerank"))] {
        let _ = (query, &docs);
    } hits
} #[cfg(any(feature = "rerank", test))] fn apply_rerank_order(
    mut hits: Vec<SearchHit>, top_k: usize, scores: impl IntoIterator<Item = (usize, f32)>,
) -> Vec<SearchHit> {
    let k = top_k.min(hits.len()); let mut prefix: Vec<Option<SearchHit>> = hits.drain(..k).map(Some).collect(); let mut seen = vec![false; k]; let mut ranked: Vec<(f32, usize)> = scores
        .into_iter() .filter(|(index, score)| {
            let valid = *index < k && score.is_finite() && !seen.get(*index).copied().unwrap_or(true); if valid { seen[*index] = true; } valid
        }) .map(|(index, score)| (score, index)) .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));

    let mut out = Vec::with_capacity(prefix.len() + hits.len()); out.extend(ranked.into_iter().filter_map(|(_, index)| prefix[index].take())); out.extend(prefix.into_iter().flatten()); out.append(&mut hits); out
} fn enforce_result_gates(mut hits: Vec<SearchHit>, hybrid: bool, limit: usize) -> Vec<SearchHit> {
    if hybrid { hits = cap_per_file(hits); } hits.truncate(limit); hits
} fn estimate_prevented_reads(root: &Path, hits: &[SearchHit]) -> (u64, u64, u64) {
    // Process-wide cache: warm search paths re-hit the same sample files every call.
    // Values match fs::metadata; only the syscall is elided on cache hits.
    use std::sync::OnceLock; static META_CACHE: OnceLock<Mutex<std::collections::HashMap<String, u64>>> = OnceLock::new(); let cache = META_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut files = HashSet::new(); let mut read_bytes_estimate = 0u64; {
        let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner()); for h in hits {
            if !files.insert(h.file.as_str()) { continue; } let key = if Path::new(&h.file).is_absolute() {
                h.file.clone()
            } else {
                root.join(&h.file).to_string_lossy().into_owned()
            }; let len = if let Some(&n) = guard.get(&key) {
                n
            } else {
                let n = std::fs::metadata(&key).ok().map(|m| m.len()).unwrap_or(0); guard.insert(key, n); n
            }; read_bytes_estimate += len;
        }
    } let returned_excerpt_bytes = hits.iter().map(|h| h.excerpt.len() as u64).sum(); (
        read_bytes_estimate, returned_excerpt_bytes, read_bytes_estimate.saturating_sub(returned_excerpt_bytes),
    )
} fn record_ledger_from_env(response: &SearchResponse) {
    let Some(path) = std::env::var_os("ASGREP_LEDGER_PATH") else {
        return;
    }; let _ = append_ledger_entry(Path::new(&path), response);
} fn append_ledger_entry(path: &Path, response: &SearchResponse) -> std::io::Result<()> {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(); let mut line = serde_json::to_vec(&serde_json::json!({
        "ts": ts, "query": response.query, "hits": response.hits.len(), "bytes": {
            "read_bytes_estimate": response.read_bytes_estimate, "returned_excerpt_bytes": response.returned_excerpt_bytes, "prevented_read_bytes": response.prevented_read_bytes,
        },
    })) .map_err(std::io::Error::other)?; line.push(b'\n'); OpenOptions::new().create(true).append(true).open(path)?.write_all(&line)
} fn cap_per_file(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut counts = std::collections::HashMap::new(); let mut kept = Vec::with_capacity(hits.len()); let mut overflow = Vec::new(); for hit in hits {
        let c = counts.entry(hit.file.clone()).or_insert(0); if *c < MAX_HITS_PER_FILE { *c += 1; kept.push(hit); } else {
            overflow.push(hit);
        }
    } kept.extend(overflow); kept
} fn compile_glob(pattern: &str) -> std::result::Result<regex::Regex, regex::Error> {
    let mut result = String::from("^"); let mut chars = pattern.chars().peekable(); while let Some(c) = chars.next() {
        match c {
            '*' if chars.peek() == Some(&'*') => {
                chars.next(); if chars.peek() == Some(&'/') { chars.next(); result.push_str("(?:.*/)?"); } else {
                    result.push_str(".*");
                }
            } '*' => result.push_str(".*"), '?' => result.push('.'), c if "\\.+()|[]{}^$".contains(c) => {
                result.push('\\'); result.push(c);
            } _ => result.push(c),
        }
    } result.push('$'); regex::Regex::new(&result)
} fn contains_term_token(text: &str, term: &str) -> bool {
    !term.is_empty()
        && text.match_indices(term).any(|(start, matched)| {
            let before = text[..start].chars().next_back(); let after = text[start + matched.len()..].chars().next(); before.is_none_or(|ch| !ch.is_alphanumeric() && ch != '_')
                && after.is_none_or(|ch| !ch.is_alphanumeric() && ch != '_')
        })
}

fn excerpt_term_coverage(terms: &[String], hit: &SearchHit) -> u32 {
    let text = hit.excerpt.to_lowercase(); terms
        .iter() .filter(|term| contains_term_token(&text, term)) .count() as u32
}

#[cfg(test)] mod tests {
    use super::*;

    fn hit(file: &str, line: u32, score: f64) -> SearchHit {
        SearchHit {
            kind: HitKind::Asgrep, file: file.to_owned(), line_start: line, line_end: line, symbol: None, caller: None, callee: None, language: None, score, excerpt: String::new(),
        }
    }

    #[test] fn rerank_can_promote_candidate_beyond_final_limit() {
        let options = SearchOptions {
            limit: 16, use_rerank: true, rerank_top_k: 20, ..SearchOptions::default()
        }; let hits: Vec<_> = (0..20)
            .map(|index| hit(&format!("candidate-{index}.rs"), index + 1, 1.0 - f64::from(index) / 100.0)) .collect();

        let candidates = enforce_result_gates(hits, false, rerank_candidate_limit(&options)); assert_eq!(candidates.len(), 20);
        let reranked = apply_rerank_order(candidates, options.rerank_top_k, [(16, 1.0)]); let final_hits = enforce_result_gates(reranked, false, options.limit);

        assert_eq!(final_hits.len(), options.limit); assert_eq!(final_hits[0].file, "candidate-16.rs");
    }

    #[test] fn rerank_reorders_prefix_without_overwriting_fused_scores() {
        let hits = vec![
            hit("a.rs", 1, 0.9), hit("b.rs", 2, 0.8), hit("c.rs", 3, 0.7), hit("tail.rs", 4, 0.6),
        ];

        let reranked = apply_rerank_order(
            hits, 3, [(2, 0.99), (0, 0.5), (7, 1.0), (2, 0.2), (1, f32::NAN)],
        );

        let identity: Vec<_> = reranked
            .iter() .map(|hit| (hit.file.as_str(), hit.score)) .collect();
        assert_eq!(
            identity, vec![("c.rs", 0.7), ("a.rs", 0.9), ("b.rs", 0.8), ("tail.rs", 0.6)]
        );
    }

    #[test] fn hybrid_cap_and_limit_are_reapplied_after_rerank() {
        let hits = vec![
            hit("a.rs", 1, 0.9), hit("a.rs", 2, 0.8), hit("a.rs", 3, 0.7), hit("a.rs", 4, 0.6), hit("b.rs", 1, 0.5),
        ]; let reranked = apply_rerank_order(
            hits, 5, [(3, 1.0), (2, 0.9), (1, 0.8), (0, 0.7), (4, 0.1)],
        );

        let gated = enforce_result_gates(reranked, true, 4); let identity: Vec<_> = gated
            .iter() .map(|hit| (hit.file.as_str(), hit.line_start, hit.score)) .collect();
        assert_eq!(
            identity, vec![
                ("a.rs", 4, 0.6), ("a.rs", 3, 0.7), ("a.rs", 2, 0.8), ("b.rs", 1, 0.5),
            ]
        );
    }
}
