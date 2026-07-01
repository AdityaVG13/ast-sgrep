mod hits;
mod passes;
mod types;

pub use types::{HitKind, SearchHit, SearchOptions, SearchResponse};

use std::sync::{Arc, Mutex};
use std::thread;

use ast_sgrep_embed::SemanticChunkRow;

use crate::query::{ParsedQuery, QueryMode};
use crate::semantic_ann::flatten_vectors_for_search;
use crate::store::IndexStore;
use crate::Result;

use hits::dedup_hits;
use passes::embed::{embed_pass_with_context, EmbedContext};
use passes::lexical::lexical_pass;
use passes::modes::{search_callers, search_defs, search_imports};
use passes::symbol::{anchor_pass, symbol_pass};

/// File count at which lexical/symbol/anchor passes run on separate DB connections.
const PARALLEL_PASS_FILE_THRESHOLD: usize = 128;

struct SemanticCache {
    lang_filter: Option<String>,
    max_id: i64,
    embed_backend: String,
    chunks: Arc<Vec<SemanticChunkRow>>,
    flat_vectors: Arc<Vec<f32>>,
}

pub struct Searcher {
    store: IndexStore,
    options: SearchOptions,
    semantic_cache: Arc<Mutex<Option<SemanticCache>>>,
}

impl Searcher {
    pub fn new(options: SearchOptions) -> Result<Self> {
        let store = IndexStore::open(&options.root, options.index_path.as_deref())?;
        Ok(Self {
            store,
            options,
            semantic_cache: Arc::new(Mutex::new(None)),
        })
    }

    pub fn with_store(store: IndexStore, options: SearchOptions) -> Self {
        Self {
            store,
            options,
            semantic_cache: Arc::new(Mutex::new(None)),
        }
    }

    pub fn store(&self) -> &IndexStore {
        &self.store
    }

    pub fn search(&self, query_str: &str) -> Result<SearchResponse> {
        let parsed = ParsedQuery::parse(query_str);

        let hits = match parsed.mode {
            QueryMode::Callers => search_callers(&self.store, &parsed)?,
            QueryMode::Defs => search_defs(&self.store, &parsed)?,
            QueryMode::Imports => search_imports(&self.store, &parsed)?,
            QueryMode::Pattern => {
                let pattern = parsed.terms.first().map(|s| s.as_str()).unwrap_or("");
                crate::pattern::search_pattern(
                    pattern,
                    &self.options.root,
                    self.options.lang_filter.as_deref(),
                )?
            }
            QueryMode::Hybrid => self.search_hybrid(&parsed)?,
        };

        Ok(finish_response(&parsed, self.options.limit, hits, true))
    }

    pub fn search_semantic(&self, query_str: &str) -> Result<SearchResponse> {
        let parsed = ParsedQuery::parse(query_str);
        let ctx = self.semantic_context()?;
        let hits = embed_pass_with_context(&self.store, &self.options, &parsed, ctx)?;
        Ok(finish_response(&parsed, self.options.limit, hits, false))
    }

    fn search_hybrid(&self, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
        let parallel_sql = self
            .store
            .status()
            .map(|s| s.file_count >= PARALLEL_PASS_FILE_THRESHOLD)
            .unwrap_or(false);

        if parallel_sql {
            self.search_hybrid_large(parsed)
        } else {
            self.search_hybrid_small(parsed)
        }
    }

    fn search_hybrid_small(&self, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
        let mut hits = run_serial_passes(&self.store, &self.options, parsed)?;
        if self.options.use_embed {
            if let Some(ctx) = self.semantic_context()? {
                hits.extend(embed_pass_with_context(
                    &self.store,
                    &self.options,
                    parsed,
                    Some(ctx),
                )?);
            }
        }
        Ok(hits)
    }

