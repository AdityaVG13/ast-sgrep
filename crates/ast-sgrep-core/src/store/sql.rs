//! Shared SQLite query helpers.

use rusqlite::{params, Connection, ToSql};

use crate::Result;

pub fn optional_row<T, F>(
    conn: &Connection,
    sql: &str,
    bind: &[&dyn ToSql],
    map: F,
) -> Result<Option<T>>
where
    F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    match conn.query_row(sql, bind, map) {
        Ok(value) => Ok(Some(value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn calls_matching(
    conn: &Connection,
    column: &str,
    name: &str,
) -> Result<Vec<(String, u32, String, String)>> {
    let sql = format!(
        "SELECT f.path, c.line_no, c.caller, c.callee
         FROM callers c JOIN files f ON f.id = c.file_id
         WHERE lower(c.{column}) = lower(?1)
         ORDER BY f.path, c.line_no"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![name], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub fn append_lang_filter(parts: &mut Vec<String>, bind: &mut Vec<String>, lang: Option<&str>) {
    if let Some(lang) = lang {
        parts.push("f.language = ?".to_string());
        bind.push(lang.to_string());
    }
}

pub fn where_clause(parts: &[String]) -> String {
    if parts.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", parts.join(" AND "))
    }
}

/// `lower(column) LIKE '%' || lower(?) || '%'` for each term, plus optional language filter.
pub fn like_terms_filter(
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
    append_lang_filter(&mut parts, &mut bind, lang_filter);
    (where_clause(&parts), bind)
}

/// Caller/callee LIKE filter for hybrid symbol search.
pub fn caller_terms_filter(terms: &[String], lang_filter: Option<&str>) -> (String, Vec<String>) {
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
    append_lang_filter(&mut parts, &mut bind, lang_filter);
    (where_clause(&parts), bind)
}

pub fn query_map_rows<T, F>(
    conn: &Connection,
    sql: &str,
    lang: Option<&str>,
    mut map: F,
) -> Result<Vec<T>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut out = Vec::new();
    if let Some(lang) = lang {
        let mut stmt = conn.prepare(sql)?;
        let mut rows = stmt.query(params![lang])?;
        while let Some(row) = rows.next()? {
            out.push(map(row)?);
        }
    } else {
        let mut stmt = conn.prepare(sql)?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            out.push(map(row)?);
        }
    }
    Ok(out)
}
