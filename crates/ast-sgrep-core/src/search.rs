use std::collections::HashSet;
use std::path::PathBuf;

use rusqlite::params;

use crate::query::{ParsedQuery, QueryMode};
use crate::rank::{
    best_symbol_score, score_caller, score_def, score_lexical_rrf, SCORE_ANCHOR, SCORE_EMBED,
    SCORE_GRAPH,
};
use ast_sgrep_embed::{embed_from_bytes, rank_by_similarity};
#[cfg(feature = "cloud-embed")]
use ast_sgrep_embed::{rank_by_vector, CloudEmbeddingConfig};
use crate::store::IndexStore;
use crate::Result;

/// Kind of search hit in output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HitKind {
    Asgrep,
    Def,
    Caller,
    Graph,
    Anchor,
    Import,
    /// Structural pattern match via ast-grep delegation.
    Pattern,
    /// Semantic similarity via local embedding plugin.
    Embed,
}

impl HitKind {
    pub fn as_str(self) -> &'static str {
        match self {
            HitKind::Asgrep => "asgrep",
            HitKind::Def => "def",
            HitKind::Caller => "caller",
            HitKind::Graph => "graph",
            HitKind::Anchor => "anchor",
            HitKind::Import => "import",
            HitKind::Pattern => "pattern",
            HitKind::Embed => "embed",
        }
    }
}

/// A single search result.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchHit {
    pub kind: HitKind,
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub score: f64,
    pub excerpt: String,
}

/// Search options.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub root: PathBuf,
    pub index_path: Option<PathBuf>,
    pub limit: usize,
    pub lang_filter: Option<String>,
    /// Enable local embedding semantic search pass.
    pub use_embed: bool,
    /// Use tantivy sidecar for lexical search (large repos).
    pub use_tantivy: bool,
    /// Use cloud API for query embeddings (requires ASGREP_EMBED_API_KEY).
    pub use_cloud_embed: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            index_path: None,
            limit: Self::default_limit(),
            lang_filter: None,
            use_embed: std::env::var("ASGREP_EMBED").ok().as_deref() == Some("1"),
            use_tantivy: std::env::var("ASGREP_TANTIVY").ok().as_deref() == Some("1"),
            use_cloud_embed: std::env::var("ASGREP_CLOUD_EMBED").ok().as_deref() == Some("1"),
        }
    }
}

