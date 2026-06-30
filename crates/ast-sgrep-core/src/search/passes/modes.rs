use rusqlite::params;

use crate::query::ParsedQuery;
use crate::rank::{score_caller, score_def};
use crate::store::IndexStore;
use crate::Result;
use crate::search::hits::{import_hit, push_caller_and_graph};
use crate::search::types::{HitKind, SearchHit};

const MODE_SQL_LIMIT: usize = 200;

pub fn search_callers(
    store: &IndexStore,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
) -> Result<Vec<SearchHit>> {
    let symbol = parsed.lookup_symbol();
    let conn = store.connection();
    let mut stmt = conn.prepare(
        "SELECT f.path, f.language, c.caller, c.callee, c.line_no, c.byte_start, c.byte_end
         FROM callers c
         JOIN files f ON f.id = c.file_id
         WHERE lower(c.callee) = lower(?1) OR lower(c.callee) LIKE '%' || lower(?1) || '%'
         LIMIT ?2",
    )?;

    let rows = stmt.query_map(params![symbol, MODE_SQL_LIMIT as i64], |row| {
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
        let text = excerpt(&path, line_no, line_no)?;
        push_caller_and_graph(
            &mut hits,
            path,
            language,
            caller,
            callee.clone(),
            line_no,
            text,
            score_caller(&parsed.terms, &callee),
            true,
        );
    }
    Ok(hits)
}

pub fn search_defs(
    store: &IndexStore,
    parsed: &ParsedQuery,
    excerpt: &dyn Fn(&str, u32, u32) -> Result<String>,
) -> Result<Vec<SearchHit>> {
    let symbol = parsed.lookup_symbol();
    let conn = store.connection();
    let mut stmt = conn.prepare(
        "SELECT f.path, f.language, s.name, s.line_start, s.line_end, s.byte_start, s.byte_end
         FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE lower(s.name) = lower(?1) OR lower(s.name) LIKE '%' || lower(?1) || '%'
         LIMIT ?2",
    )?;

    let rows = stmt.query_map(params![symbol, MODE_SQL_LIMIT as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, u32>(3)?,
            row.get::<_, u32>(4)?,
        ))
    })?;

    let mut hits = Vec::new();
    for row in rows {
        let (path, language, name, line_start, line_end) = row?;
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
        let mut rows = stmt.query(params![MODE_SQL_LIMIT as i64])?;
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
        let mut rows = stmt.query(params![module, MODE_SQL_LIMIT as i64])?;
        while let Some(row) = rows.next()? {
            hits.push(read_import_row(&row)?);
        }
    }

    Ok(hits)
}

fn read_import_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SearchHit> {
    let path: String = row.get(0)?;
    let language: Option<String> = row.get(1)?;
    let module_path: String = row.get(2)?;
    let line_no: u32 = row.get(3)?;
    Ok(import_hit(path, language, module_path, line_no))
}
