use crate::query::ParsedQuery;
use crate::rank::{
    best_symbol_score_normalized, normalize_query_terms, score_caller_normalized, score_def,
    SCORE_ANCHOR, SCORE_GRAPH,
};
use crate::search::passes::bmh::retained_limit;
use crate::search::types::matches_lang;
use crate::search::types::{HitKind, SearchHit, SearchOptions, SpanHitInput};
use crate::store::sql::{caller_terms_filter, like_terms_filter, query_limit_map};
use crate::store::IndexStore;
use crate::Result;
use std::collections::{HashMap, HashSet};
const SYMBOL_SQL_LIMIT: usize = 500;
const CALLER_SQL_LIMIT: usize = 500;
const MODE_SQL_LIMIT: usize = 200;
const TYPE_SYMBOL_WEIGHT: f64 = 0.65;
const SYMBOL_KIND_ORDER: &str =
    " ORDER BY CASE WHEN s.kind IN ('function','method') THEN 0 ELSE 1 END, s.id";
const SYMBOL_SELECT: &str = "SELECT f.path, f.language, s.name, s.kind, s.line_start, s.line_end
         FROM symbols s JOIN files f ON f.id = s.file_id";
const CALLER_SELECT: &str = "SELECT f.path, f.language, c.caller, c.callee, c.line_no
         FROM callers c JOIN files f ON f.id = c.file_id";
type CallerQueryRow = (String, Option<String>, String, String, u32);
type CallerFilter = fn(&[String], Option<&str>) -> (String, Vec<String>);
type SymbolSpanRow = (String, Option<String>, String, String, u32, u32);
enum CallerMatchMode {
    Hybrid,
    CalleeOnly,
}
fn map_caller_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CallerQueryRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}
fn restrict_to_files(
    where_clause: &mut String,
    bind: &mut Vec<String>,
    allowed_files: Option<&HashSet<String>>,
) {
    let Some(allowed_files) = allowed_files else {
        return;
    };
    let mut paths = allowed_files.iter().collect::<Vec<_>>();
    paths.sort_unstable();
    where_clause.push_str(" AND f.path IN (");
    where_clause.push_str(&vec!["?"; paths.len()].join(","));
    where_clause.push(')');
    bind.extend(paths.into_iter().cloned());
}

fn query_caller_rows(
    store: &IndexStore,
    filter: CallerFilter,
    terms: &[String],
    lang_filter: Option<&str>,
    allowed_files: Option<&HashSet<String>>,
    limit: usize,
) -> Result<Vec<CallerQueryRow>> {
    let (mut where_clause, mut bind) = filter(terms, lang_filter);
    restrict_to_files(&mut where_clause, &mut bind, allowed_files);
    let sql = format!("{CALLER_SELECT}{where_clause} LIMIT ?{}", bind.len() + 1);
    query_limit_map(store.connection(), &sql, bind, limit, map_caller_row)
}
fn caller_rows_to_hits(
    store: &IndexStore,
    rows: Vec<CallerQueryRow>,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    mode: CallerMatchMode,
) -> Result<Vec<SearchHit>> {
    caller_rows_to_hits_resolved(store, rows, options, parsed, mode, None)
}

