use rusqlite::params;

use crate::query::ParsedQuery;
use crate::rank::score_lexical_rrf;
use crate::store::IndexStore;
use crate::Result;
use crate::search::types::{HitKind, SearchHit, SearchOptions};

pub fn lexical_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() {
        return Ok(Vec::new());
    }

    if options.use_tantivy {
        let sidecar = crate::tantivy_index::TantivySidecar::open_for_index(
            &options.root,
            options.index_path.as_deref(),
        )?;
        if sidecar.exists() {
            return lexical_from_sidecar(options, parsed, &sidecar);
        }
    }

    lexical_from_fts(store, options, parsed)
}

fn lexical_from_sidecar(
    options: &SearchOptions,
    parsed: &ParsedQuery,
    sidecar: &crate::tantivy_index::TantivySidecar,
) -> Result<Vec<SearchHit>> {
    let results = sidecar.search(&parsed.terms, 100)?;
    let mut line_ranks: std::collections::HashMap<(String, u32), Vec<usize>> =
        std::collections::HashMap::new();
    let mut line_meta: std::collections::HashMap<(String, u32), (Option<String>, String)> =
        std::collections::HashMap::new();
    for (file, line_no, content, language, rank) in results {
        if let Some(ref lang_filter) = options.lang_filter {
            if language.as_deref() != Some(lang_filter.as_str()) {
                continue;
            }
        }
        let key = (file.clone(), line_no);
        line_ranks.entry(key.clone()).or_default().push(rank);
        line_meta.insert(key, (language, content));
    }
    let mut hits = hits_from_ranks(line_ranks, line_meta);
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(hits)
}

fn lexical_from_fts(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let conn = store.connection();
    let mut line_ranks: std::collections::HashMap<(String, u32), Vec<usize>> =
        std::collections::HashMap::new();
    let mut line_meta: std::collections::HashMap<(String, u32), (Option<String>, String)> =
        std::collections::HashMap::new();

    for term in &parsed.terms {
        let fts_term = crate::fts::escape_fts_term(term);
        let mut stmt = conn.prepare(
            "SELECT f.path, f.language, l.line_no, l.content
             FROM lines_fts
             JOIN files f ON f.id = lines_fts.file_id
             JOIN lines l ON l.file_id = lines_fts.file_id AND l.line_no = lines_fts.line_no
             WHERE lines_fts MATCH ?1
             ORDER BY bm25(lines_fts)
             LIMIT 100",
        )?;
        let rows = stmt.query_map(params![fts_term], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, u32>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        for (rank, row) in rows.enumerate() {
            let (path, language, line_no, content) = row?;
            if let Some(ref lang_filter) = options.lang_filter {
                if language.as_deref() != Some(lang_filter.as_str()) {
                    continue;
                }
            }
            let key = (path.clone(), line_no);
            line_ranks.entry(key.clone()).or_default().push(rank);
            line_meta.insert(key, (language, content));
        }
    }

    let mut hits = hits_from_ranks(line_ranks, line_meta);
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(hits)
}

fn hits_from_ranks(
    line_ranks: std::collections::HashMap<(String, u32), Vec<usize>>,
    line_meta: std::collections::HashMap<(String, u32), (Option<String>, String)>,
) -> Vec<SearchHit> {
    line_ranks
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
        .collect()
}
