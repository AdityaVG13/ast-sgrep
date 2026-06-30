use std::collections::HashMap;
use std::path::Path;

use rusqlite::{params, Connection};

use super::index_db_path;
use crate::{IndexStatus, Result};

/// SQLite-backed index store.
pub struct IndexStore {
    conn: Connection,
    root: std::path::PathBuf,
    db_path: std::path::PathBuf,
}

impl IndexStore {
    pub fn open(root: &Path, index_path: Option<&Path>) -> Result<Self> {
        let db_path = index_db_path(root, index_path);
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&db_path)?;
        let store = Self {
            conn,
            root: root.to_path_buf(),
            db_path,
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;

            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                language TEXT,
                mtime_secs INTEGER NOT NULL,
                mtime_nanos INTEGER NOT NULL,
                content_hash TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS lines (
                file_id INTEGER NOT NULL,
                line_no INTEGER NOT NULL,
                content TEXT NOT NULL,
                PRIMARY KEY (file_id, line_no),
                FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS symbols (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                byte_start INTEGER NOT NULL,
                byte_end INTEGER NOT NULL,
                FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);

            CREATE TABLE IF NOT EXISTS callers (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                caller TEXT NOT NULL,
                callee TEXT NOT NULL,
                line_no INTEGER NOT NULL,
                byte_start INTEGER NOT NULL,
                byte_end INTEGER NOT NULL,
                FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_callers_callee ON callers(callee);
            CREATE INDEX IF NOT EXISTS idx_callers_caller ON callers(caller);

            CREATE TABLE IF NOT EXISTS imports (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                module_path TEXT NOT NULL,
                line_no INTEGER NOT NULL,
                FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_imports_module ON imports(module_path);

            CREATE VIRTUAL TABLE IF NOT EXISTS lines_fts USING fts5(
                content,
                file_id UNINDEXED,
                line_no UNINDEXED,
                tokenize = 'porter unicode61'
            );

            CREATE TABLE IF NOT EXISTS embeddings (
                file_id INTEGER NOT NULL,
                line_no INTEGER NOT NULL,
                vector BLOB NOT NULL,
                PRIMARY KEY (file_id, line_no),
                FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS semantic_chunks (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                symbol_id INTEGER,
                chunk_kind TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                symbol_name TEXT,
                text TEXT NOT NULL,
                vector BLOB NOT NULL,
                FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE,
                FOREIGN KEY (symbol_id) REFERENCES symbols(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_semantic_chunks_symbol ON semantic_chunks(symbol_name);
            ",
        )?;
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let result = self.conn.query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        );
        match result {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
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
            self.conn.execute("DELETE FROM lines WHERE file_id = ?1", params![id])?;
            self.conn.execute("DELETE FROM lines_fts WHERE file_id = ?1", params![id])?;
            self.conn.execute("DELETE FROM symbols WHERE file_id = ?1", params![id])?;
            self.conn.execute("DELETE FROM callers WHERE file_id = ?1", params![id])?;
            self.conn.execute("DELETE FROM imports WHERE file_id = ?1", params![id])?;
            self.conn.execute("DELETE FROM embeddings WHERE file_id = ?1", params![id])?;
            self.conn.execute("DELETE FROM semantic_chunks WHERE file_id = ?1", params![id])?;
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
            self.conn.execute("DELETE FROM lines WHERE file_id = ?1", params![file_id])?;
            self.conn.execute("DELETE FROM lines_fts WHERE file_id = ?1", params![file_id])?;
            self.conn.execute("DELETE FROM symbols WHERE file_id = ?1", params![file_id])?;
            self.conn.execute("DELETE FROM callers WHERE file_id = ?1", params![file_id])?;
            self.conn.execute("DELETE FROM imports WHERE file_id = ?1", params![file_id])?;
            self.conn.execute("DELETE FROM embeddings WHERE file_id = ?1", params![file_id])?;
            self.conn.execute("DELETE FROM semantic_chunks WHERE file_id = ?1", params![file_id])?;
            self.conn.execute("DELETE FROM files WHERE id = ?1", params![file_id])?;
            let _ = crate::semantic_ivf::invalidate_semantic_ivf(&self.db_path);
        }
        Ok(())
    }

    pub fn file_hash(&self, rel_path: &str) -> Result<Option<String>> {
        let result = self.conn.query_row(
            "SELECT content_hash FROM files WHERE path = ?1",
            params![rel_path],
            |row| row.get(0),
        );
        match result {
            Ok(hash) => Ok(Some(hash)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn file_mtime(&self, rel_path: &str) -> Result<Option<(i64, u32)>> {
        let result = self.conn.query_row(
            "SELECT mtime_secs, mtime_nanos FROM files WHERE path = ?1",
            params![rel_path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match result {
            Ok(mtime) => Ok(Some(mtime)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn all_file_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM files ORDER BY path")?;
        let paths = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(paths)
    }

    pub fn status(&self) -> Result<IndexStatus> {
        let file_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;
        let line_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM lines", [], |row| row.get(0))?;
        let symbol_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))?;
        let caller_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM callers", [], |row| row.get(0))?;
        let import_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM imports", [], |row| row.get(0))?;
        let semantic_chunk_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM semantic_chunks", [], |row| row.get(0))
            .unwrap_or(0);
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

    /// All lines for tantivy sidecar rebuild.
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

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Max `semantic_chunks.id` for IVF fingerprinting.
    pub fn semantic_chunk_max_id(&self) -> Result<Option<i64>> {
        let result = self.conn.query_row(
            "SELECT MAX(id) FROM semantic_chunks",
            [],
            |row| row.get(0),
        );
        match result {
            Ok(id) => Ok(id),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// All semantic chunks in stable id order (for IVF sidecar alignment).
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
        let mut out = Vec::new();
        if let Some(lang) = lang_filter {
            let mut stmt = self.conn.prepare(&sql)?;
            let mut rows = stmt.query(rusqlite::params![lang])?;
            while let Some(row) = rows.next()? {
                out.push(read_semantic_chunk_row(row)?);
            }
        } else {
            let mut stmt = self.conn.prepare(&sql)?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                out.push(read_semantic_chunk_row(row)?);
            }
        }
        Ok(out)
    }

    /// Symbols defined in a single file.
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

    /// Incoming calls: (file, line, caller, callee) for a callee name.
    pub fn incoming_calls(
        &self,
        callee: &str,
    ) -> Result<Vec<(String, u32, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.path, c.line_no, c.caller, c.callee
             FROM callers c JOIN files f ON f.id = c.file_id
             WHERE lower(c.callee) = lower(?1)
             ORDER BY f.path, c.line_no",
        )?;
        let rows = stmt.query_map(params![callee], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Outgoing calls: (file, line, caller, callee) for a caller name.
    pub fn outgoing_calls(
        &self,
        caller: &str,
    ) -> Result<Vec<(String, u32, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.path, c.line_no, c.caller, c.callee
             FROM callers c JOIN files f ON f.id = c.file_id
             WHERE lower(c.caller) = lower(?1)
             ORDER BY f.path, c.line_no",
        )?;
        let rows = stmt.query_map(params![caller], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Reconstruct file text from indexed lines (for LSP incremental edits).
    pub fn file_text(&self, rel_path: &str) -> Result<Option<String>> {
        let lines = self.file_lines(rel_path)?;
        if lines.is_empty() {
            return Ok(None);
        }
        Ok(Some(
            lines
                .iter()
                .map(|(_, content)| content.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        ))
    }

    /// All lines for a file in order.
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

    /// Get a single line of text from the index.
    pub fn line_content(&self, rel_path: &str, line_no: u32) -> Result<Option<String>> {
        let result = self.conn.query_row(
            "SELECT l.content FROM lines l
             JOIN files f ON f.id = l.file_id
             WHERE f.path = ?1 AND l.line_no = ?2",
            params![rel_path, line_no],
            |row| row.get(0),
        );
        match result {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

fn read_semantic_chunk_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ast_sgrep_embed::SemanticChunkRow> {
    let file: String = row.get(0)?;
    let line_start: u32 = row.get(1)?;
    let line_end: u32 = row.get(2)?;
    let symbol: String = row.get::<_, Option<String>>(3)?.unwrap_or_default();
    let excerpt: String = row.get(4)?;
    let vector: Vec<u8> = row.get(5)?;
    Ok((
        file,
        line_start,
        line_end,
        symbol,
        excerpt,
        ast_sgrep_embed::embed_from_bytes(&vector).unwrap_or_default(),
    ))
}

#[derive(Debug, Clone)]
pub struct SymbolRow {
    pub name: String,
    pub kind: String,
    pub line_start: u32,
    pub line_end: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}

#[derive(Debug, Clone)]
pub struct CallerRow {
    pub caller: String,
    pub callee: String,
    pub line_no: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}

#[derive(Debug, Clone)]
pub struct ImportRow {
    pub module_path: String,
    pub line_no: u32,
}