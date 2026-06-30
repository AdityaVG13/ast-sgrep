use crate::query::ParsedQuery;
use crate::store::IndexStore;
use crate::Result;
use crate::search::hits::import_hit;
use crate::search::types::{SearchHit, SearchOptions};

use super::symbol::{caller_hits_for_terms, def_hits_for_terms};

const MODE_SQL_LIMIT: usize = 200;

pub fn search_callers(
    store: &IndexStore,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
) -> Result<Vec<SearchHit>> {
    let symbol = parsed.lookup_symbol();
    if symbol.is_empty() {
        return Ok(Vec::new());
    }
    let mode_query = ParsedQuery {
        terms: vec![symbol],
        ..parsed.clone()
    };
    caller_hits_for_terms(
        store,
        &mode_search_options(),
        &mode_query,
        excerpt,
        MODE_SQL_LIMIT,
    )
}

pub fn search_defs(
    store: &IndexStore,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
) -> Result<Vec<SearchHit>> {
    let symbol = parsed.lookup_symbol();
    if symbol.is_empty() {
        return Ok(Vec::new());
    }
    let mode_query = ParsedQuery {
        terms: vec![symbol],
        ..parsed.clone()
    };
    def_hits_for_terms(
        store,
        &mode_search_options(),
        &mode_query,
        excerpt,
        MODE_SQL_LIMIT,
    )
}

pub fn search_imports(store: &IndexStore, parsed: &ParsedQuery) -> Result<Vec<SearchHit>> {
    let module = parsed.lookup_symbol();
    let conn = store.connection();
    let mut hits = Vec::new();

    if module.is_empty() {
        let mut stmt = conn.prepare(
            "SELECT f.path, f.language, i.module_path, i.line_no
             FROM imports i
             JOIN files f ON f.id = i.file_id
             LIMIT ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![MODE_SQL_LIMIT as i64])?;
        while let Some(row) = rows.next()? {
            hits.push(read_import_row(&row)?);
        }
    } else {
        let mut stmt = conn.prepare(
            "SELECT f.path, f.language, i.module_path, i.line_no
             FROM imports i
             JOIN files f ON f.id = i.file_id
             WHERE lower(i.module_path) LIKE '%' || lower(?1) || '%'
             LIMIT ?2",
        )?;
        let mut rows = stmt.query(rusqlite::params![module, MODE_SQL_LIMIT as i64])?;
        while let Some(row) = rows.next()? {
            hits.push(read_import_row(&row)?);
        }
    }

    Ok(hits)
}

fn mode_search_options() -> SearchOptions {
    SearchOptions {
        lang_filter: None,
        ..SearchOptions::default()
    }
}

fn read_import_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SearchHit> {
    let path: String = row.get(0)?;
    let language: Option<String> = row.get(1)?;
    let module_path: String = row.get(2)?;
    let line_no: u32 = row.get(3)?;
    Ok(import_hit(path, language, module_path, line_no))
}
