use crate::store::sql::{configure_connection, configure_connection_readonly};
use crate::store::{index_db_path, INDEX_DB, INDEX_DIR};
use crate::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
type TantivySearchRow = (String, u32, String, Option<String>, usize);
pub const LEXICAL_DB: &str = "lexical.db";
pub const TANTIVY_AUTO_THRESHOLD: usize = 1000;
pub struct TantivySidecar {
    db_path: PathBuf,
    source_identity: String,
    conn: Connection,
}
impl TantivySidecar {
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_for_index(root, None)
    }
    pub fn open_for_index(root: &Path, index_path: Option<&Path>) -> Result<Self> {
        let db_path = sidecar_path(root, index_path);
        if let Some(dir) = db_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(&db_path)?;
        configure_connection(&conn)?;
        let sidecar = Self {
            db_path,
            source_identity: source_identity(root, index_path),
            conn,
        };
        sidecar.init_schema()?;
        Ok(sidecar)
    }
    /// Open an existing lexical sidecar for search without creating an empty DB.
    ///
    /// Empty / schema-only auto-created sidecars are not treated as ready (hkdi / s7jw.2).
    pub fn open_existing_for_search(
        root: &Path,
        index_path: Option<&Path>,
    ) -> Result<Option<Self>> {
        let db_path = sidecar_path(root, index_path);
        if !db_path.exists() {
            return Ok(None);
        }
        let metadata = std::fs::metadata(&db_path)?;
        // Never treat a zero-length / freshly-touched empty file as ready.
        if metadata.len() == 0 {
            return Ok(None);
        }
        let conn =
            Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        configure_connection_readonly(&conn)?;
        let sidecar = Self {
            db_path,
            source_identity: source_identity(root, index_path),
            conn,
        };
        if !sidecar.is_search_ready()? {
            return Ok(None);
        }
        Ok(Some(sidecar))
    }
    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            " CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL); CREATE VIRTUAL TABLE IF NOT EXISTS lines_fts USING fts5(
                file UNINDEXED, line_no UNINDEXED, language UNINDEXED, content, tokenize = 'porter unicode61'
            );",
        )?;
        Ok(())
    }
    pub fn path(&self) -> &Path {
        &self.db_path
    }
    pub fn exists(&self) -> bool {
        self.db_path.exists()
    }
    /// True when the sidecar has indexed at least one line (hkdi / y1oy.4).
    pub fn is_search_ready(&self) -> Result<bool> {
        let lines: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'lines'", [], |r| {
                r.get(0)
            })
            .ok();
        let Some(raw) = lines else {
            return Ok(false);
        };
        let n: usize = raw.parse().unwrap_or(0);
        Ok(n > 0)
    }
    pub fn rebuild_from_lines(&self, lines: &[crate::store::IndexedLineRow]) -> Result<()> {
        self.rebuild_from_lines_with_generation(lines, 0)
    }

    pub fn rebuild_from_lines_with_generation(
        &self,
        lines: &[crate::store::IndexedLineRow],
        source_generation: i64,
    ) -> Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        if let Err(e) = self.rebuild_inner(lines, source_generation) {
            let _ = self.conn.execute_batch("ROLLBACK");
            return Err(e);
        }
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }
    fn rebuild_inner(
        &self,
        lines: &[crate::store::IndexedLineRow],
        source_generation: i64,
    ) -> Result<()> {
        self.conn.execute("DELETE FROM lines_fts", [])?;
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO lines_fts(file, line_no, language, content) VALUES(?1, ?2, ?3, ?4)",
        )?;
        for (file, line_no, content, language) in lines {
            stmt.execute(params![
                file.as_ref(),
                line_no,
                language.as_deref().unwrap_or(""),
                content
            ])?;
        }
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES('lines', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![lines.len().to_string()],
        )?;
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES('source_generation', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![source_generation.to_string()],
        )?;
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES('source_identity', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![self.source_identity],
        )?;
        Ok(())
    }
    pub fn is_fresh(&self, source_generation: i64) -> Result<bool> {
        let owner = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'source_identity'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        // Local generation counters can coincide across databases. Legacy or
        // copied sidecars without this exact owner binding must be rebuilt.
        if owner.as_deref() != Some(self.source_identity.as_str()) {
            return Ok(false);
        }
        let stored = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'source_generation'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(stored.and_then(|value| value.parse::<i64>().ok()) == Some(source_generation))
    }
    /// Language is canonical (`Language::as_str`) and admitted before the
    /// candidate limit, matching primary lexical FTS rather than filtering a page.
    pub fn search(
        &self,
        terms: &[String],
        limit: usize,
        lang_filter: Option<&str>,
    ) -> Result<Vec<TantivySearchRow>> {
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let fts_query = crate::fts::escape_fts_query(terms);
        let mut stmt = self.conn.prepare_cached(
            "SELECT file, line_no, content, language FROM lines_fts
             WHERE lines_fts MATCH ?1 AND (?3 IS NULL OR language = ?3)
             ORDER BY bm25(lines_fts) LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![fts_query, limit as i64, lang_filter], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        rows.into_iter()
            .enumerate()
            .map(|(rank, row)| {
                let (file, line_no, content, lang) = row?;
                Ok((
                    file,
                    line_no,
                    content,
                    (!lang.is_empty()).then_some(lang),
                    rank,
                ))
            })
            .collect()
    }
}
pub fn sidecar_path(root: &Path, index_path: Option<&Path>) -> PathBuf {
    let db = index_db_path(root, index_path);
    let parent = db.parent().unwrap_or(root);
    match db.file_name() {
        // Keep the established default/generation layout. Custom database
        // basenames never share its lexical sidecar or one another's.
        Some(name) if name == INDEX_DB => parent.join(LEXICAL_DB),
        Some(name) => {
            let mut name = name.to_os_string();
            name.push(".lexical.db");
            parent.join(name)
        }
        None => root.join(INDEX_DIR).join(LEXICAL_DB),
    }
}

fn source_identity(root: &Path, index_path: Option<&Path>) -> String {
    let db = index_db_path(root, index_path);
    let db = db.canonicalize().unwrap_or(db);
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut hasher = blake3::Hasher::new();
    for path in [&db, &root] {
        let bytes = path.as_os_str().as_encoded_bytes();
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    hasher.finalize().to_hex().to_string()
}

pub fn should_use_tantivy(file_count: usize, force: bool) -> bool {
    force || file_count >= TANTIVY_AUTO_THRESHOLD
}
