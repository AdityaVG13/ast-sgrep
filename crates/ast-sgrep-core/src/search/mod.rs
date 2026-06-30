mod hits;
mod passes;
mod types;

pub use types::{HitKind, SearchHit, SearchOptions, SearchResponse};

use crate::query::{ParsedQuery, QueryMode};
use crate::store::IndexStore;
use crate::Result;

use hits::dedup_hits;
use passes::embed::embed_pass;
use passes::lexical::lexical_pass;
use passes::modes::{search_callers, search_defs, search_imports};
use passes::symbol::{anchor_pass, symbol_pass};

/// Hybrid search engine over the index.
pub struct Searcher {
    store: IndexStore,
    options: SearchOptions,
}

impl Searcher {
    pub fn new(options: SearchOptions) -> Result<Self> {
        let store = IndexStore::open(&options.root, options.index_path.as_deref())?;
        Ok(Self { store, options })
    }

    pub fn with_store(store: IndexStore, options: SearchOptions) -> Self {
        Self { store, options }
    }

    pub fn store(&self) -> &IndexStore {
        &self.store
    }

    pub fn search(&self, query_str: &str) -> Result<SearchResponse> {
        let parsed = ParsedQuery::parse(query_str);

        let hits = match parsed.mode {
            QueryMode::Callers => search_callers(&self.store, &parsed, &|p, s, e| {
                self.excerpt_for_span(p, s, e)
            })?,
            QueryMode::Defs => search_defs(&self.store, &parsed, &|p, s, e| {
                self.excerpt_for_span(p, s, e)
            })?,
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

    /// Semantic-only search — runs the embed pass without lexical/symbol fusion.
    pub fn search_semantic(&self, query_str: &str) -> Result<SearchResponse> {
        let parsed = ParsedQuery::parse(query_str);
        let hits = embed_pass(&self.store, &self.options, &parsed)?;
        Ok(finish_response(&parsed, self.options.limit, hits, false))
    }

    fn search_hybrid(&self, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
        let excerpt = |p: &str, s: u32, e: u32| self.excerpt_for_span(p, s, e);
        let mut hits = Vec::new();
        hits.extend(lexical_pass(&self.store, &self.options, parsed)?);
        hits.extend(symbol_pass(&self.store, &self.options, parsed, &excerpt)?);
        hits.extend(anchor_pass(&self.store, &self.options, parsed, &excerpt)?);
        if self.options.use_embed {
            hits.extend(embed_pass(&self.store, &self.options, parsed)?);
        }
        Ok(hits)
    }

    fn excerpt_for_span(
        &self,
        rel_path: &str,
        line_start: u32,
        line_end: u32,
    ) -> Result<String> {
        self.store.excerpt_span(rel_path, line_start, line_end)
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