impl SearchOptions {
    pub fn default_limit() -> usize {
        std::env::var("ASGREP_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(16)
    }
}

/// Search response wrapper for JSON output.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub limit: usize,
    pub hits: Vec<SearchHit>,
}

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

    pub fn search(&self, query_str: &str) -> Result<SearchResponse> {
        let parsed = ParsedQuery::parse(query_str);
        let limit = self.options.limit;

        let mut hits = match parsed.mode {
            QueryMode::Callers => self.search_callers(&parsed)?,
            QueryMode::Defs => self.search_defs(&parsed)?,
            QueryMode::Imports => self.search_imports(&parsed)?,
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

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits = dedup_hits(hits);
        hits.truncate(limit);

        Ok(SearchResponse {
            query: parsed.raw,
            limit,
            hits,
        })
    }

    fn search_hybrid(&self, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
        let mut hits = Vec::new();
        hits.extend(self.lexical_pass(parsed)?);
        hits.extend(self.symbol_pass(parsed)?);
        hits.extend(self.anchor_pass(parsed)?);
        if self.options.use_embed {
            hits.extend(self.embed_pass(parsed)?);
        }
        Ok(hits)
    }

    fn embed_pass(&self, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
        if parsed.terms.is_empty() {
            return Ok(Vec::new());
        }
        let query = parsed.terms.join(" ");
        let conn = self.store.connection();
        let mut stmt = conn.prepare(
            "SELECT f.path, l.line_no, l.content, f.language, e.vector
             FROM embeddings e
             JOIN lines l ON l.file_id = e.file_id AND l.line_no = e.line_no
             JOIN files f ON f.id = e.file_id",
        )?;
        let mut rows = stmt.query([])?;
        let mut lines = Vec::new();
        while let Some(row) = rows.next()? {
            let file: String = row.get(0)?;
            let line_no: u32 = row.get(1)?;
            let content: String = row.get(2)?;
            let lang: Option<String> = row.get(3)?;
            let vector: Vec<u8> = row.get(4)?;
            if let Some(ref lang_filter) = self.options.lang_filter {
                if lang.as_deref() != Some(lang_filter.as_str()) {
                    continue;
                }
            }
            lines.push((file, line_no, content, embed_from_bytes(&vector)));
        }
        let ranked = if self.options.use_cloud_embed {
            #[cfg(feature = "cloud-embed")]
            {
                if let Some(config) = CloudEmbeddingConfig::from_env() {
                    match ast_sgrep_embed::embed_via_api(&query, &config) {
                        Ok(query_vec) => rank_by_vector(&query_vec, &lines, 50),
                        Err(_) => rank_by_similarity(&query, &lines, 50),
                    }
                } else {
                    rank_by_similarity(&query, &lines, 50)
                }
            }
            #[cfg(not(feature = "cloud-embed"))]
            {
                rank_by_similarity(&query, &lines, 50)
            }
        } else {
            rank_by_similarity(&query, &lines, 50)
        };
        Ok(ranked
            .into_iter()
            .map(|(sim, file, line_no, content)| SearchHit {
                kind: HitKind::Embed,
                file,
                line_start: line_no,
                line_end: line_no,
                symbol: None,
                caller: None,
                callee: None,
                language: None,
                score: SCORE_EMBED * f64::from(sim),
                excerpt: content,
            })
            .collect())
    }

    fn lexical_pass(&self, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
        if parsed.terms.is_empty() {
            return Ok(Vec::new());
        }

        if self.options.use_tantivy {
            let sidecar = crate::tantivy_index::TantivySidecar::open(&self.options.root)?;
            if sidecar.exists() {
                let results = sidecar.search(&parsed.terms, 100)?;
                let mut line_ranks: std::collections::HashMap<(String, u32), Vec<usize>> =
                    std::collections::HashMap::new();
                let mut line_meta: std::collections::HashMap<(String, u32), (Option<String>, String)> =
                    std::collections::HashMap::new();
                for (file, line_no, content, language, rank) in results {
                    if let Some(ref lang_filter) = self.options.lang_filter {
                        if language.as_deref() != Some(lang_filter.as_str()) {
                            continue;
                        }
                    }
                    let key = (file.clone(), line_no);
                    line_ranks.entry(key.clone()).or_default().push(rank);
                    line_meta.insert(key, (language, content));
                }
                let mut hits: Vec<SearchHit> = line_ranks
                    .into_iter()
                    .map(|((path, line_no), ranks)| {
                        let (language, content) = line_meta
                            .get(&(path.clone(), line_no))
                            .cloned()
                            .unwrap_or((None, String::new()));
                        SearchHit {
                            kind: HitKind::Asgrep,
                            file: path,
                            line_start: line_no,
                            line_end: line_no,
                            symbol: None,
                            caller: None,
                            callee: None,
                            language,
                            score: score_lexical_rrf(&ranks),
                            excerpt: content,
                        }
                    })
                    .collect();
                hits.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                return Ok(hits);
            }
        }

        let conn = self.store.connection();
        // Per-term FTS with RRF fusion across ranked lists
        let mut line_ranks: std::collections::HashMap<(String, u32), Vec<usize>> =
            std::collections::HashMap::new();
        let mut line_meta: std::collections::HashMap<(String, u32), (Option<String>, String)> =
            std::collections::HashMap::new();

        for term in &parsed.terms {
            let mut stmt = conn.prepare(
                "SELECT f.path, f.language, l.line_no, l.content
                 FROM lines_fts
                 JOIN files f ON f.id = lines_fts.file_id
                 JOIN lines l ON l.file_id = lines_fts.file_id AND l.line_no = lines_fts.line_no
                 WHERE lines_fts MATCH ?1
                 ORDER BY bm25(lines_fts)
                 LIMIT 100",
            )?;
            let rows = stmt.query_map(params![term], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            for (rank, row) in rows.enumerate() {
                let (path, language, line_no, content) = row?;
                if let Some(ref lang_filter) = self.options.lang_filter {
                    if language.as_deref() != Some(lang_filter.as_str()) {
                        continue;
                    }
                }
                let key = (path.clone(), line_no);
                line_ranks.entry(key.clone()).or_default().push(rank);
                line_meta.insert(key, (language, content));
            }
        }

        let mut hits: Vec<SearchHit> = line_ranks
            .into_iter()
            .map(|((path, line_no), ranks)| {
                let (language, content) = line_meta
                    .get(&(path.clone(), line_no))
                    .cloned()
                    .unwrap_or((None, String::new()));
                SearchHit {
                    kind: HitKind::Asgrep,
                    file: path,
                    line_start: line_no,
                    line_end: line_no,
                    symbol: None,
                    caller: None,
                    callee: None,
                    language,
                    score: score_lexical_rrf(&ranks),
                    excerpt: content,
                }
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(hits)
    }

    fn symbol_pass(&self, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
        let mut hits = Vec::new();
        let conn = self.store.connection();

        // DEF hits
        let mut def_stmt = conn.prepare(
            "SELECT f.path, f.language, s.name, s.kind, s.line_start, s.line_end, s.byte_start, s.byte_end
             FROM symbols s
             JOIN files f ON f.id = s.file_id",
        )?;
        let def_rows = def_stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, u32>(5)?,
                row.get::<_, usize>(6)?,
                row.get::<_, usize>(7)?,
            ))
        })?;

        for row in def_rows {
            let (path, language, name, _kind, line_start, line_end, byte_start, byte_end) = row?;
            let sym_score = best_symbol_score(&parsed.terms, &name);
            if sym_score == 0.0 && !parsed.terms.is_empty() {
                continue;
            }
            if let Some(ref lang_filter) = self.options.lang_filter {
                if language.as_deref() != Some(lang_filter.as_str()) {
                    continue;
                }
            }
            let excerpt = self.excerpt_for_span(&path, byte_start, byte_end, line_start, line_end)?;
            hits.push(SearchHit {
                kind: HitKind::Def,
                file: path.clone(),
                line_start,
                line_end,
                symbol: Some(name.clone()),
                caller: None,
                callee: None,
                language: language.clone(),
                score: score_def(&parsed.terms, &name),
                excerpt,
            });
        }

        // CALLER hits
        let mut caller_stmt = conn.prepare(
            "SELECT f.path, f.language, c.caller, c.callee, c.line_no, c.byte_start, c.byte_end
             FROM callers c
             JOIN files f ON f.id = c.file_id",
        )?;
        let caller_rows = caller_stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, usize>(5)?,
                row.get::<_, usize>(6)?,
            ))
        })?;

        for row in caller_rows {
            let (path, language, caller, callee, line_no, byte_start, byte_end) = row?;
            let sym_score = best_symbol_score(&parsed.terms, &callee);
            if sym_score == 0.0 && !parsed.terms.is_empty() {
                // Also match caller name
                let caller_score = best_symbol_score(&parsed.terms, &caller);
                if caller_score == 0.0 {
                    continue;
                }
            }
            if let Some(ref lang_filter) = self.options.lang_filter {
                if language.as_deref() != Some(lang_filter.as_str()) {
                    continue;
                }
            }
            let excerpt = self.excerpt_for_span(&path, byte_start, byte_end, line_no, line_no)?;
            hits.push(SearchHit {
                kind: HitKind::Caller,
                file: path.clone(),
                line_start: line_no,
                line_end: line_no,
                symbol: None,
                caller: Some(caller.clone()),
                callee: Some(callee.clone()),
                language: language.clone(),
                score: score_caller(&parsed.terms, &callee),
                excerpt,
            });

            // GRAPH edge
            if sym_score > 0.0 || parsed.primary_symbol().is_some_and(|s| callee.contains(s)) {
                hits.push(SearchHit {
                    kind: HitKind::Graph,
                    file: path,
                    line_start: line_no,
                    line_end: line_no,
                    symbol: Some(callee.clone()),
                    caller: Some(caller.clone()),
                    callee: Some(callee.clone()),
                    language,
                    score: SCORE_GRAPH,
                    excerpt: format!("{caller} calls {callee}"),
                });
            }
        }

        Ok(hits)
    }

    fn anchor_pass(&self, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
        let anchor_symbol = match parsed.primary_symbol() {
            Some(s) => s.to_string(),
            None => {
                // Try to find best matching symbol from terms
                parsed
                    .terms
                    .iter()
                    .find(|t| t.len() > 3)
                    .cloned()
                    .unwrap_or_default()
            }
        };
        if anchor_symbol.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.store.connection();
        let mut stmt = conn.prepare(
            "SELECT f.path, f.language, s.name, s.line_start, s.line_end, s.byte_start, s.byte_end
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE lower(s.name) = lower(?1) OR lower(s.name) LIKE '%' || lower(?1) || '%'",
        )?;

        let rows = stmt.query_map(params![anchor_symbol], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u32>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, usize>(5)?,
                row.get::<_, usize>(6)?,
            ))
        })?;

        let mut hits = Vec::new();
        for row in rows {
            let (path, language, name, line_start, line_end, byte_start, byte_end) = row?;
            if let Some(ref lang_filter) = self.options.lang_filter {
                if language.as_deref() != Some(lang_filter.as_str()) {
                    continue;
                }
            }
            let excerpt = self.excerpt_for_span(&path, byte_start, byte_end, line_start, line_end)?;
            hits.push(SearchHit {
                kind: HitKind::Anchor,
                file: path,
                line_start,
                line_end,
                symbol: Some(name),
                caller: None,
                callee: None,
                language,
                score: SCORE_ANCHOR,
                excerpt,
            });
        }
        Ok(hits)
    }

    fn search_callers(&self, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
        let symbol = parsed
            .terms
            .first()
            .cloned()
            .unwrap_or_default();
        let conn = self.store.connection();
        let mut stmt = conn.prepare(
            "SELECT f.path, f.language, c.caller, c.callee, c.line_no, c.byte_start, c.byte_end
             FROM callers c
             JOIN files f ON f.id = c.file_id
             WHERE lower(c.callee) = lower(?1) OR lower(c.callee) LIKE '%' || lower(?1) || '%'",
        )?;

        let rows = stmt.query_map(params![symbol], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, usize>(5)?,
                row.get::<_, usize>(6)?,
            ))
        })?;

        let mut hits = Vec::new();
        for row in rows {
            let (path, language, caller, callee, line_no, byte_start, byte_end) = row?;
            let excerpt = self.excerpt_for_span(&path, byte_start, byte_end, line_no, line_no)?;
            hits.push(SearchHit {
                kind: HitKind::Caller,
                file: path.clone(),
                line_start: line_no,
                line_end: line_no,
                symbol: None,
                caller: Some(caller.clone()),
                callee: Some(callee.clone()),
                language: language.clone(),
                score: score_caller(&parsed.terms, &callee),
                excerpt: excerpt.clone(),
            });
            hits.push(SearchHit {
                kind: HitKind::Graph,
                file: path,
                line_start: line_no,
                line_end: line_no,
                symbol: Some(callee.clone()),
                caller: Some(caller.clone()),
                callee: Some(callee.clone()),
                language,
                score: SCORE_GRAPH,
                excerpt: format!("{caller} calls {callee}"),
            });
        }
        Ok(hits)
    }

    fn search_defs(&self, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
        let symbol = parsed
            .terms
            .first()
            .cloned()
            .unwrap_or_default();
        let conn = self.store.connection();
        let mut stmt = conn.prepare(
            "SELECT f.path, f.language, s.name, s.line_start, s.line_end, s.byte_start, s.byte_end
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE lower(s.name) = lower(?1) OR lower(s.name) LIKE '%' || lower(?1) || '%'",
        )?;

        let rows = stmt.query_map(params![symbol], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u32>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, usize>(5)?,
                row.get::<_, usize>(6)?,
            ))
        })?;

        let mut hits = Vec::new();
        for row in rows {
            let (path, language, name, line_start, line_end, byte_start, byte_end) = row?;
            let excerpt = self.excerpt_for_span(&path, byte_start, byte_end, line_start, line_end)?;
            hits.push(SearchHit {
                kind: HitKind::Def,
                file: path,
                line_start,
                line_end,
                symbol: Some(name.clone()),
                caller: None,
                callee: None,
                language,
                score: score_def(&parsed.terms, &name),
                excerpt,
            });
        }
        Ok(hits)
    }

    fn search_imports(&self, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
        let module = parsed.terms.join(" ");
        let conn = self.store.connection();

        let mut hits = Vec::new();
        if module.is_empty() {
            let mut stmt = conn.prepare(
                "SELECT f.path, f.language, i.module_path, i.line_no
                 FROM imports i
                 JOIN files f ON f.id = i.file_id",
            )?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let path: String = row.get(0)?;
                let language: Option<String> = row.get(1)?;
                let module_path: String = row.get(2)?;
                let line_no: u32 = row.get(3)?;
                hits.push(import_hit(path, language, module_path, line_no));
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT f.path, f.language, i.module_path, i.line_no
                 FROM imports i
                 JOIN files f ON f.id = i.file_id
                 WHERE lower(i.module_path) LIKE '%' || lower(?1) || '%'",
            )?;
            let mut rows = stmt.query(params![module])?;
            while let Some(row) = rows.next()? {
                let path: String = row.get(0)?;
                let language: Option<String> = row.get(1)?;
                let module_path: String = row.get(2)?;
                let line_no: u32 = row.get(3)?;
                hits.push(import_hit(path, language, module_path, line_no));
            }
        }

        Ok(hits)
    }

    fn excerpt_for_span(
        &self,
        rel_path: &str,
        _byte_start: usize,
        _byte_end: usize,
        line_start: u32,
        line_end: u32,
    ) -> Result<String> {
        let conn = self.store.connection();
        let mut stmt = conn.prepare(
            "SELECT l.content FROM lines l
             JOIN files f ON f.id = l.file_id
             WHERE f.path = ?1 AND l.line_no >= ?2 AND l.line_no <= ?3
             ORDER BY l.line_no",
        )?;
        let rows = stmt.query_map(params![rel_path, line_start, line_end], |row| {
            row.get::<_, String>(0)
        })?;
        let lines: Vec<String> = rows.collect::<std::result::Result<_, _>>()?;
        Ok(lines.join("\n"))
    }
}

fn import_hit(
    path: String,
    language: Option<String>,
    module_path: String,
    line_no: u32,
) -> SearchHit {
    SearchHit {
        kind: HitKind::Import,
        file: path,
        line_start: line_no,
        line_end: line_no,
        symbol: Some(module_path.clone()),
        caller: None,
        callee: None,
        language,
        score: 2.0,
        excerpt: format!("import {module_path}"),
    }
}

fn dedup_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for hit in hits {
        let key = (
            hit.kind.as_str(),
            hit.file.clone(),
            hit.line_start,
            hit.line_end,
            hit.symbol.clone(),
            hit.caller.clone(),
            hit.callee.clone(),
        );
        if seen.insert(key) {
            out.push(hit);
        }
    }
    out
}
