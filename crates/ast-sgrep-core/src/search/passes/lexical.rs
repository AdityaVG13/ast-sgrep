use std::collections::HashMap;

use rusqlite::params;

use crate::query::ParsedQuery;
use crate::rank::score_lexical_rrf;
use crate::store::IndexStore;
use crate::Result;
use crate::search::hits::matches_lang;
use crate::search::types::{HitKind, SearchHit, SearchOptions};

type LineRanks = HashMap<(String, u32), Vec<usize>>;
type LineMeta = HashMap<(String, u32), (Option<String>, String)>;

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
    let (mut line_ranks, mut line_meta) = empty_lexical_maps();
    for (file, line_no, content, language, rank) in results {
        accumulate_lexical_line(
            options,
            &mut line_ranks,
            &mut line_meta,
            file,
            line_no,
            language,
            content,
            rank,
        );
    }
    Ok(finalize_lexical_hits(line_ranks, line_meta))
}

fn lexical_from_fts(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let conn = store.connection();
    let (mut line_ranks, mut line_meta) = empty_lexical_maps();
    let fts_query = crate::fts::escape_fts_query(&parsed.terms);
    let mut stmt = conn.prepare_cached(
        "SELECT f.path, f.language, l.line_no, l.content
         FROM lines_fts
         JOIN files f ON f.id = lines_fts.file_id
         JOIN lines l ON l.file_id = lines_fts.file_id AND l.line_no = lines_fts.line_no
         WHERE lines_fts MATCH ?1
         ORDER BY bm25(lines_fts)
         LIMIT 100",
    )?;
    let rows = stmt.query_map(params![fts_query], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, u32>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    for (rank, row) in rows.enumerate() {
        let (path, language, line_no, content) = row?;
        accumulate_lexical_line(
            options,
            &mut line_ranks,
            &mut line_meta,
            path,
            line_no,
            language,
            content,
            rank,
        );
    }

    Ok(finalize_lexical_hits(line_ranks, line_meta))
}

fn empty_lexical_maps() -> (LineRanks, LineMeta) {
    (HashMap::new(), HashMap::new())
}

fn accumulate_lexical_line(
    options: &SearchOptions,
    line_ranks: &mut LineRanks,
    line_meta: &mut LineMeta,
    path: String,
    line_no: u32,
    language: Option<String>,
    content: String,
    rank: usize,
) {
    if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
        return;
    }
    let key = (path.clone(), line_no);
    line_ranks.entry(key.clone()).or_default().push(rank);
    line_meta.insert(key, (language, content));
}

fn finalize_lexical_hits(line_ranks: LineRanks, line_meta: LineMeta) -> Vec<SearchHit> {
    hits_from_ranks(line_ranks, line_meta)
}

fn hits_from_ranks(line_ranks: LineRanks, line_meta: LineMeta) -> Vec<SearchHit> {
    line_ranks
        .into_iter()
        .map(|((path, line_no), ranks)| {
            let (language, content) = line_meta
                .get(&(path.clone(), line_no))
                .cloned()
                .unwrap_or((None, String::new()));
            SearchHit::span(
                HitKind::Asgrep,
                path,
                line_no,
                line_no,
                score_lexical_rrf(&ranks),
                content,
                None,
                language,
            )
        })
        .collect()
}