/// dvc4: same as above, but classifies how each name match resolved when a
/// store is available to count candidates.
fn caller_rows_to_hits_resolved(
    excerpt_store: &IndexStore,
    rows: Vec<CallerQueryRow>,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    mode: CallerMatchMode,
    store: Option<&IndexStore>,
) -> Result<Vec<SearchHit>> {
    let primary_lower = parsed.primary_symbol().map(|s| s.to_lowercase());
    // am6l: normalize query terms once per query, not once per scored row.
    let norm_terms = normalize_query_terms(&parsed.terms);
    let mut caller_hits = Vec::new();
    let mut graph_hits = Vec::new();
    for (path, language, caller, callee, line_no) in rows {
        if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
            continue;
        }
        let callee_score = best_symbol_score_normalized(&norm_terms, &callee);
        let matched = match mode {
            CallerMatchMode::Hybrid => {
                callee_score > 0.0 || best_symbol_score_normalized(&norm_terms, &caller) > 0.0
            }
            CallerMatchMode::CalleeOnly => callee_score > 0.0,
        };
        if !matched {
            continue;
        }
        caller_hits.push(SearchHit::caller(
            path.clone(),
            language.clone(),
            caller.clone(),
            callee.clone(),
            line_no,
            String::new(),
            score_caller_normalized(&norm_terms, &callee),
        ));
        let graph = match mode {
            CallerMatchMode::CalleeOnly => Some(SCORE_GRAPH),
            CallerMatchMode::Hybrid => {
                let exact = callee_score >= crate::rank::SCORE_EXACT_SYMBOL
                    || primary_lower
                        .as_ref()
                        .is_some_and(|s| callee.to_lowercase() == *s);
                exact.then_some(SCORE_GRAPH)
            }
        };
        if let Some(graph_score) = graph {
            graph_hits.push(SearchHit::graph_scored(
                path,
                language,
                caller,
                callee,
                line_no,
                graph_score,
            ));
        }
    }
    retain_scored_hits(&mut caller_hits, options);
    retain_scored_hits(&mut graph_hits, options);
    attach_indexed_excerpts(excerpt_store, &mut caller_hits)?;
    if let Some(store) = store {
        let mut candidate_counts = HashMap::new();
        attach_caller_resolutions(store, &mut candidate_counts, &mut caller_hits)?;
        attach_caller_resolutions(store, &mut candidate_counts, &mut graph_hits)?;
    }
    caller_hits.extend(graph_hits);
    Ok(caller_hits)
}

fn retain_scored_hits(hits: &mut Vec<SearchHit>, options: &SearchOptions) {
    let limit = retained_limit(options);
    hits.sort_unstable_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.file.cmp(&right.file))
            .then_with(|| left.line_start.cmp(&right.line_start))
            .then_with(|| left.symbol.cmp(&right.symbol))
    });
    hits.truncate(limit);
}

fn attach_indexed_excerpts(store: &IndexStore, hits: &mut [SearchHit]) -> Result<()> {
    for hit in hits {
        hit.excerpt = store.indexed_excerpt_in_range(&hit.file, hit.line_start, hit.line_end)?;
    }
    Ok(())
}

