use std::path::{Path, PathBuf};
use rusqlite::{params, Connection};
use crate::store::pragmas::configure_connection;
use crate::store::{index_db_path, INDEX_DIR};
use crate::Result;
type TantivySearchRow = (String, u32, String, Option<String>, usize);
pub const LEXICAL_DB: &str = "lexical.db";
pub const TANTIVY_AUTO_THRESHOLD: usize = 1000;
pub struct TantivySidecar {
    db_path: PathBuf,
    conn: Connection,
}
impl TantivySidecar {
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_for_index(root, None)
    }

    pub fn open_for_index(root: &Path, index_path: Option<&Path>) -> Result<Self> {
        let dir = index_db_path(root, index_path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| root.join(INDEX_DIR));
        std::fs::create_dir_all(&dir)?;
        let db_path = dir.join(LEXICAL_DB);
        let conn = Connection::open(&db_path)?;
        configure_connection(&conn)?;
        let sidecar = Self { db_path, conn };
        sidecar.init_schema()?;
        Ok(sidecar)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE VIRTUAL TABLE IF NOT EXISTS lines_fts USING fts5(
                file UNINDEXED, line_no UNINDEXED, language UNINDEXED, content,
                tokenize = 'porter unicode61'
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

    pub fn rebuild_from_lines(&self, lines: &[crate::store::IndexedLineRow]) -> Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        if let Err(e) = self.rebuild_inner(lines) {
            let _ = self.conn.execute_batch("ROLLBACK");
            return Err(e);
        }
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }

    fn rebuild_inner(&self, lines: &[crate::store::IndexedLineRow]) -> Result<()> {
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
            "INSERT INTO meta(key, value) VALUES('lines', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![lines.len().to_string()],
        )?;
        Ok(())
    }

    pub fn search(&self, terms: &[String], limit: usize) -> Result<Vec<TantivySearchRow>> {
        if terms.is_empty() { return Ok(Vec::new()); }
        let fts_query = crate::fts::escape_fts_query(terms);
        let mut stmt = self.conn.prepare_cached(
            "SELECT file, line_no, content, language FROM lines_fts
             WHERE lines_fts MATCH ?1 ORDER BY bm25(lines_fts) LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![fts_query, limit as i64], |row| {
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
pub fn should_use_tantivy(file_count: usize, force: bool) -> bool {
    force || file_count >= TANTIVY_AUTO_THRESHOLD
}