    fn search_hybrid_large(&self, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
        let root = self.options.root.clone();
        let index_path = self.options.index_path.clone();
        let options = self.options.clone();
        let parsed = parsed.clone();

        let parsed_sql = parsed.clone();

        thread::scope(|scope| -> Result<Vec<SearchHit>> {
            let embed = if self.options.use_embed {
                let cache = Arc::clone(&self.semantic_cache);
                let root_e = root.clone();
                let index_path_e = index_path.clone();
                let options_e = options.clone();
                Some(scope.spawn(move || {
                    let store = IndexStore::open(&root_e, index_path_e.as_deref())?;
                    load_semantic_context(&store, &options_e, &cache)
                }))
            } else {
                None
            };

            let sql = scope.spawn(move || {
                run_parallel_passes(&root, index_path.as_deref(), &options, &parsed_sql)
            });

            let mut hits = join_worker(sql.join())?;
            if let Some(embed) = embed {
                if let Some(ctx) = join_worker(embed.join())? {
                    hits.extend(embed_pass_with_context(
                        &self.store,
                        &self.options,
                        &parsed,
                        Some(ctx),
                    )?);
                }
            }
            Ok(hits)
        })
    }

    fn semantic_context(&self) -> Result<Option<EmbedContext>> {
        load_semantic_context(&self.store, &self.options, &self.semantic_cache)
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
    let max_id = store.semantic_chunk_max_id()?.unwrap_or(0);
    let lang_filter = options.lang_filter.clone();
    let embed_backend = store
        .get_meta("embed_backend")?
        .unwrap_or_else(|| "semantic".to_string());
    {
        let guard = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = guard.as_ref() {
            if cached.lang_filter == lang_filter
                && cached.max_id == max_id
                && cached.embed_backend == embed_backend
            {
                return Ok(Some(EmbedContext {
                    chunks: Arc::clone(&cached.chunks),
                    flat_vectors: Arc::clone(&cached.flat_vectors),
                }));
            }
        }
    }

    let chunks = store.all_semantic_chunks(lang_filter.as_deref())?;
    if chunks.is_empty() {
        return Ok(None);
    }
    let dim = chunks[0].5.len();
    let flat_vectors = flatten_vectors_for_search(&chunks, dim);
    let entry = SemanticCache {
        lang_filter,
        max_id,
        embed_backend,
        chunks: Arc::new(chunks),
        flat_vectors: Arc::new(flat_vectors),
    };
    let ctx = EmbedContext {
        chunks: Arc::clone(&entry.chunks),
        flat_vectors: Arc::clone(&entry.flat_vectors),
    };
    *cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(entry);
    Ok(Some(ctx))
}

fn run_serial_passes(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let mut hits = Vec::with_capacity(64);
    hits.extend(lexical_pass(store, options, parsed)?);
    hits.extend(symbol_pass(store, options, parsed)?);
    hits.extend(anchor_pass(store, options, parsed)?);
    Ok(hits)
}

fn run_parallel_passes(
    root: &std::path::Path,
    index_path: Option<&std::path::Path>,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let parsed = parsed.clone();
    let options = options.clone();
    let root = root.to_path_buf();
    let index_path = index_path.map(|p| p.to_path_buf());

    thread::scope(|scope| -> Result<Vec<SearchHit>> {
        let lex = scope.spawn(|| {
            let store = IndexStore::open(&root, index_path.as_deref())?;
            lexical_pass(&store, &options, &parsed)
        });
        let sym = scope.spawn(|| {
            let store = IndexStore::open(&root, index_path.as_deref())?;
            symbol_pass(&store, &options, &parsed)
        });
        let anchor = scope.spawn(|| {
            let store = IndexStore::open(&root, index_path.as_deref())?;
            anchor_pass(&store, &options, &parsed)
        });

        let mut hits = join_worker(lex.join())?;
        hits.extend(join_worker(sym.join())?);
        hits.extend(join_worker(anchor.join())?);
        Ok(hits)
    })
}

fn join_worker<T>(join: thread::Result<Result<T>>) -> Result<T> {
    join.map_err(|e| crate::StoreError::Other(format!("search worker panicked: {e:?}")))?
}

fn finish_response(
    parsed: &ParsedQuery,
    limit: usize,
    mut hits: Vec<SearchHit>,
    dedup: bool,
) -> SearchResponse {
    if dedup {
        hits = dedup_hits(hits);
    }
    if hits.len() > limit {
        let nth = limit.saturating_sub(1);
        hits.select_nth_unstable_by(nth, |a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(limit);
    }
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    SearchResponse {
        query: parsed.raw.clone(),
        limit,
        hits,
    }
}