fn attach_caller_resolutions(
    store: &IndexStore,
    candidate_counts: &mut HashMap<String, (HashMap<String, usize>, usize)>,
    hits: &mut [SearchHit],
) -> Result<()> {
    for hit in hits {
        let Some(callee) = hit.callee.as_deref() else {
            continue;
        };
        if !candidate_counts.contains_key(callee) {
            candidate_counts.insert(
                callee.to_owned(),
                store.symbol_name_candidate_counts(callee)?,
            );
        }
        let (by_file, repo) = &candidate_counts[callee];
        let same_file = by_file.get(&hit.file).copied().unwrap_or(0);
        hit.resolution = Some(crate::resolution::Resolution::from_candidates(
            same_file,
            *repo,
            std::iter::empty(),
        ));
    }
    Ok(())
}
fn read_symbol_span_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolSpanRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}
fn query_symbol_spans(
    store: &IndexStore,
    where_clause: &str,
    bind: Vec<String>,
    limit: usize,
) -> Result<Vec<SymbolSpanRow>> {
    let sql = format!(
        "{SYMBOL_SELECT}{where_clause}{SYMBOL_KIND_ORDER} LIMIT ?{}",
        bind.len() + 1
    );
    query_limit_map(store.connection(), &sql, bind, limit, read_symbol_span_row)
}
fn kind_weight(kind: &str) -> f64 {
    match kind {
        "function" | "method" => 1.0,
        _ => TYPE_SYMBOL_WEIGHT,
    }
}
fn symbol_span_rows_to_hits(
    store: &IndexStore,
    rows: Vec<SymbolSpanRow>,
    options: &SearchOptions,
    kind: HitKind,
    score_for: impl Fn(&str) -> f64,
) -> Result<Vec<SearchHit>> {
    let mut hits = Vec::with_capacity(rows.len());
    for (path, language, name, sym_kind, line_start, line_end) in rows {
        if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
            continue;
        }
        hits.push(SearchHit::span(SpanHitInput {
            kind,
            file: path,
            line_start,
            line_end,
            score: score_for(&name) * kind_weight(&sym_kind),
            excerpt: String::new(),
            symbol: Some(name),
            language,
        }));
    }
    retain_scored_hits(&mut hits, options);
    attach_indexed_excerpts(store, &mut hits)?;
    Ok(hits)
}
pub fn symbol_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let mut hits = def_hits_for_terms(store, options, parsed, SYMBOL_SQL_LIMIT)?;
    hits.extend(caller_hits_for_terms(
        store,
        options,
        parsed,
        CALLER_SQL_LIMIT,
    )?);
    Ok(hits)
}
pub fn symbol_pass_for_files(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    allowed_files: &HashSet<String>,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() || allowed_files.is_empty() {
        return Ok(Vec::new());
    }
    let (mut where_clause, mut bind) =
        like_terms_filter("s.name", &parsed.terms, options.lang_filter.as_deref());
    restrict_to_files(&mut where_clause, &mut bind, Some(allowed_files));
    let rows = query_symbol_spans(store, &where_clause, bind, SYMBOL_SQL_LIMIT)?;
    let mut hits = symbol_span_rows_to_hits(store, rows, options, HitKind::Def, |name| {
        score_def(&parsed.terms, name)
    })?;
    hits.extend(caller_rows_to_hits(
        store,
        query_caller_rows(
            store,
            caller_terms_filter,
            &parsed.terms,
            options.lang_filter.as_deref(),
            Some(allowed_files),
            CALLER_SQL_LIMIT,
        )?,
        options,
        parsed,
        CallerMatchMode::Hybrid,
    )?);
    Ok(hits)
}

pub fn anchor_pass_for_files(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    allowed_files: &HashSet<String>,
) -> Result<Vec<SearchHit>> {
    if allowed_files.is_empty() {
        return Ok(Vec::new());
    }
    let anchor_symbol = match parsed.primary_symbol() {
        Some(symbol) => symbol.to_string(),
        None => parsed
            .terms
            .iter()
            .find(|term| term.len() > 3)
            .cloned()
            .unwrap_or_default(),
    };
    if anchor_symbol.is_empty() {
        return Ok(Vec::new());
    }
    let terms = vec![anchor_symbol];
    let (mut where_clause, mut bind) =
        like_terms_filter("s.name", &terms, options.lang_filter.as_deref());
    restrict_to_files(&mut where_clause, &mut bind, Some(allowed_files));
    let rows = query_symbol_spans(store, &where_clause, bind, SYMBOL_SQL_LIMIT)?;
    let term_count = parsed.terms.len();
    symbol_span_rows_to_hits(store, rows, options, HitKind::Anchor, |name| {
        let matched = parsed
            .terms
            .iter()
            .filter(|term| crate::rank::score_symbol(term, name) > 0.0)
            .count();
        if matched == 0 {
            0.0
        } else {
            SCORE_ANCHOR * (matched as f64 / term_count as f64).sqrt()
        }
    })
}

