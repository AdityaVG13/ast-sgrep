use crate::Result;
use rusqlite::{params, Connection, ToSql};
use std::time::Duration;
// Full DDL for the current schema; init_schema applies when user_version is lower.
pub(crate) const SCHEMA_DDL: &str = "\
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);\
CREATE TABLE IF NOT EXISTS files (id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, language TEXT,\
  mtime_secs INTEGER NOT NULL, mtime_nanos INTEGER NOT NULL, content_hash TEXT NOT NULL);\
CREATE TABLE IF NOT EXISTS lines (file_id INTEGER NOT NULL, line_no INTEGER NOT NULL, content TEXT NOT NULL,\
  PRIMARY KEY (file_id, line_no), FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE);\
CREATE TABLE IF NOT EXISTS symbols (id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL, name TEXT NOT NULL,\
  kind TEXT NOT NULL, line_start INTEGER NOT NULL, line_end INTEGER NOT NULL,\
  byte_start INTEGER NOT NULL, byte_end INTEGER NOT NULL,\
  FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE);\
CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);\
CREATE INDEX IF NOT EXISTS idx_symbols_file_id ON symbols(file_id);\
CREATE TABLE IF NOT EXISTS callers (id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL, caller TEXT NOT NULL,\
  callee TEXT NOT NULL, line_no INTEGER NOT NULL, byte_start INTEGER NOT NULL, byte_end INTEGER NOT NULL,\
  FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE);\
CREATE INDEX IF NOT EXISTS idx_callers_callee ON callers(callee);\
CREATE INDEX IF NOT EXISTS idx_callers_caller ON callers(caller);\
CREATE INDEX IF NOT EXISTS idx_callers_file_id ON callers(file_id);\
CREATE TABLE IF NOT EXISTS imports (id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL,\
  module_path TEXT NOT NULL, line_no INTEGER NOT NULL,\
  FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE);\
CREATE INDEX IF NOT EXISTS idx_imports_module ON imports(module_path);\
CREATE INDEX IF NOT EXISTS idx_imports_file_id ON imports(file_id);\
CREATE TABLE IF NOT EXISTS pattern_nodes (id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL, signature TEXT NOT NULL,\
  line_start INTEGER NOT NULL, line_end INTEGER NOT NULL, excerpt TEXT NOT NULL,\
  FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE);\
CREATE INDEX IF NOT EXISTS idx_pattern_nodes_signature ON pattern_nodes(signature);\
CREATE INDEX IF NOT EXISTS idx_pattern_nodes_file ON pattern_nodes(file_id);\
CREATE VIRTUAL TABLE IF NOT EXISTS lines_fts USING fts5(content, file_id UNINDEXED, line_no UNINDEXED, tokenize = 'porter unicode61');\
CREATE VIRTUAL TABLE IF NOT EXISTS lines_trigram USING fts5(content, content = 'lines', content_rowid = 'rowid', tokenize = 'trigram');\
CREATE TABLE IF NOT EXISTS embeddings (file_id INTEGER NOT NULL, line_no INTEGER NOT NULL, vector BLOB NOT NULL,\
  PRIMARY KEY (file_id, line_no), FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE);\
CREATE TABLE IF NOT EXISTS semantic_chunks (id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL, symbol_id INTEGER,\
  chunk_kind TEXT NOT NULL, line_start INTEGER NOT NULL, line_end INTEGER NOT NULL, symbol_name TEXT,\
  text TEXT NOT NULL, vector BLOB NOT NULL,\
  FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE,\
  FOREIGN KEY (symbol_id) REFERENCES symbols(id) ON DELETE CASCADE);\
CREATE INDEX IF NOT EXISTS idx_semantic_chunks_symbol ON semantic_chunks(symbol_name);\
CREATE INDEX IF NOT EXISTS idx_semantic_chunks_file_id ON semantic_chunks(file_id);\
CREATE TABLE IF NOT EXISTS embed_cache (chunk_hash TEXT NOT NULL, model_id TEXT NOT NULL, backend TEXT NOT NULL,\
  dim INTEGER NOT NULL, vector BLOB NOT NULL, accessed_at INTEGER NOT NULL, PRIMARY KEY (chunk_hash, model_id));\
CREATE INDEX IF NOT EXISTS idx_embed_cache_accessed ON embed_cache(accessed_at);";
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
    let sql = format!( "SELECT f.path, c.line_no, c.caller, c.callee FROM callers c JOIN files f ON f.id = c.file_id
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
/// OR of `lower(col) LIKE %term%` over `columns` × `terms`, plus optional lang filter.
fn or_like_filter(
    columns: &[&str],
    terms: &[String],
    lang_filter: Option<&str>,
) -> (String, Vec<String>) {
    let mut bind = Vec::new();
    let mut parts = Vec::new();
    if !terms.is_empty() && !columns.is_empty() {
        let conds: Vec<String> = terms
            .iter()
            .map(|_| {
                let per_col: Vec<String> = columns
                    .iter()
                    .map(|c| format!("lower({c}) LIKE '%' || lower(?) || '%' ESCAPE '\\'"))
                    .collect();
                if per_col.len() == 1 {
                    per_col.into_iter().next().unwrap()
                } else {
                    format!("({})", per_col.join(" OR "))
                }
            })
            .collect();
        for t in terms {
            let esc = escape_like_term(t);
            for _ in columns {
                bind.push(esc.clone());
            }
        }
        parts.push(format!("({})", conds.join(" OR ")));
    }
    append_lang_filter(&mut parts, &mut bind, lang_filter);
    (where_clause(&parts), bind)
}
pub fn like_terms_filter(
    column: &str,
    terms: &[String],
    lang_filter: Option<&str>,
) -> (String, Vec<String>) {
    or_like_filter(&[column], terms, lang_filter)
}
pub fn caller_terms_filter(terms: &[String], lang_filter: Option<&str>) -> (String, Vec<String>) {
    or_like_filter(&["c.callee", "c.caller"], terms, lang_filter)
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
    match lang {
        Some(lang) => query_cached_map(conn, sql, params![lang], map),
        None => query_cached_map(conn, sql, [], map),
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
/// Fast existence-style probe: true when table has ≥ `threshold` rows (LIMIT probe, not full COUNT).
pub fn at_least_rows(conn: &Connection, table: &str, threshold: usize) -> Result<bool> {
    if threshold == 0 {
        return Ok(true);
    }
    let n: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM (SELECT 1 FROM {table} LIMIT {threshold})"),
        [],
        |row| row.get(0),
    )?;
    Ok(n as usize >= threshold)
}
/// Delete lines_fts + content-sync trigram + lines for one file.
/// When `from_line` is `Some(n)`, only rows with `line_no >= n` are removed (truncate path).
pub fn delete_file_lines(conn: &Connection, file_id: i64, from_line: Option<u32>) -> Result<()> {
    match from_line {
        Some(first) => {
            conn.prepare_cached(
                "DELETE FROM lines_trigram WHERE rowid IN \
                 (SELECT rowid FROM lines WHERE file_id = ?1 AND line_no >= ?2)",
            )?
            .execute(params![file_id, first])?;
            conn.prepare_cached("DELETE FROM lines_fts WHERE file_id = ?1 AND line_no >= ?2")?
                .execute(params![file_id, first])?;
            conn.prepare_cached("DELETE FROM lines WHERE file_id = ?1 AND line_no >= ?2")?
                .execute(params![file_id, first])?;
        }
        None => {
            conn.prepare_cached("DELETE FROM lines_fts WHERE file_id = ?1")?
                .execute(params![file_id])?;
            conn.prepare_cached(
                "DELETE FROM lines_trigram WHERE rowid IN (SELECT rowid FROM lines WHERE file_id = ?1)",
            )?
            .execute(params![file_id])?;
            conn.prepare_cached("DELETE FROM lines WHERE file_id = ?1")?
                .execute(params![file_id])?;
        }
    }
    Ok(())
}
/// Delete per-file child rows (lines/FTS/trigram first, then relational tables).
pub fn delete_file_children(conn: &Connection, file_id: i64) -> Result<()> {
    delete_file_lines(conn, file_id, None)?;
    for t in [
        "symbols",
        "callers",
        "imports",
        "pattern_nodes",
        "embeddings",
        "semantic_chunks",
    ] {
        conn.prepare_cached(&format!("DELETE FROM {t} WHERE file_id=?1"))?
            .execute(params![file_id])?;
    }
    Ok(())
}
/// Full wipe of index content tables (schema left intact). Order keeps FTS/content-sync safe.
pub const CLEAR_ALL_SQL: &str = "\
DELETE FROM lines_trigram; DELETE FROM lines_fts; DELETE FROM semantic_chunks; \
DELETE FROM pattern_nodes; DELETE FROM embeddings; DELETE FROM imports; \
DELETE FROM callers; DELETE FROM symbols; DELETE FROM lines; DELETE FROM files;";
fn emb_vec(r: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Vec<f32>> {
    let v: Vec<u8> = r.get(idx)?;
    Ok(ast_sgrep_embed::embed_from_bytes(&v).unwrap_or_default())
}
pub fn read_legacy_emb(
    r: &rusqlite::Row<'_>,
) -> rusqlite::Result<ast_sgrep_embed::SemanticChunkRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(1)?,
        r.get::<_, Option<String>>(3)?.unwrap_or_default(),
        r.get(2)?,
        emb_vec(r, 4)?,
    ))
}
pub fn read_sem_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ast_sgrep_embed::SemanticChunkRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get::<_, Option<String>>(3)?.unwrap_or_default(),
        r.get(4)?,
        emb_vec(r, 5)?,
    ))
}
/// ` AND f.language = ?1` when a language bind is present.
pub fn lang_and_clause(lang: Option<&str>) -> &'static str {
    if lang.is_some() {
        " AND f.language = ?1"
    } else {
        ""
    }
}
pub fn configure_connection(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.set_prepared_statement_cache_capacity(128);
    let journal_mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    }
    conn.execute_batch(
        "PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL; PRAGMA wal_autocheckpoint = 1000;",
    )?;
    if std::env::var_os("ASGREP_SQLITE_DEFAULTS").is_none() {
        conn.execute_batch("PRAGMA mmap_size = 268435456; PRAGMA cache_size = -16384;")?;
    }
    Ok(())
}
pub fn integrity_check(conn: &Connection) -> Result<String> {
    conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(Into::into)
}
