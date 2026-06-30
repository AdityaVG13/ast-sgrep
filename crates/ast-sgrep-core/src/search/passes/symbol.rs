use rusqlite::params;

use crate::query::ParsedQuery;
use crate::rank::{best_symbol_score, score_caller, score_def, SCORE_ANCHOR};
use crate::store::IndexStore;
use crate::Result;
use crate::search::hits::push_caller_and_graph;
use crate::search::types::{HitKind, SearchHit, SearchOptions};

const SYMBOL_SQL_LIMIT: usize = 500;
const CALLER_SQL_LIMIT: usize = 500;

pub fn symbol_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
) -> Result<Vec<SearchHit>> {
    let mut hits = Vec::new();
    hits.extend(def_hits_for_terms(store, options, parsed, excerpt)?);
    hits.extend(caller_hits_for_terms(store, options, parsed, excerpt)?);
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

    let conn = store.connection();
    let mut stmt = conn.prepare(
        "SELECT f.path, f.language, s.name, s.line_start, s.line_end, s.byte_start, s.byte_end
         FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE lower(s.name) = lower(?1) OR lower(s.name) LIKE '%' || lower(?1) || '%'
         LIMIT ?2",
    )?;

    let rows = stmt.query_map(params![anchor_symbol, SYMBOL_SQL_LIMIT as i64], |row| {
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
        if let Some(ref lang_filter) = options.lang_filter {
            if language.as_deref() != Some(lang_filter.as_str()) {
                continue;
            }
        }
        let text = excerpt(&path, line_start, line_end)?;
        let _ = (byte_start, byte_end);
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

fn def_hits_for_terms(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() {
        return Ok(Vec::new());
    }

    let (sql, bind): (String, Vec<String>) = build_term_filter_sql(
        "SELECT f.path, f.language, s.name, s.kind, s.line_start, s.line_end, s.byte_start, s.byte_end
         FROM symbols s
         JOIN files f ON f.id = s.file_id",
        "s.name",
        &parsed.terms,
        options.lang_filter.as_deref(),
    );

    let conn = store.connection();
    let mut stmt = conn.prepare(&format!("{sql} LIMIT ?{}", bind.len() + 1))?;
    let mut hits = Vec::new();
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
        bind.into_iter().map(|s| Box::new(s) as _).collect();
    params_vec.push(Box::new(SYMBOL_SQL_LIMIT as i64));
    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();

    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, u32>(4)?,
            row.get::<_, u32>(5)?,
            row.get::<_, usize>(6)?,
            row.get::<_, usize>(7)?,
        ))
    })?;

    for row in rows {
        let (path, language, name, line_start, line_end, _byte_start, _byte_end) = row?;
        if let Some(ref lang_filter) = options.lang_filter {
            if language.as_deref() != Some(lang_filter.as_str()) {
                continue;
            }
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

fn caller_hits_for_terms(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() {
        return Ok(Vec::new());
    }

    let (sql, bind): (String, Vec<String>) = build_caller_filter_sql(
        "SELECT f.path, f.language, c.caller, c.callee, c.line_no, c.byte_start, c.byte_end
         FROM callers c
         JOIN files f ON f.id = c.file_id",
        &parsed.terms,
        options.lang_filter.as_deref(),
    );

    let conn = store.connection();
    let mut stmt = conn.prepare(&format!("{sql} LIMIT ?{}", bind.len() + 1))?;
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
        bind.into_iter().map(|s| Box::new(s) as _).collect();
    params_vec.push(Box::new(CALLER_SQL_LIMIT as i64));
    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();

    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, u32>(4)?,
        ))
    })?;

    let mut hits = Vec::new();
    for row in rows {
        let (path, language, caller, callee, line_no) = row?;
        if let Some(ref lang_filter) = options.lang_filter {
            if language.as_deref() != Some(lang_filter.as_str()) {
                continue;
            }
        }
        let callee_score = best_symbol_score(&parsed.terms, &callee);
        let caller_score = best_symbol_score(&parsed.terms, &caller);
        if callee_score == 0.0 && caller_score == 0.0 {
            continue;
        }
        let text = excerpt(&path, line_no, line_no)?;
        let include_graph = callee_score > 0.0
            || parsed
                .primary_symbol()
                .is_some_and(|s| callee.to_lowercase().contains(&s.to_lowercase()));
        push_caller_and_graph(
            &mut hits,
            path,
            language,
            caller,
            callee.clone(),
            line_no,
            text,
            score_caller(&parsed.terms, &callee),
            include_graph,
        );
    }
    Ok(hits)
}

fn build_term_filter_sql(
    base: &str,
    column: &str,
    terms: &[String],
    lang_filter: Option<&str>,
) -> (String, Vec<String>) {
    let mut bind: Vec<String> = terms.to_vec();
    let mut parts: Vec<String> = Vec::new();
    if !terms.is_empty() {
        let term_conds: Vec<String> = terms
            .iter()
            .map(|_| format!("lower({column}) LIKE '%' || lower(?) || '%'"))
            .collect();
        parts.push(format!("({})", term_conds.join(" OR ")));
    }
    if let Some(lang) = lang_filter {
        parts.push("f.language = ?".to_string());
        bind.push(lang.to_string());
    }
    let where_clause = if parts.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", parts.join(" AND "))
    };
    (format!("{base}{where_clause}"), bind)
}

fn build_caller_filter_sql(
    base: &str,
    terms: &[String],
    lang_filter: Option<&str>,
) -> (String, Vec<String>) {
    let mut bind: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    if !terms.is_empty() {
        let term_conds: Vec<String> = terms
            .iter()
            .map(|_| {
                "(lower(c.callee) LIKE '%' || lower(?) || '%' OR lower(c.caller) LIKE '%' || lower(?) || '%')".to_string()
            })
            .collect();
        for term in terms {
            bind.push(term.clone());
            bind.push(term.clone());
        }
        parts.push(format!("({})", term_conds.join(" OR ")));
    }
    if let Some(lang) = lang_filter {
        parts.push("f.language = ?".to_string());
        bind.push(lang.to_string());
    }
    let where_clause = if parts.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", parts.join(" AND "))
    };
    (format!("{base}{where_clause}"), bind)
}