pub fn anchor_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let anchor_symbol = match parsed.primary_symbol() {
        Some(s) => s.to_string(),
        None => parsed
            .terms
            .iter()
            .find(|t| t.len() > 3)
            .cloned()
            .unwrap_or_default(),
    };
    if anchor_symbol.is_empty() {
        return Ok(vec![]);
    }
    let terms = vec![anchor_symbol];
    let (where_clause, bind) = like_terms_filter("s.name", &terms, options.lang_filter.as_deref());
    let rows = query_symbol_spans(store, &where_clause, bind, SYMBOL_SQL_LIMIT)?;
    let term_count = parsed.terms.len();
    symbol_span_rows_to_hits(store, rows, options, HitKind::Anchor, |name| {
        let matched = parsed
            .terms
            .iter()
            .filter(|t| crate::rank::score_symbol(t, name) > 0.0)
            .count();
        if matched == 0 {
            0.0
        } else {
            SCORE_ANCHOR * (matched as f64 / term_count as f64).sqrt()
        }
    })
}
pub(crate) fn def_hits_for_terms(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() {
        return Ok(vec![]);
    }
    let (where_clause, bind) =
        like_terms_filter("s.name", &parsed.terms, options.lang_filter.as_deref());
    let rows = query_symbol_spans(store, &where_clause, bind, limit)?;
    symbol_span_rows_to_hits(store, rows, options, HitKind::Def, |name| {
        score_def(&parsed.terms, name)
    })
}
fn exact_eq_filter(column: &str, value: &str, lang: Option<&str>) -> (String, Vec<String>) {
    use crate::store::sql::{append_lang_filter, where_clause};
    let mut bind = vec![value.to_string()];
    let mut parts = vec![format!("lower({column}) = lower(?)")];
    append_lang_filter(&mut parts, &mut bind, lang);
    (where_clause(&parts), bind)
}
pub(crate) fn caller_hits_for_terms(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() {
        return Ok(vec![]);
    }
    caller_rows_to_hits(
        store,
        query_caller_rows(
            store,
            caller_terms_filter,
            &parsed.terms,
            options.lang_filter.as_deref(),
            None,
            limit,
        )?,
        options,
        parsed,
        CallerMatchMode::Hybrid,
    )
}
fn prefixed_mode_query(parsed: &ParsedQuery) -> Option<ParsedQuery> {
    let symbol = parsed.lookup_symbol();
    (!symbol.is_empty()).then(|| ParsedQuery {
        terms: vec![symbol],
        ..parsed.clone()
    })
}
pub fn search_callers(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let Some(q) = prefixed_mode_query(parsed) else {
        return Ok(vec![]);
    };
    // callers:Name uses equality on indexed callee column.
    let name = q.terms.first().map(String::as_str).unwrap_or("");
    if name.is_empty() {
        return Ok(vec![]);
    }
    let (where_clause, bind) = exact_eq_filter("c.callee", name, options.lang_filter.as_deref());
    let sql = format!("{CALLER_SELECT}{where_clause} LIMIT ?{}", bind.len() + 1);
    let rows = query_limit_map(
        store.connection(),
        &sql,
        bind,
        MODE_SQL_LIMIT,
        map_caller_row,
    )?;
    caller_rows_to_hits_resolved(
        store,
        rows,
        options,
        &q,
        CallerMatchMode::CalleeOnly,
        Some(store),
    )
}
pub fn search_defs(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let Some(q) = prefixed_mode_query(parsed) else {
        return Ok(vec![]);
    };
    // defs:Name is an exact symbol lookup — equality uses the name index.
    let name = q.terms.first().map(String::as_str).unwrap_or("");
    if name.is_empty() {
        return Ok(vec![]);
    }
    let (where_clause, bind) = exact_eq_filter("s.name", name, options.lang_filter.as_deref());
    let rows = query_symbol_spans(store, &where_clause, bind, MODE_SQL_LIMIT)?;
    symbol_span_rows_to_hits(store, rows, options, HitKind::Def, |n| {
        score_def(&q.terms, n)
    })
}
pub fn search_imports(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let module = parsed.lookup_symbol();
    let module = (!module.is_empty()).then_some(module.as_str());
    Ok(store
        .query_imports(module, options.lang_filter.as_deref(), MODE_SQL_LIMIT)?
        .into_iter()
        .map(|(path, language, module_path, line_no)| {
            SearchHit::import(path, language, module_path, line_no)
        })
        .collect())
}

