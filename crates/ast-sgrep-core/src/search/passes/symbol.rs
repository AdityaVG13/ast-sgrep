use crate::query::ParsedQuery;
use crate::rank::{best_symbol_score, score_caller, score_def, SCORE_ANCHOR};
use crate::store::sql::{callee_terms_filter, caller_terms_filter, like_terms_filter, query_limit_map};
use crate::store::IndexStore;
use crate::Result;
use crate::search::hits::matches_lang;
use crate::search::types::{HitKind, SearchHit, SearchOptions};

const SYMBOL_SQL_LIMIT: usize = 500;
const CALLER_SQL_LIMIT: usize = 500;

const SYMBOL_SELECT: &str = "SELECT f.path, f.language, s.name, s.kind, s.line_start, s.line_end, s.byte_start, s.byte_end,
         (SELECT GROUP_CONCAT(l.content, char(10)) FROM lines l
          WHERE l.file_id = f.id AND l.line_no >= s.line_start AND l.line_no <= s.line_end
          ORDER BY l.line_no) AS excerpt
         FROM symbols s
         JOIN files f ON f.id = s.file_id";

const CALLER_SELECT: &str = "SELECT f.path, f.language, c.caller, c.callee, c.line_no, l.content
         FROM callers c
         JOIN files f ON f.id = c.file_id
         JOIN lines l ON l.file_id = c.file_id AND l.line_no = c.line_no";

type CallerQueryRow = (String, Option<String>, String, String, u32, String);

enum CallerMatchMode {
    Hybrid,
    CalleeOnly,
}

fn query_caller_rows(
    store: &IndexStore,
    filter: fn(&[String], Option<&str>) -> (String, Vec<String>),
    terms: &[String],
    lang_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<CallerQueryRow>> {
    let (where_clause, bind) = filter(terms, lang_filter);
    let sql = format!("{CALLER_SELECT}{where_clause} LIMIT ?{}", bind.len() + 1);
    query_limit_map(store.connection(), &sql, bind, limit, |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, u32>(4)?,
            row.get::<_, String>(5)?,
        ))
    })
}

fn caller_rows_to_hits(
    rows: Vec<CallerQueryRow>,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    mode: CallerMatchMode,
) -> Result<Vec<SearchHit>> {
    let primary_lower = parsed
        .primary_symbol()
        .map(|s| s.to_lowercase());
    let mut hits = Vec::new();
    for (path, language, caller, callee, line_no, text) in rows {
        if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
            continue;
        }
        let callee_score = best_symbol_score(&parsed.terms, &callee);
        let caller_score = best_symbol_score(&parsed.terms, &caller);
        let matched = match mode {
            CallerMatchMode::Hybrid => callee_score > 0.0 || caller_score > 0.0,
            CallerMatchMode::CalleeOnly => callee_score > 0.0,
        };
        if !matched {
            continue;
        }
        let score = score_caller(&parsed.terms, &callee);
        let include_graph = match mode {
            CallerMatchMode::CalleeOnly => true,
            CallerMatchMode::Hybrid => {
                callee_score > 0.0
                    || primary_lower
                        .as_ref()
                        .is_some_and(|s| callee.to_lowercase().contains(s))
            }
        };
        hits.push(SearchHit::caller(
            path.clone(),
            language.clone(),
            caller.clone(),
            callee.clone(),
            line_no,
            text,
            score,
        ));
        if include_graph {
            hits.push(SearchHit::graph(path, language, caller, callee, line_no));
        }
    }
    Ok(hits)
}

type SymbolSpanRow = (String, Option<String>, String, u32, u32, String);

fn read_symbol_span_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolSpanRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(4)?,
        row.get(5)?,
        row.get::<_, Option<String>>(8)?.unwrap_or_default(),
    ))
}

fn symbol_span_rows_to_hits(
    rows: Vec<SymbolSpanRow>,
    options: &SearchOptions,
    kind: HitKind,
    score_for: impl Fn(&str) -> f64,
) -> Result<Vec<SearchHit>> {
    let mut hits = Vec::new();
    for (path, language, name, line_start, line_end, text) in rows {
        if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
            continue;
        }
        let score = score_for(&name);
        hits.push(SearchHit::span(
            kind, path, line_start, line_end, score, text, Some(name), language,
        ));
    }
    Ok(hits)
}

pub fn symbol_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let mut hits = Vec::new();
    hits.extend(def_hits_for_terms(
        store,
        options,
        parsed,
        SYMBOL_SQL_LIMIT,
    )?);
    hits.extend(caller_hits_for_terms(
        store,
        options,
        parsed,
        CALLER_SQL_LIMIT,
    )?);
    Ok(hits)
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
        return Ok(Vec::new());
    }

    let terms = vec![anchor_symbol];
    let (where_clause, bind) =
        like_terms_filter("s.name", &terms, options.lang_filter.as_deref());
    let sql = format!("{SYMBOL_SELECT}{where_clause} LIMIT ?{}", bind.len() + 1);
    let rows = query_limit_map(store.connection(), &sql, bind, SYMBOL_SQL_LIMIT, read_symbol_span_row)?;

    symbol_span_rows_to_hits(rows, options, HitKind::Anchor, |_| SCORE_ANCHOR)
}

pub(crate) fn def_hits_for_terms(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() {
        return Ok(Vec::new());
    }

    let (where_clause, bind) = like_terms_filter(
        "s.name",
        &parsed.terms,
        options.lang_filter.as_deref(),
    );
    let sql = format!("{SYMBOL_SELECT}{where_clause} LIMIT ?{}", bind.len() + 1);
    let rows = query_limit_map(store.connection(), &sql, bind, limit, read_symbol_span_row)?;

    symbol_span_rows_to_hits(rows, options, HitKind::Def, |name| {
        score_def(&parsed.terms, name)
    })
}

pub(crate) fn caller_hits_for_terms(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() {
        return Ok(Vec::new());
    }
    let rows = query_caller_rows(
        store,
        caller_terms_filter,
        &parsed.terms,
        options.lang_filter.as_deref(),
        limit,
    )?;
    caller_rows_to_hits(rows, options, parsed, CallerMatchMode::Hybrid)
}

pub(crate) fn callee_hits_for_terms(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() {
        return Ok(Vec::new());
    }
    let rows = query_caller_rows(
        store,
        callee_terms_filter,
        &parsed.terms,
        options.lang_filter.as_deref(),
        limit,
    )?;
    caller_rows_to_hits(rows, options, parsed, CallerMatchMode::CalleeOnly)
}
