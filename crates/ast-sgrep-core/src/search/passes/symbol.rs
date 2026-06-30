use crate::query::ParsedQuery;
use crate::rank::{best_symbol_score, score_caller, score_def, SCORE_ANCHOR};
use crate::store::sql::{callee_terms_filter, caller_terms_filter, like_terms_filter, query_limit_map};
use crate::store::IndexStore;
use crate::Result;
use crate::search::hits::{matches_lang, push_caller_and_graph};
use crate::search::types::{HitKind, SearchHit, SearchOptions};

const SYMBOL_SQL_LIMIT: usize = 500;
const CALLER_SQL_LIMIT: usize = 500;

const SYMBOL_SELECT: &str = "SELECT f.path, f.language, s.name, s.kind, s.line_start, s.line_end, s.byte_start, s.byte_end
         FROM symbols s
         JOIN files f ON f.id = s.file_id";

const CALLER_SELECT: &str = "SELECT f.path, f.language, c.caller, c.callee, c.line_no, c.byte_start, c.byte_end
         FROM callers c
         JOIN files f ON f.id = c.file_id";

type CallerQueryRow = (String, Option<String>, String, String, u32);

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
        ))
    })
}

fn caller_rows_to_hits(
    rows: Vec<CallerQueryRow>,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
    mode: CallerMatchMode,
) -> Result<Vec<SearchHit>> {
    let mut hits = Vec::new();
    for (path, language, caller, callee, line_no) in rows {
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
        let text = excerpt(&path, line_no, line_no)?;
        let score = score_caller(&parsed.terms, &callee);
        let include_graph = match mode {
            CallerMatchMode::CalleeOnly => true,
            CallerMatchMode::Hybrid => {
                callee_score > 0.0
                    || parsed
                        .primary_symbol()
                        .is_some_and(|s| callee.to_lowercase().contains(&s.to_lowercase()))
            }
        };
        push_caller_and_graph(
            &mut hits,
            path,
            language,
            caller,
            callee,
            line_no,
            text,
            score,
            include_graph,
        );
    }
    Ok(hits)
}

pub fn symbol_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
) -> Result<Vec<SearchHit>> {
    let mut hits = Vec::new();
    hits.extend(def_hits_for_terms(
        store,
        options,
        parsed,
        excerpt,
        SYMBOL_SQL_LIMIT,
    )?);
    hits.extend(caller_hits_for_terms(
        store,
        options,
        parsed,
        excerpt,
        CALLER_SQL_LIMIT,
    )?);
    Ok(hits)
}

pub fn anchor_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
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
    let rows = query_limit_map(store.connection(), &sql, bind, SYMBOL_SQL_LIMIT, |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, u32>(4)?,
            row.get::<_, u32>(5)?,
        ))
    })?;

    let mut hits = Vec::new();
    for (path, language, name, line_start, line_end) in rows {
        if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
            continue;
        }
        let text = excerpt(&path, line_start, line_end)?;
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
            excerpt: text,
        });
    }
    Ok(hits)
}

pub(crate) fn def_hits_for_terms(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
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
    let rows = query_limit_map(store.connection(), &sql, bind, limit, |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, u32>(4)?,
            row.get::<_, u32>(5)?,
        ))
    })?;

    let mut hits = Vec::new();
    for (path, language, name, line_start, line_end) in rows {
        if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
            continue;
        }
        let text = excerpt(&path, line_start, line_end)?;
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
            excerpt: text,
        });
    }
    Ok(hits)
}

pub(crate) fn caller_hits_for_terms(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
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
    caller_rows_to_hits(rows, options, parsed, excerpt, CallerMatchMode::Hybrid)
}

pub(crate) fn callee_hits_for_terms(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
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
    caller_rows_to_hits(rows, options, parsed, excerpt, CallerMatchMode::CalleeOnly)
}