#[cfg(test)]
mod cascade_tests {
    use super::{def_hits_for_terms, symbol_pass_for_files};
    use crate::query::ParsedQuery;
    use crate::search::SearchOptions;
    use crate::store::{IndexStore, SymbolRow, UpsertFileInput};
    use std::collections::HashSet;
    use tempfile::TempDir;

    #[test]
    fn survivor_file_filter_precedes_global_symbol_limit() {
        let temp = TempDir::new().unwrap();
        let store = IndexStore::open(temp.path(), None).unwrap();
        let symbol = SymbolRow {
            name: "target_symbol".into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 1,
            byte_start: 0,
            byte_end: 13,
        };
        for index in 0..=500 {
            let path = if index == 500 {
                "survivor.rs".to_string()
            } else {
                format!("decoy_{index:03}.rs")
            };
            let lines = [(1, "fn target_symbol() {}".to_string())];
            store
                .upsert_file(UpsertFileInput {
                    rel_path: &path,
                    language: Some("rust"),
                    mtime_secs: 1,
                    mtime_nanos: 0,
                    content_hash: &format!("hash-{index}"),
                    lines: &lines,
                    eol: "\n",
                    symbols: std::slice::from_ref(&symbol),
                    callers: &[],
                    imports: &[],
                    pattern_nodes: &[],
                    semantic_chunks: &[],
                    embed_semantic: false,
                    embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
                })
                .unwrap();
        }
        let allowed = HashSet::from(["survivor.rs".to_string()]);
        let hits = symbol_pass_for_files(
            &store,
            &SearchOptions {
                root: temp.path().to_path_buf(),
                ..SearchOptions::default()
            },
            &ParsedQuery::parse("target_symbol"),
            &allowed,
        )
        .unwrap();
        assert!(
            hits.iter().any(|hit| hit.file == "survivor.rs"),
            "survivor after the global SQL ceiling was lost: {hits:#?}"
        );
        assert!(hits.iter().all(|hit| allowed.contains(&hit.file)));
    }

    #[test]
    fn symbol_excerpts_are_read_only_for_retained_candidates() {
        let temp = TempDir::new().unwrap();
        let store = IndexStore::open(temp.path(), None).unwrap();
        for (path, name) in [("discarded.rs", "target_suffix"), ("kept.rs", "target")] {
            let lines = [(1, format!("fn {name}() {{}}"))];
            let symbol = SymbolRow {
                name: name.into(),
                kind: "function".into(),
                line_start: 1,
                line_end: 1,
                byte_start: 0,
                byte_end: lines[0].1.len(),
            };
            store
                .upsert_file(UpsertFileInput {
                    rel_path: path,
                    language: Some("rust"),
                    mtime_secs: 1,
                    mtime_nanos: 0,
                    content_hash: name,
                    lines: &lines,
                    eol: "\n",
                    symbols: &[symbol],
                    callers: &[],
                    imports: &[],
                    pattern_nodes: &[],
                    semantic_chunks: &[],
                    embed_semantic: false,
                    embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
                })
                .unwrap();
        }
        store
            .connection()
            .execute(
                "UPDATE lines SET content = x'ff' WHERE file_id = (SELECT id FROM files WHERE path = 'discarded.rs')",
                [],
            )
            .unwrap();
        let options = SearchOptions {
            root: temp.path().to_path_buf(),
            limit: 1,
            ..SearchOptions::default()
        };
        let parsed = ParsedQuery::parse("target");
        let hits = def_hits_for_terms(&store, &options, &parsed, super::SYMBOL_SQL_LIMIT).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file, "kept.rs");
        assert_eq!(hits[0].excerpt, "fn target() {}");
    }
}
