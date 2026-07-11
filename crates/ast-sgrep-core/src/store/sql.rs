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
    let mut stmt = conn.prepare_cached(sql)?;
    match stmt.query_row(bind, map) {
        Ok(v) => Ok(Some(v)),
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
        "SELECT f.path, c.line_no, c.caller, c.callee FROM callers c JOIN files f ON f.id = c.file_id
         WHERE lower(c.{column}) = lower(?1) ORDER BY f.path, c.line_no"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params![name], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}
pub fn append_lang_filter(parts: &mut Vec<String>, bind: &mut Vec<String>, lang: Option<&str>) {
    if let Some(lang) = lang {
        parts.push("f.language = ?".into());
        bind.push(lang.into());
    }
}
pub fn where_clause(parts: &[String]) -> String {
    if parts.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", parts.join(" AND "))
    }
}
fn escape_like_term(term: &str) -> String {
    let mut out = String::with_capacity(term.len());
    for ch in term.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '%' => out.push_str("\\%"),
            '_' => out.push_str("\\_"),
            other => out.push(other),
        }
    }
    out
}
pub fn like_terms_filter(
    column: &str,
    terms: &[String],
    lang_filter: Option<&str>,
) -> (String, Vec<String>) {
    let mut bind: Vec<String> = terms.iter().map(|t| escape_like_term(t)).collect();
    let mut parts = Vec::new();
    if !terms.is_empty() {
        let conds: Vec<String> = terms
            .iter()
            .map(|_| format!("lower({column}) LIKE '%' || lower(?) || '%' ESCAPE '\\'"))
            .collect();
        parts.push(format!("({})", conds.join(" OR ")));
    }
    append_lang_filter(&mut parts, &mut bind, lang_filter);
    (where_clause(&parts), bind)
}
pub fn callee_terms_filter(terms: &[String], lang_filter: Option<&str>) -> (String, Vec<String>) {
    like_terms_filter("c.callee", terms, lang_filter)
}
pub fn caller_terms_filter(terms: &[String], lang_filter: Option<&str>) -> (String, Vec<String>) {
    let escaped: Vec<String> = terms.iter().map(|t| escape_like_term(t)).collect();
    let mut bind = Vec::new();
    let mut parts = Vec::new();
    if !terms.is_empty() {
        let conds: Vec<String> = terms
            .iter()
            .map(|_| {
                "(lower(c.callee) LIKE '%' || lower(?) || '%' ESCAPE '\\' OR lower(c.caller) LIKE '%' || lower(?) || '%' ESCAPE '\\')"
                    .to_string()
            })
            .collect();
        for t in &escaped {
            bind.push(t.clone());
            bind.push(t.clone());
        }
        parts.push(format!("({})", conds.join(" OR ")));
    }
    append_lang_filter(&mut parts, &mut bind, lang_filter);
    (where_clause(&parts), bind)
}
fn collect_mapped_rows<T, F>(mut rows: rusqlite::Rows<'_>, mut map: F) -> Result<Vec<T>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(map(row)?);
    }
    Ok(out)
}
pub fn query_map_rows<T, F>(
    conn: &Connection,
    sql: &str,
    lang: Option<&str>,
    map: F,
) -> Result<Vec<T>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut stmt = conn.prepare_cached(sql)?;
    if let Some(lang) = lang {
        collect_mapped_rows(stmt.query(params![lang])?, map)
    } else {
        collect_mapped_rows(stmt.query([])?, map)
    }
}
pub fn query_limit_map<T, F>(
    conn: &Connection,
    sql: &str,
    bind: Vec<String>,
    limit: usize,
    mut map: F,
) -> Result<Vec<T>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut stmt = conn.prepare_cached(sql)?;
    let mut params_vec: Vec<Box<dyn ToSql>> = bind.into_iter().map(|s| Box::new(s) as _).collect();
    params_vec.push(Box::new(limit as i64));
    let refs: Vec<&dyn ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(refs.as_slice(), |row| map(row))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}
pub fn query_cached_map<T, P, F>(conn: &Connection, sql: &str, params: P, map: F) -> Result<Vec<T>>
where
    P: rusqlite::Params,
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(params, map)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}
pub fn count_star(conn: &Connection, table: &str) -> Result<usize> {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get::<_, i64>(0)
    })
    .map(|n| n as usize)
    .map_err(Into::into)
}
