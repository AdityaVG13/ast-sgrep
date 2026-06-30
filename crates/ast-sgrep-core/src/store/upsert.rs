use std::collections::HashMap;

use rusqlite::params;

use super::rows::{CallerRow, ImportRow, SymbolRow};
use super::schema::delete_file_children;
use super::sql::optional_row;
use super::IndexStore;
use crate::Result;

impl IndexStore {
    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        optional_row(&self.conn, "SELECT value FROM meta WHERE key = ?1", &[&key], |row| {
            row.get(0)
        })
    }

    pub fn delete_meta(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM meta WHERE key = ?1", params![key])?;
        Ok(())
    }

    pub fn begin_file_tx(&self) -> Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        Ok(())
    }

    pub fn commit_file_tx(&self) -> Result<()> {
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }

    pub fn rollback_file_tx(&self) -> Result<()> {
        let _ = self.conn.execute_batch("ROLLBACK");
        Ok(())
    }

    pub fn upsert_file(
        &self,
        rel_path: &str,
        language: Option<&str>,
        mtime_secs: i64,
        mtime_nanos: u32,
        content_hash: &str,
        lines: &[(u32, String)],
        eol: &str,
        symbols: &[SymbolRow],
        callers: &[CallerRow],
        imports: &[ImportRow],
        semantic_chunks: &[crate::semantic_chunk::SemanticChunkInput],
        embed_semantic: bool,
        embed_backend: ast_sgrep_embed::EmbedPreference,
    ) -> Result<i64> {
        self.begin_file_tx()?;
        let result = self.upsert_file_inner(
            rel_path,
            language,
            mtime_secs,
            mtime_nanos,
            content_hash,
            lines,
            eol,
            symbols,
            callers,
            imports,
            semantic_chunks,
            embed_semantic,
            embed_backend,
        );
        match result {
            Ok(file_id) => {
                self.commit_file_tx()?;
                Ok(file_id)
            }
            Err(e) => {
                self.rollback_file_tx()?;
                Err(e)
            }
        }
    }

    fn upsert_file_inner(
        &self,
        rel_path: &str,
        language: Option<&str>,
        mtime_secs: i64,
        mtime_nanos: u32,
        content_hash: &str,
        lines: &[(u32, String)],
        eol: &str,
        symbols: &[SymbolRow],
        callers: &[CallerRow],
        imports: &[ImportRow],
        semantic_chunks: &[crate::semantic_chunk::SemanticChunkInput],
        embed_semantic: bool,
        embed_backend: ast_sgrep_embed::EmbedPreference,
    ) -> Result<i64> {
        let file_id: Option<i64> = self.conn.query_row(
            "SELECT id FROM files WHERE path = ?1",
            params![rel_path],
            |row| row.get(0),
        ).ok();

        let file_id = if let Some(id) = file_id {
            delete_file_children(&self.conn, id)?;
            self.conn.execute(
                "UPDATE files SET language = ?1, mtime_secs = ?2, mtime_nanos = ?3, content_hash = ?4 WHERE id = ?5",
                params![language, mtime_secs, mtime_nanos, content_hash, id],
            )?;
            id
        } else {
            self.conn.execute(
                "INSERT INTO files(path, language, mtime_secs, mtime_nanos, content_hash)
                 VALUES(?1, ?2, ?3, ?4, ?5)",
                params![rel_path, language, mtime_secs, mtime_nanos, content_hash],
            )?;
            self.conn.last_insert_rowid()
        };

        {
            let mut line_stmt = self.conn.prepare(
                "INSERT INTO lines(file_id, line_no, content) VALUES(?1, ?2, ?3)",
            )?;
            let mut fts_stmt = self.conn.prepare(
                "INSERT INTO lines_fts(content, file_id, line_no) VALUES(?1, ?2, ?3)",
            )?;
            for (line_no, content) in lines {
                line_stmt.execute(params![file_id, line_no, content])?;
                fts_stmt.execute(params![content, file_id, line_no])?;
            }
        }

        self.set_meta(&format!("eol:{rel_path}"), eol)?;

        let mut symbol_ids: Vec<i64> = Vec::with_capacity(symbols.len());
        {
            let mut sym_stmt = self.conn.prepare(
                "INSERT INTO symbols(file_id, name, kind, line_start, line_end, byte_start, byte_end)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for sym in symbols {
                sym_stmt.execute(params![
                    file_id,
                    sym.name,
                    sym.kind,
                    sym.line_start,
                    sym.line_end,
                    sym.byte_start,
                    sym.byte_end,
                ])?;
                symbol_ids.push(self.conn.last_insert_rowid());
            }
        }

        if embed_semantic && !semantic_chunks.is_empty() {
            let name_to_id: HashMap<String, i64> = symbols
                .iter()
                .zip(symbol_ids.iter())
                .map(|(sym, id)| (format!("{}:{}", sym.name, sym.line_start), *id))
                .collect();
            let mut chunk_stmt = self.conn.prepare(
                "INSERT INTO semantic_chunks(
                    file_id, symbol_id, chunk_kind, line_start, line_end, symbol_name, text, vector
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            let mut embed_dim: Option<usize> = None;
            let mut backend_kind: Option<ast_sgrep_embed::EmbedBackendKind> = None;
            for chunk in semantic_chunks {
                let key = format!("{}:{}", chunk.symbol_name, chunk.line_start);
                let symbol_id = name_to_id.get(&key).copied();
                let text = crate::semantic_chunk::render_chunk_text(chunk);
                let result = ast_sgrep_embed::embed_with_chain(&text, embed_backend);
                embed_dim = Some(result.vector.len());
                backend_kind = Some(result.backend);
                let bytes = ast_sgrep_embed::embed_to_bytes(&result.vector);
                chunk_stmt.execute(params![
                    file_id,
                    symbol_id,
                    "symbol",
                    chunk.line_start,
                    chunk.line_end,
                    chunk.symbol_name,
                    text,
                    bytes,
                ])?;
            }
            if let Some(kind) = backend_kind {
                self.set_meta("embed_backend", kind.as_meta_str())?;
            }
            if let Some(dim) = embed_dim {
                self.set_meta("embed_dim", &dim.to_string())?;
            }
        }

        {
            let mut caller_stmt = self.conn.prepare(
                "INSERT INTO callers(file_id, caller, callee, line_no, byte_start, byte_end)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for caller in callers {
                caller_stmt.execute(params![
                    file_id,
                    caller.caller,
                    caller.callee,
                    caller.line_no,
                    caller.byte_start,
                    caller.byte_end,
                ])?;
            }
        }

        {
            let mut import_stmt = self.conn.prepare(
                "INSERT INTO imports(file_id, module_path, line_no) VALUES(?1, ?2, ?3)",
            )?;
            for import in imports {
                import_stmt.execute(params![file_id, import.module_path, import.line_no])?;
            }
        }

        let _ = crate::semantic_ivf::invalidate_semantic_ivf(&self.db_path);

        Ok(file_id)
    }

    pub fn remove_file(&self, rel_path: &str) -> Result<()> {
        if let Ok(file_id) = self.conn.query_row(
            "SELECT id FROM files WHERE path = ?1",
            params![rel_path],
            |row| row.get::<_, i64>(0),
        ) {
            delete_file_children(&self.conn, file_id)?;
            self.conn.execute("DELETE FROM files WHERE id = ?1", params![file_id])?;
            self.delete_meta(&format!("eol:{rel_path}"))?;
            let _ = crate::semantic_ivf::invalidate_semantic_ivf(&self.db_path);
        }
        Ok(())
    }
}
