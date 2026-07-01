mod hits;
mod passes;
mod types;

pub use types::{HitKind, SearchHit, SearchOptions, SearchResponse};

use std::sync::{Arc, Mutex};

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

struct SemanticCache {
    lang_filter: Option<String>,
    max_id: i64,
    chunks: Arc<Vec<SemanticChunkRow>>,
    flat_vectors: Arc<Vec<f32>>,
}

pub struct Searcher {
    store: IndexStore,
    options: SearchOptions,
    semantic_cache: Mutex<Option<SemanticCache>>,
}

impl Searcher {
    pub fn new(options: SearchOptions) -> Result<Self> {
        let store = IndexStore::open(&options.root, options.index_path.as_deref())?;
        Ok(Self {
            store,
            options,
            semantic_cache: Mutex::new(None),
        })
    }

    pub fn with_store(store: IndexStore, options: SearchOptions) -> Self {
        Self {
            store,
            options,
            semantic_cache: Mutex::new(None),
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
        let mut hits = Vec::new();
        hits.extend(lexical_pass(&self.store, &self.options, parsed)?);
        hits.extend(symbol_pass(&self.store, &self.options, parsed)?);
        hits.extend(anchor_pass(&self.store, &self.options, parsed)?);
        if self.options.use_embed {
            let ctx = self.semantic_context()?;
            hits.extend(embed_pass_with_context(
                &self.store,
                &self.options,
                parsed,
                ctx,
            )?);
        }
        Ok(hits)
    }

    fn semantic_context(&self) -> Result<Option<EmbedContext>> {
        if !self.options.use_embed {
            return Ok(None);
        }
        let max_id = self.store.semantic_chunk_max_id()?.unwrap_or(0);
        let lang_filter = self.options.lang_filter.clone();
        {
            let guard = self.semantic_cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(cache) = guard.as_ref() {
                if cache.lang_filter == lang_filter && cache.max_id == max_id {
                    return Ok(Some(EmbedContext {
                        chunks: Arc::clone(&cache.chunks),
                        flat_vectors: Arc::clone(&cache.flat_vectors),
                    }));
                }
            }
        }

        let chunks = self
            .store
            .all_semantic_chunks(lang_filter.as_deref())?;
        if chunks.is_empty() {
            return Ok(None);
        }
        let dim = chunks[0].5.len();
        let flat_vectors = flatten_vectors_for_search(&chunks, dim);
        let cache = SemanticCache {
            lang_filter,
            max_id,
            chunks: Arc::new(chunks),
            flat_vectors: Arc::new(flat_vectors),
        };
        let ctx = EmbedContext {
            chunks: Arc::clone(&cache.chunks),
            flat_vectors: Arc::clone(&cache.flat_vectors),
        };
        *self.semantic_cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(cache);
        Ok(Some(ctx))
    }
}

fn finish_response(
    parsed: &ParsedQuery,
    limit: usize,
    mut hits: Vec<SearchHit>,
    dedup: bool,
) -> SearchResponse {
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if dedup {
        hits = dedup_hits(hits);
    }
    hits.truncate(limit);
    SearchResponse {
        query: parsed.raw.clone(),
        limit,
        hits,
    }
}
