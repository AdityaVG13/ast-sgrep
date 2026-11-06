use super::super::sql::{
    append_lang_filter, lang_and_clause, like_terms_filter, optional_row, query_cached_map,
    query_limit_map, query_map_rows, read_legacy_emb, where_clause,
};
use super::{ImportQueryRow, IndexStore, IMPORT_SELECT};
use crate::Result;
use rusqlite::{params, ToSql};

impl IndexStore {
    pub fn file_text(&self, path: &str) -> Result<Option<String>> {
        let lines = self.file_lines(path)?;
        if lines.is_empty() {
            return Ok(None);
        }
        let sep = match self.get_meta(&format!("eol:{path}"))? {
            Some(v) if v == "crlf" => "\r\n",
            _ => "\n",
        };
        Ok(Some(
            lines
                .iter()
                .map(|(_, c)| c.as_str())
                .collect::<Vec<_>>()
                .join(sep),
        ))
    }
    pub fn file_lines(&self, path: &str) -> Result<Vec<(u32, String)>> {
        query_cached_map( &self.conn, "SELECT l.line_no, l.content FROM lines l JOIN files f ON f.id=l.file_id WHERE f.path=?1 ORDER BY l.line_no",
            params![path], |r| Ok((r.get(0)?, r.get(1)?)), )
    }
    /// Indexed source rows in an inclusive range, excluding only the global
    /// empty EOF cursor. `None` means no indexed rows; `Some([])` means an
    /// indexed empty/past-EOF window and must not trigger a disk fallback.
    /// One statement keeps EOF discovery and range selection on one snapshot.
    pub fn source_lines_in_range(
        &self,
        path: &str,
        start: u32,
        end: u32,
    ) -> Result<Option<Vec<(u32, String)>>> {
        if let Some(bundle) = self.read_bundle(None)? {
            return Ok(bundle.source_lines_in_range(path, start, end));
        }
        let mut stmt = self.conn.prepare_cached(
            "WITH tail AS (
                SELECT l.file_id, l.line_no - (l.content = '') AS source_end
                FROM lines l JOIN files f ON f.id = l.file_id
                WHERE f.path = ?1 ORDER BY l.line_no DESC LIMIT 1
             )
             SELECT l.line_no, l.content FROM tail
             LEFT JOIN lines l ON l.file_id = tail.file_id
                AND l.line_no >= ?2 AND l.line_no <= MIN(?3, tail.source_end)
             ORDER BY l.line_no",
        )?;
        let mut rows = stmt.query(params![path, start, end])?;
        let mut lines = None;
        while let Some(row) = rows.next()? {
            let selected = lines.get_or_insert_with(Vec::new);
            if let Some(no) = row.get::<_, Option<u32>>(0)? {
                selected.push((no, row.get(1)?));
            }
        }
        Ok(lines)
    }

    /// Last real line number for an indexed path — trailing empty lines
    /// trimmed exactly like `source_lines_in_range`'s `source_end`, floored
    /// at 1 so a line-less file still has a bound. `None` = path not
    /// indexed (caller falls back to a disk count). Backs the shared
    /// reader's past-EOF rejection (MCP `read_node` parity).
    pub fn source_line_total(&self, path: &str) -> Result<Option<u32>> {
        if let Some(bundle) = self.read_bundle(None)? {
            return Ok(bundle.source_line_total(path));
        }
        let mut stmt = self.conn.prepare_cached(
            "WITH tail AS (
                 SELECT l.line_no - (l.content = '') AS source_end
                 FROM lines l JOIN files f ON f.id = l.file_id
                 WHERE f.path = ?1 ORDER BY l.line_no DESC LIMIT 1
             )
             SELECT MAX(CASE WHEN source_end < 1 THEN 1 ELSE source_end END) FROM tail",
        )?;
        let mut rows = stmt.query(params![path])?;
        Ok(match rows.next()? {
            Some(row) => Some(row.get::<_, Option<i64>>(0)?.unwrap_or(1).max(1) as u32),
            None => None,
        })
    }
    pub fn line_content(&self, path: &str, line: u32) -> Result<Option<String>> {
        optional_row(
            &self.conn, "SELECT l.content FROM lines l JOIN files f ON f.id=l.file_id WHERE f.path=?1 AND l.line_no=?2",
            &[&path as &dyn ToSql, &line as &dyn ToSql], |r| r.get(0),
        )
    }
    pub fn query_imports(
        &self,
        module: Option<&str>,
        lang: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ImportQueryRow>> {
        if let Ok(Some(bundle)) = self.read_bundle(None) {
            return Ok(crate::store::bundle_graph::imports_like(
                &bundle, module, lang, limit,
            ));
        }
        let map = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?));
        let Some(m) = module.filter(|m| !m.is_empty()) else {
            let mut parts = Vec::new();
            let mut bind = Vec::new();
            append_lang_filter(&mut parts, &mut bind, lang);
            let w = where_clause(&parts);
            return query_limit_map(
                &self.conn,
                &format!("{IMPORT_SELECT}{w} LIMIT ?{}", bind.len() + 1),
                bind,
                limit,
                map,
            );
        };
        let (w, bind) = like_terms_filter("i.module_path", &[m.to_string()], lang);
        query_limit_map(
            &self.conn,
            &format!("{IMPORT_SELECT}{w} LIMIT ?{}", bind.len() + 1),
            bind,
            limit,
            map,
        )
    }
    pub fn all_legacy_embeddings(
        &self,
        lang: Option<&str>,
    ) -> Result<Vec<ast_sgrep_embed::SemanticChunkRow>> {
        let sql = format!(
            "SELECT f.path, l.line_no, l.content, sc.symbol_name, e.vector FROM embeddings e \
             JOIN lines l ON l.file_id=e.file_id AND l.line_no=e.line_no JOIN files f ON f.id=e.file_id \
             LEFT JOIN semantic_chunks sc ON sc.file_id=f.id AND sc.line_start=l.line_no WHERE 1=1{} LIMIT 5000",
            lang_and_clause(lang)
        );
        query_map_rows(&self.conn, &sql, lang, read_legacy_emb)
    }
    pub fn file_exists(&self, path: &str) -> Result<bool> {
        Ok(self
            .conn
            .prepare_cached("SELECT 1 FROM files WHERE path=?1")?
            .exists(params![path])?)
    }
}
