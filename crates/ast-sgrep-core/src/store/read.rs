use rusqlite::params;

use super::rows::{read_semantic_chunk_row, SymbolRow};
use super::sql::{calls_matching, optional_row, query_map_rows};
use super::IndexStore;
use crate::{IndexStatus, Result};

impl IndexStore {
    pub fn file_hash(&self, rel_path: &str) -> Result<Option<String>> {
        optional_row(
            &self.conn,
            "SELECT content_hash FROM files WHERE path = ?1",
            &[&rel_path],
            |row| row.get(0),
        )
    }

    pub fn file_mtime(&self, rel_path: &str) -> Result<Option<(i64, u32)>> {
        optional_row(
            &self.conn,
            "SELECT mtime_secs, mtime_nanos FROM files WHERE path = ?1",
            &[&rel_path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    }

    pub fn all_file_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM files ORDER BY path")?;
        let paths = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(paths)
    }

    pub fn status(&self) -> Result<IndexStatus> {
        let (
            file_count,
            line_count,
            symbol_count,
            caller_count,
            import_count,
            semantic_chunk_count,
        ): (usize, usize, usize, usize, usize, usize) = self.conn.query_row(
            "SELECT
                (SELECT COUNT(*) FROM files),
                (SELECT COUNT(*) FROM lines),
                (SELECT COUNT(*) FROM symbols),
                (SELECT COUNT(*) FROM callers),
                (SELECT COUNT(*) FROM imports),
                (SELECT COUNT(*) FROM semantic_chunks)",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )?;
        let embed_backend = self.get_meta("embed_backend")?;
        let embed_dim = self
            .get_meta("embed_dim")?
            .and_then(|d| d.parse().ok());
        let semantic_ivf_present = crate::semantic_ivf::semantic_ivf_path(&self.db_path).exists();

        Ok(IndexStatus {
            root: self.root.display().to_string(),
            index_path: self.db_path.display().to_string(),
            file_count,
            line_count,
            symbol_count,
            caller_count,
            import_count,
            semantic_chunk_count,
            embed_backend,
            embed_dim,
            semantic_ivf_present,
        })
    }

    pub fn all_indexed_lines(&self) -> Result<Vec<(String, u32, String, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.path, l.line_no, l.content, f.language
             FROM lines l JOIN files f ON f.id = l.file_id
             ORDER BY f.path, l.line_no",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?));
        }
        Ok(out)
    }

    pub fn semantic_chunk_max_id(&self) -> Result<Option<i64>> {
        optional_row(
            &self.conn,
            "SELECT MAX(id) FROM semantic_chunks",
            &[],
            |row| row.get(0),
        )
    }

    pub fn all_semantic_chunks(
        &self,
        lang_filter: Option<&str>,
    ) -> Result<Vec<ast_sgrep_embed::SemanticChunkRow>> {
        let lang_clause = if lang_filter.is_some() {
            " AND f.language = ?1"
        } else {
            ""
        };
        let sql = format!(
            "SELECT f.path, sc.line_start, sc.line_end, sc.symbol_name, sc.text, sc.vector
             FROM semantic_chunks sc
             JOIN files f ON f.id = sc.file_id
             WHERE 1=1{lang_clause}
             ORDER BY sc.id"
        );
        query_map_rows(&self.conn, &sql, lang_filter, read_semantic_chunk_row)
    }

    pub fn symbols_in_file(&self, rel_path: &str) -> Result<Vec<SymbolRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.name, s.kind, s.line_start, s.line_end, s.byte_start, s.byte_end
             FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE f.path = ?1 ORDER BY s.line_start",
        )?;
        let rows = stmt.query_map(params![rel_path], |row| {
            Ok(SymbolRow {
                name: row.get(0)?,
                kind: row.get(1)?,
                line_start: row.get(2)?,
                line_end: row.get(3)?,
                byte_start: row.get(4)?,
                byte_end: row.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn incoming_calls(
        &self,
        callee: &str,
    ) -> Result<Vec<(String, u32, String, String)>> {
        calls_matching(&self.conn, "callee", callee)
    }

    pub fn outgoing_calls(
        &self,
        caller: &str,
    ) -> Result<Vec<(String, u32, String, String)>> {
        calls_matching(&self.conn, "caller", caller)
    }

    pub fn file_text(&self, rel_path: &str) -> Result<Option<String>> {
        let lines = self.file_lines(rel_path)?;
        if lines.is_empty() {
            return Ok(None);
        }
        let sep = match self.get_meta(&format!("eol:{rel_path}"))? {
            Some(v) if v == "crlf" => "\r\n",
            _ => "\n",
        };
        Ok(Some(
            lines
                .iter()
                .map(|(_, content)| content.as_str())
                .collect::<Vec<_>>()
                .join(sep),
        ))
    }

    pub fn file_lines(&self, rel_path: &str) -> Result<Vec<(u32, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.line_no, l.content FROM lines l
             JOIN files f ON f.id = l.file_id
             WHERE f.path = ?1 ORDER BY l.line_no",
        )?;
        let rows = stmt.query_map(params![rel_path], |row| {
            Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn line_content(&self, rel_path: &str, line_no: u32) -> Result<Option<String>> {
        optional_row(
            &self.conn,
            "SELECT l.content FROM lines l
             JOIN files f ON f.id = l.file_id
             WHERE f.path = ?1 AND l.line_no = ?2",
            &[&rel_path, &line_no],
            |row| row.get(0),
        )
    }

    pub fn query_imports(
        &self,
        module: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, Option<String>, String, u32)>> {
        if module.is_none_or(|m| m.is_empty()) {
            let mut stmt = self.conn.prepare(
                "SELECT f.path, f.language, i.module_path, i.line_no
                 FROM imports i
                 JOIN files f ON f.id = i.file_id
                 LIMIT ?1",
            )?;
            let rows = stmt.query_map(rusqlite::params![limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?;
            return rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into);
        }

        let module = module.unwrap();
        let mut stmt = self.conn.prepare(
            "SELECT f.path, f.language, i.module_path, i.line_no
             FROM imports i
             JOIN files f ON f.id = i.file_id
             WHERE lower(i.module_path) LIKE '%' || lower(?1) || '%'
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![module, limit as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn excerpt_span(&self, rel_path: &str, line_start: u32, line_end: u32) -> Result<String> {
        let mut stmt = self.conn.prepare(
            "SELECT l.content FROM lines l
             JOIN files f ON f.id = l.file_id
             WHERE f.path = ?1 AND l.line_no >= ?2 AND l.line_no <= ?3
             ORDER BY l.line_no",
        )?;
        let rows = stmt.query_map(params![rel_path, line_start, line_end], |row| {
            row.get::<_, String>(0)
        })?;
        let lines: Vec<String> = rows.collect::<std::result::Result<_, _>>()?;
        Ok(lines.join("\n"))
    }
}
