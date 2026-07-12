use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path};
use std::sync::Arc;
use ast_sgrep_lang::PatternNode;
use blake3::Hasher;
use rusqlite::types::{Type, ValueRef};
use rusqlite::{params, Connection, OptionalExtension};
use super::index_db_path;
use super::pragmas::configure_connection;
use super::sql::{
    calls_matching, count_star, like_terms_filter, optional_row, query_cached_map, query_limit_map,
    query_map_rows,
};
use crate::{IndexStatus, Result};
const SCHEMA_VERSION: i64 = 5;
const DEFAULT_EMBED_CACHE_CAP: usize = 100_000;
const IMPORT_SELECT: &str =
    "SELECT f.path, f.language, i.module_path, i.line_no FROM imports i JOIN files f ON f.id = i.file_id";
const SYM_LOC: &str =
    "SELECT f.path, s.name, f.language, s.line_start, s.line_end FROM symbols s JOIN files f ON f.id = s.file_id";
const SYM_FILE: &str =
    "SELECT f.path, f.language, s.name, s.kind, s.line_start, s.line_end FROM symbols s JOIN files f ON f.id = s.file_id";
// Full DDL for the current schema; init_schema applies when user_version is lower.
const SCHEMA_DDL: &str = "\
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
pub type IndexedLineRow = (Arc<str>, u32, String, Option<Arc<str>>);
pub type ImportQueryRow = (String, Option<String>, String, u32);
pub type CallRow = (String, u32, String, String);
#[derive(Debug, Clone)]
pub struct SymbolFileRow {
    pub path: String, pub language: Option<String>, pub name: String, pub kind: String,
    pub line_start: u32, pub line_end: u32,
}
pub struct PatternNodeRow {
    pub path: String, pub language: Option<String>, pub line_start: u32, pub line_end: u32,
    pub excerpt: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticChunkStats { pub count: usize, pub max_id: i64, pub dim: usize, }
#[derive(Debug, Clone)]
pub struct SymbolRow {
    pub name: String, pub kind: String, pub line_start: u32, pub line_end: u32,
    pub byte_start: usize, pub byte_end: usize,
}
#[derive(Debug, Clone)]
pub struct CallerRow {
    pub caller: String, pub callee: String, pub line_no: u32,
    pub byte_start: usize, pub byte_end: usize,
}
#[derive(Debug, Clone)]
pub struct ImportRow { pub module_path: String, pub line_no: u32, }
#[derive(Debug, Clone)]
pub struct SymbolLocationRow {
    pub path: String, pub name: String, pub language: Option<String>,
    pub line_start: u32, pub line_end: u32,
}
pub struct UpsertFileInput<'a> {
    pub rel_path: &'a str, pub language: Option<&'a str>,
    pub mtime_secs: i64, pub mtime_nanos: u32, pub content_hash: &'a str,
    pub lines: &'a [(u32, String)], pub eol: &'a str,
    pub symbols: &'a [SymbolRow], pub callers: &'a [CallerRow], pub imports: &'a [ImportRow],
    pub pattern_nodes: &'a [PatternNode],
    pub semantic_chunks: &'a [crate::semantic_chunk::SemanticChunkInput],
    pub embed_semantic: bool, pub embed_backend: ast_sgrep_embed::EmbedPreference,
}
pub struct IndexStore {
    conn: Connection, root: std::path::PathBuf, db_path: std::path::PathBuf,
    file_tx_active: std::cell::Cell<bool>, cache_seq: std::cell::Cell<i64>,
}
impl IndexStore {
    pub fn open(root: &Path, index_path: Option<&Path>) -> Result<Self> {
        let db_path = index_db_path(root, index_path);
        if let Some(p) = db_path.parent() { std::fs::create_dir_all(p)?; }
        let conn = Connection::open(&db_path)?;
        configure_connection(&conn)?;
        let store = Self {
            conn, root: root.to_path_buf(), db_path,
            file_tx_active: std::cell::Cell::new(false), cache_seq: std::cell::Cell::new(0),
        };
        store.init_schema()?;
        store.init_cache_seq()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<()> {
        let version: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version >= SCHEMA_VERSION { return Ok(()); }
        self.conn.execute_batch(SCHEMA_DDL)?;
        if version < 3 {
            self.conn.execute_batch(
                "INSERT INTO lines_trigram(rowid, content) SELECT rowid, content FROM lines;",
            )?;
        }
        self.conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
        Ok(())
    }

    pub fn root(&self) -> &Path { &self.root }
    pub fn db_path(&self) -> &Path { &self.db_path }
    pub fn connection(&self) -> &Connection { &self.conn }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.prepare_cached(
            "INSERT INTO meta(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )?.execute(params![key, value])?;
        Ok(())
    }
    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        optional_row(&self.conn, "SELECT value FROM meta WHERE key = ?1", &[&key], |r| r.get(0))
    }
    pub fn delete_meta(&self, key: &str) -> Result<()> {
        self.conn.execute("DELETE FROM meta WHERE key = ?1", params![key])?;
        Ok(())
    }

    pub fn begin_file_tx(&self) -> Result<()> {
        if !self.conn.is_autocommit() { return Ok(()); }
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        self.file_tx_active.set(true);
        Ok(())
    }
    pub fn commit_file_tx(&self) -> Result<()> {
        if !self.file_tx_active.replace(false) { return Ok(()); }
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }
    pub fn rollback_file_tx(&self) -> Result<()> {
        if !self.file_tx_active.replace(false) { return Ok(()); }
        let _ = self.conn.execute_batch("ROLLBACK");
        Ok(())
    }
    pub fn begin_bulk_tx(&self) -> Result<()> {
        if !self.conn.is_autocommit() {
            return Ok(());
        }
        // Bulk load: big page cache, memory temps, deferred durability until COMMIT.
        self.conn.execute_batch(
            "PRAGMA temp_store = MEMORY;
             PRAGMA cache_size = -131072;
             PRAGMA mmap_size = 536870912;
             PRAGMA synchronous = OFF;
             BEGIN IMMEDIATE",
        )?;
        Ok(())
    }
    pub fn commit_bulk_tx(&self) -> Result<()> {
        if self.conn.is_autocommit() {
            return Ok(());
        }
        self.file_tx_active.set(false);
        self.conn.execute_batch("COMMIT")?;
        // Restore durable settings; avoid TRUNCATE checkpoint (full fsync) on hot path.
        let _ = self.conn.execute_batch(
            "PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -16384",
        );
        Ok(())
    }
    pub fn rollback_bulk_tx(&self) -> Result<()> {
        if self.conn.is_autocommit() { return Ok(()); }
        self.file_tx_active.set(false);
        let _ = self.conn.execute_batch("ROLLBACK");
        Ok(())
    }

    fn init_cache_seq(&self) -> Result<()> {
        let max: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(accessed_at), 0) FROM embed_cache", [], |r| r.get(0),
        )?;
        self.cache_seq.set(max);
        Ok(())
    }
    fn next_cache_seq(&self) -> i64 {
        let n = self.cache_seq.get().saturating_add(1);
        self.cache_seq.set(n);
        n
    }
    fn drop_cache(&self, h: &str, m: &str) {
        let _ = self.conn.execute(
            "DELETE FROM embed_cache WHERE chunk_hash = ?1 AND model_id = ?2", params![h, m],
        );
    }

    fn lookup_embed_cache(&self, h: &str, m: &str) -> Result<Option<CacheRow>> {
        let raw: Option<(Vec<u8>, String, i64)> = optional_row(
            &self.conn,
            "SELECT vector, backend, dim FROM embed_cache WHERE chunk_hash = ?1 AND model_id = ?2",
            &[&h, &m], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let Some((vector, backend_str, dim_i64)) = raw else { return Ok(None); };
        let Some(backend) = ast_sgrep_embed::EmbedBackendKind::parse(&backend_str) else {
            self.drop_cache(h, m); return Ok(None);
        };
        let ok = usize::try_from(dim_i64).ok().and_then(|d| d.checked_mul(4))
            .is_some_and(|n| n > 0 && vector.len() == n);
        if !ok { self.drop_cache(h, m); return Ok(None); }
        Ok(Some(CacheRow { vector, backend, dim: dim_i64 as usize }))
    }

    fn insert_embed_cache_entries(&self, entries: &[CacheEntry]) -> Result<()> {
        if entries.is_empty() { return Ok(()); }
        let at = self.next_cache_seq();
        let mut st = self.conn.prepare_cached(
            "INSERT INTO embed_cache(chunk_hash, model_id, backend, dim, vector, accessed_at)
             VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(chunk_hash, model_id) DO UPDATE SET
             vector=excluded.vector, backend=excluded.backend, dim=excluded.dim, accessed_at=excluded.accessed_at",
        )?;
        for e in entries {
            st.execute(params![&e.chunk_hash, &e.model_id, e.backend.as_meta_str(), e.dim as i64, &e.vector, at])?;
        }
        Ok(())
    }
    fn touch_embed_cache_entries(&self, keys: &[(String, String)]) -> Result<()> {
        if keys.is_empty() { return Ok(()); }
        let at = self.next_cache_seq();
        let mut st = self.conn.prepare_cached(
            "UPDATE embed_cache SET accessed_at = ?1 WHERE chunk_hash = ?2 AND model_id = ?3",
        )?;
        for (h, m) in keys { st.execute(params![at, h, m])?; }
        Ok(())
    }
    fn evict_embed_cache(&self, max_entries: usize) -> Result<()> {
        if max_entries == 0 { self.conn.execute("DELETE FROM embed_cache", [])?; return Ok(()); }
        let count: i64 = self.conn.query_row("SELECT COUNT(*) FROM embed_cache", [], |r| r.get(0))?;
        let over = count.saturating_sub(max_entries as i64);
        if over <= 0 { return Ok(()); }
        self.conn.prepare_cached(
            "DELETE FROM embed_cache WHERE rowid IN (
               SELECT rowid FROM embed_cache ORDER BY accessed_at ASC, rowid ASC LIMIT ?1)",
        )?.execute(params![over])?;
        Ok(())
    }

    pub fn clear_all_data(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM lines_trigram; DELETE FROM lines_fts; DELETE FROM semantic_chunks;
             DELETE FROM pattern_nodes; DELETE FROM embeddings; DELETE FROM imports;
             DELETE FROM callers; DELETE FROM symbols; DELETE FROM lines; DELETE FROM files;",
        )?;
        let _ = self.conn.execute_batch("VACUUM");
        Ok(())
    }

    pub fn upsert_file(&self, input: UpsertFileInput<'_>) -> Result<i64> {
        let emb = self.embed_chunks(input.semantic_chunks, input.embed_semantic, input.embed_backend)?;
        self.begin_file_tx()?;
        match self.upsert_file_inner(input, &emb.chunks) {
            Ok(id) => {
                if let Err(e) = self.insert_embed_cache_entries(&emb.cache_entries) {
                    eprintln!("[asgrep] warning: failed to write embedding cache: {e}");
                }
                let hits: Vec<_> = emb.cache_hits.iter().map(|h| (h.chunk_hash.clone(), h.model_id.clone())).collect();
                if let Err(e) = self.touch_embed_cache_entries(&hits) {
                    eprintln!("[asgrep] warning: failed to touch embedding cache: {e}");
                }
                if let Err(e) = self.evict_embed_cache(embed_cache_cap()) {
                    eprintln!("[asgrep] warning: failed to evict embedding cache: {e}");
                }
                self.commit_file_tx()?;
                Ok(id)
            }
            Err(e) => { self.rollback_file_tx()?; Err(e) }
        }
    }

    fn upsert_file_inner(&self, input: UpsertFileInput<'_>, emb: &[EmbeddedChunk]) -> Result<i64> {
        let file_id = self.upsert_file_row(
            input.rel_path, input.language, input.mtime_secs, input.mtime_nanos, input.content_hash,
        )?;
        self.insert_lines(file_id, input.lines)?;
        self.set_meta(&format!("eol:{}", input.rel_path), input.eol)?;
        let symbol_ids = self.insert_symbols(file_id, input.symbols)?;
        self.insert_semantic_chunks(file_id, input.symbols, &symbol_ids, input.semantic_chunks, emb)?;
        self.insert_callers(file_id, input.callers)?;
        self.insert_imports(file_id, input.imports)?;
        self.insert_pattern_nodes(file_id, input.pattern_nodes)?;
        crate::semantic_ann::mark_semantic_ivf_stale(self);
        Ok(file_id)
    }

    fn upsert_file_row(
        &self, path: &str, lang: Option<&str>, mtime_secs: i64, mtime_nanos: u32, hash: &str,
    ) -> Result<i64> {
        let existing: Option<i64> = optional_row(
            &self.conn, "SELECT id FROM files WHERE path = ?1", &[&path], |r| r.get(0),
        )?;
        if let Some(id) = existing {
            delete_file_children(&self.conn, id)?;
            self.conn.prepare_cached(
                "UPDATE files SET language=?1, mtime_secs=?2, mtime_nanos=?3, content_hash=?4 WHERE id=?5",
            )?.execute(params![lang, mtime_secs, mtime_nanos, hash, id])?;
            return Ok(id);
        }
        self.conn.prepare_cached(
            "INSERT INTO files(path, language, mtime_secs, mtime_nanos, content_hash) VALUES(?1,?2,?3,?4,?5)",
        )?.execute(params![path, lang, mtime_secs, mtime_nanos, hash])?;
        Ok(self.conn.last_insert_rowid())
    }

    fn insert_lines(&self, file_id: i64, lines: &[(u32, String)]) -> Result<()> {
        let mut ls = self
            .conn
            .prepare_cached("INSERT INTO lines(file_id, line_no, content) VALUES(?1,?2,?3)")?;
        let mut fts = self.conn.prepare_cached(
            "INSERT INTO lines_fts(rowid, content, file_id, line_no) VALUES(?1,?2,?3,?4)",
        )?;
        let mut tri = self
            .conn
            .prepare_cached("INSERT INTO lines_trigram(rowid, content) VALUES(?1,?2)")?;
        for (no, content) in lines {
            ls.execute(params![file_id, no, content])?;
            let rid = self.conn.last_insert_rowid();
            fts.execute(params![rid, content, file_id, no])?;
            tri.execute(params![rid, content])?;
        }
        Ok(())
    }

    fn insert_symbols(&self, file_id: i64, symbols: &[SymbolRow]) -> Result<Vec<i64>> {
        let mut ids = Vec::with_capacity(symbols.len());
        let mut st = self.conn.prepare_cached(
            "INSERT INTO symbols(file_id, name, kind, line_start, line_end, byte_start, byte_end) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        )?;
        for s in symbols {
            st.execute(params![file_id, s.name, s.kind, s.line_start, s.line_end, s.byte_start as i64, s.byte_end as i64])?;
            ids.push(self.conn.last_insert_rowid());
        }
        Ok(ids)
    }

    fn insert_semantic_chunks(
        &self, file_id: i64, symbols: &[SymbolRow], symbol_ids: &[i64],
        chunks: &[crate::semantic_chunk::SemanticChunkInput], emb: &[EmbeddedChunk],
    ) -> Result<()> {
        if emb.is_empty() { return Ok(()); }
        if emb.len() < chunks.len() && emb[0].backend == ast_sgrep_embed::EmbedBackendKind::Neural {
            let first = &chunks[0];
            let last = &chunks[chunks.len() - 1];
            for e in emb {
                self.conn.execute(
                    "INSERT INTO semantic_chunks(file_id, symbol_id, chunk_kind, line_start, line_end, symbol_name, text, vector)
                     VALUES(?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![file_id, "file", first.line_start, last.line_end, "", &e.text, &e.vector_bytes],
                )?;
            }
            let last = &emb[emb.len() - 1];
            return self.persist_embed_metadata(Some(last.dim), Some(last.backend));
        }
        let name_to_id: HashMap<String, i64> = symbols.iter().zip(symbol_ids)
            .map(|(s, id)| (format!("{}:{}", s.name, s.line_start), *id)).collect();
        let mut st = self.conn.prepare_cached(
            "INSERT INTO semantic_chunks(file_id, symbol_id, chunk_kind, line_start, line_end, symbol_name, text, vector)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        )?;
        for (c, e) in chunks.iter().zip(emb.iter()) {
            let sid = name_to_id.get(&format!("{}:{}", c.symbol_name, c.line_start)).copied();
            st.execute(params![file_id, sid, "symbol", c.line_start, c.line_end, c.symbol_name, e.text, e.vector_bytes])?;
        }
        let last = emb.last();
        self.persist_embed_metadata(last.map(|e| e.dim), last.map(|e| e.backend))
    }

    fn persist_embed_metadata(
        &self, dim: Option<usize>, kind: Option<ast_sgrep_embed::EmbedBackendKind>,
    ) -> Result<()> {
        if let Some(k) = kind {
            self.set_meta("embed_backend", k.as_meta_str())?;
            if k == ast_sgrep_embed::EmbedBackendKind::Neural {
                self.set_meta("embed_model", ast_sgrep_embed::neural_configured_model_id())?;
            } else {
                self.delete_meta("embed_model")?;
            }
        }
        if let Some(d) = dim { self.set_meta("embed_dim", &d.to_string())?; }
        Ok(())
    }

    fn insert_callers(&self, file_id: i64, callers: &[CallerRow]) -> Result<()> {
        let mut st = self.conn.prepare_cached(
            "INSERT INTO callers(file_id, caller, callee, line_no, byte_start, byte_end) VALUES(?1,?2,?3,?4,?5,?6)",
        )?;
        for c in callers {
            st.execute(params![file_id, c.caller, c.callee, c.line_no, c.byte_start as i64, c.byte_end as i64])?;
        }
        Ok(())
    }
    fn insert_pattern_nodes(&self, file_id: i64, nodes: &[PatternNode]) -> Result<()> {
        let mut st = self.conn.prepare_cached(
            "INSERT INTO pattern_nodes(file_id, signature, line_start, line_end, excerpt) VALUES(?1,?2,?3,?4,?5)",
        )?;
        for n in nodes { st.execute(params![file_id, n.signature, n.line_start, n.line_end, n.excerpt])?; }
        Ok(())
    }
    fn insert_imports(&self, file_id: i64, imports: &[ImportRow]) -> Result<()> {
        let mut st = self.conn.prepare_cached("INSERT INTO imports(file_id, module_path, line_no) VALUES(?1,?2,?3)")?;
        for i in imports { st.execute(params![file_id, i.module_path, i.line_no])?; }
        Ok(())
    }

    pub fn remove_file(&self, rel_path: &str) -> Result<()> {
        if let Some(id) = optional_row(
            &self.conn, "SELECT id FROM files WHERE path = ?1", &[&rel_path], |r| r.get::<_, i64>(0),
        )? {
            delete_file_children(&self.conn, id)?;
            self.conn.execute("DELETE FROM files WHERE id = ?1", params![id])?;
            self.delete_meta(&format!("eol:{rel_path}"))?;
            crate::semantic_ann::mark_semantic_ivf_stale(self);
        }
        Ok(())
    }
    pub fn file_hash(&self, rel_path: &str) -> Result<Option<String>> {
        optional_row(&self.conn, "SELECT content_hash FROM files WHERE path = ?1", &[&rel_path], |r| r.get(0))
    }
    pub fn all_file_paths(&self) -> Result<Vec<String>> {
        query_cached_map(&self.conn, "SELECT path FROM files ORDER BY path", [], |r| r.get(0))
    }

    pub fn status(&self) -> Result<IndexStatus> {
        let (fc, lc, sc, cc, ic, sec): (i64, i64, i64, i64, i64, i64) = self.conn.query_row(
            "SELECT (SELECT COUNT(*) FROM files),(SELECT COUNT(*) FROM lines),
                    (SELECT COUNT(*) FROM symbols),(SELECT COUNT(*) FROM callers),
                    (SELECT COUNT(*) FROM imports),(SELECT COUNT(*) FROM semantic_chunks)",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )?;
        Ok(IndexStatus {
            root: self.root.display().to_string(),
            index_path: self.db_path.display().to_string(),
            file_count: fc as usize, line_count: lc as usize, symbol_count: sc as usize,
            caller_count: cc as usize, import_count: ic as usize, semantic_chunk_count: sec as usize,
            embed_backend: self.get_meta("embed_backend")?,
            embed_dim: self.get_meta("embed_dim")?.and_then(|d| d.parse().ok()),
            semantic_ivf_present: crate::semantic_ivf::semantic_ivf_path(&self.db_path).exists(),
        })
    }

    pub fn indexed_line_count(&self) -> Result<usize> { count_star(&self.conn, "lines") }

    pub fn all_indexed_lines(&self) -> Result<Vec<IndexedLineRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.path, l.line_no, l.content, f.language FROM lines l JOIN files f ON f.id = l.file_id ORDER BY f.path, l.line_no",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        let mut last_path: Option<Arc<str>> = None;
        let mut last_lang: Option<Arc<str>> = None;
        while let Some(row) = rows.next()? {
            let path = row.get_ref(0)?.as_str().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(e))
            })?;
            if last_path.as_deref() != Some(path) {
                last_path = Some(Arc::from(path));
                last_lang = match row.get_ref(3)? {
                    ValueRef::Null => None,
                    v => Some(Arc::from(v.as_str().map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(e))
                    })?)),
                };
            }
            out.push((Arc::clone(last_path.as_ref().expect("path")), row.get(1)?, row.get(2)?, last_lang.clone()));
        }
        Ok(out)
    }

    pub fn semantic_chunk_max_id(&self) -> Result<Option<i64>> {
        optional_row(&self.conn, "SELECT MAX(id) FROM semantic_chunks", &[], |r| r.get::<_, Option<i64>>(0))
            .map(Option::flatten)
    }

    pub fn semantic_chunk_stats(&self, lang: Option<&str>) -> Result<SemanticChunkStats> {
        let max_id = self.semantic_chunk_max_id()?.unwrap_or(0);
        let (count, dim): (i64, i64) = if let Some(l) = lang {
            self.conn.query_row(
                "SELECT COUNT(*), COALESCE(MAX(length(sc.vector)/4),0) FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE f.language=?1",
                params![l], |r| Ok((r.get(0)?, r.get(1)?)),
            )?
        } else {
            self.conn.query_row(
                "SELECT COUNT(*), COALESCE(MAX(length(vector)/4),0) FROM semantic_chunks",
                [], |r| Ok((r.get(0)?, r.get(1)?)),
            )?
        };
        Ok(SemanticChunkStats { count: count as usize, max_id, dim: dim as usize })
    }

    pub fn semantic_chunk_ids(&self, lang: Option<&str>) -> Result<Vec<i64>> {
        let (sql, l) = if lang.is_some() {
            ("SELECT sc.id FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE f.language=?1 ORDER BY sc.id", lang)
        } else {
            ("SELECT id FROM semantic_chunks ORDER BY id", None)
        };
        query_map_rows(&self.conn, sql, l, |r| r.get(0))
    }

    pub fn semantic_chunks_by_ids(
        &self, ids: &[i64],
    ) -> Result<Vec<(i64, ast_sgrep_embed::SemanticChunkRow)>> {
        let mut out = Vec::with_capacity(ids.len());
        for batch in ids.chunks(500) {
            let ph = std::iter::repeat_n("?", batch.len()).collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT sc.id, f.path, sc.line_start, sc.line_end, sc.symbol_name, sc.text, sc.vector
                 FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE sc.id IN ({ph})"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(batch.iter()), |r| {
                let id = r.get(0)?;
                let v: Vec<u8> = r.get(6)?;
                Ok((id, (
                    r.get(1)?, r.get(2)?, r.get(3)?,
                    r.get::<_, Option<String>>(4)?.unwrap_or_default(), r.get(5)?,
                    ast_sgrep_embed::embed_from_bytes(&v).unwrap_or_default(),
                )))
            })?;
            out.extend(rows.collect::<std::result::Result<Vec<_>, _>>()?);
        }
        Ok(out)
    }

    pub fn all_semantic_chunks(
        &self, lang: Option<&str>,
    ) -> Result<Vec<ast_sgrep_embed::SemanticChunkRow>> {
        let c = if lang.is_some() { " AND f.language = ?1" } else { "" };
        let sql = format!(
            "SELECT f.path, sc.line_start, sc.line_end, sc.symbol_name, sc.text, sc.vector
             FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE 1=1{c} ORDER BY sc.id"
        );
        query_map_rows(&self.conn, &sql, lang, read_semantic_chunk_row)
    }

    pub fn symbols_in_file(&self, rel_path: &str) -> Result<Vec<SymbolRow>> {
        query_cached_map(
            &self.conn,
            "SELECT s.name, s.kind, s.line_start, s.line_end, s.byte_start, s.byte_end
             FROM symbols s JOIN files f ON f.id=s.file_id WHERE f.path=?1 ORDER BY s.line_start",
            params![rel_path],
            |r| Ok(SymbolRow {
                name: r.get(0)?, kind: r.get(1)?, line_start: r.get(2)?, line_end: r.get(3)?,
                byte_start: r.get::<_, i64>(4)? as usize, byte_end: r.get::<_, i64>(5)? as usize,
            }),
        )
    }

    pub fn incoming_calls(&self, callee: &str) -> Result<Vec<CallRow>> { calls_matching(&self.conn, "callee", callee) }
    pub fn outgoing_calls(&self, caller: &str) -> Result<Vec<CallRow>> { calls_matching(&self.conn, "caller", caller) }

    pub fn symbol_at_line(&self, path: &str, line: u32) -> Result<Option<SymbolLocationRow>> {
        let sql = format!(
            "{SYM_LOC} WHERE f.path=?1 AND s.line_start<=?2 AND s.line_end>=?2
             ORDER BY (s.line_end-s.line_start), s.line_start DESC, s.name LIMIT 1"
        );
        optional_row(&self.conn, &sql, &[&path, &line], read_sym_loc)
    }
    pub fn first_symbol_in_file(&self, path: &str) -> Result<Option<SymbolLocationRow>> {
        let sql = format!("{SYM_LOC} WHERE f.path=?1 ORDER BY s.line_start, s.line_end, s.name LIMIT 1");
        optional_row(&self.conn, &sql, &[&path], read_sym_loc)
    }
    pub fn symbols_named(&self, name: &str, limit: usize) -> Result<Vec<SymbolLocationRow>> {
        let sql = format!("{SYM_LOC} WHERE s.name=?1 ORDER BY f.path, s.line_start, s.line_end LIMIT ?2");
        query_cached_map(&self.conn, &sql, params![name, limit as i64], read_sym_loc)
    }

    pub fn imports_from_file(&self, path: &str) -> Result<Vec<ImportRow>> {
        query_cached_map(
            &self.conn,
            "SELECT i.module_path, i.line_no FROM imports i JOIN files f ON f.id=i.file_id
             WHERE f.path=?1 ORDER BY i.line_no, i.module_path",
            params![path],
            |r| Ok(ImportRow { module_path: r.get(0)?, line_no: r.get(1)? }),
        )
    }

    pub fn resolve_module_path(&self, from_file: &str, module: &str) -> Result<Vec<String>> {
        let module = module.trim().trim_matches(['"', '\'']);
        if module.is_empty() { return Ok(Vec::new()); }
        let parent = Path::new(from_file).parent().unwrap_or_else(|| Path::new(""));
        let crate_src = from_file.find("/src/").map(|i| Path::new(&from_file[..i + 4]));
        let slash = module.replace("::", "/");
        let mut bases = Vec::new();
        if let Some(rest) = slash.strip_prefix("crate/") {
            if let Some(src) = crate_src { bases.push(src.join(rest)); }
        } else if slash == "crate" {
            if let Some(src) = crate_src { bases.push(src.to_path_buf()); }
        } else if slash.starts_with("super/") || slash.starts_with("self/") {
            let mut base = parent.to_path_buf();
            let mut rest = slash.as_str();
            while let Some(n) = rest.strip_prefix("super/") { base.pop(); rest = n; }
            rest = rest.strip_prefix("self/").unwrap_or(rest);
            bases.push(base.join(rest));
        } else if module.starts_with('.') {
            bases.push(parent.join(module));
        } else {
            bases.push(parent.join(&slash));
            if let Some(src) = crate_src { bases.push(src.join(&slash)); }
        }
        const EXTS: &[&str] = &["rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "cs", "rb"];
        let mut cands = BTreeSet::new();
        for base in bases {
            let n = normalize_rel(&base);
            cands.insert(n.clone());
            if base.extension().is_none() {
                for e in EXTS { cands.insert(format!("{n}.{e}")); }
                cands.insert(format!("{n}/mod.rs"));
                for e in ["ts", "tsx", "js", "jsx"] { cands.insert(format!("{n}/index.{e}")); }
            }
        }
        let mut out = Vec::new();
        for c in cands {
            if optional_row(&self.conn, "SELECT path FROM files WHERE path=?1", &[&c], |r| r.get::<_, String>(0))?
                .is_some()
            {
                out.push(c);
            }
        }
        Ok(out)
    }

    pub fn all_symbols(&self) -> Result<Vec<SymbolFileRow>> {
        query_cached_map(&self.conn, &format!("{SYM_FILE} ORDER BY f.path, s.line_start"), [], read_sym_file)
    }
    pub fn symbols_matching(&self, name: &str, limit: usize) -> Result<Vec<SymbolFileRow>> {
        let sql = format!(
            "{SYM_FILE} WHERE lower(s.name) LIKE '%' || lower(?1) || '%' ESCAPE '\\'
             ORDER BY f.path, s.line_start LIMIT ?2"
        );
        query_cached_map(&self.conn, &sql, params![name, limit as i64], read_sym_file)
    }

    pub fn files_importing_module(&self, module: &str, limit: usize) -> Result<Vec<ImportQueryRow>> {
        let sql = format!(
            "{IMPORT_SELECT} WHERE lower(i.module_path) LIKE '%' || lower(?1) || '%' ESCAPE '\\'
             ORDER BY f.path, i.line_no LIMIT ?2"
        );
        query_cached_map(&self.conn, &sql, params![module, limit as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
    }

    pub fn file_language(&self, path: &str) -> Result<Option<String>> {
        self.conn.prepare_cached("SELECT language FROM files WHERE path=?1")?
            .query_row(params![path], |r| r.get(0)).optional().map_err(Into::into)
    }
    pub fn pattern_node_count(&self) -> Result<usize> { count_star(&self.conn, "pattern_nodes") }

    pub fn pattern_nodes_matching(
        &self, signature: &str, lang: Option<&str>,
    ) -> Result<Vec<PatternNodeRow>> {
        let mut sql = String::from(
            "SELECT f.path, f.language, n.line_start, n.line_end, n.excerpt
             FROM pattern_nodes n JOIN files f ON f.id=n.file_id WHERE n.signature=?1",
        );
        if lang.is_some() { sql.push_str(" AND f.language=?2"); }
        sql.push_str(" ORDER BY f.path, n.line_start");
        let map = |r: &rusqlite::Row<'_>| Ok(PatternNodeRow {
            path: r.get(0)?, language: r.get(1)?, line_start: r.get(2)?, line_end: r.get(3)?, excerpt: r.get(4)?,
        });
        match lang {
            Some(l) => query_cached_map(&self.conn, &sql, params![signature, l], map),
            None => query_cached_map(&self.conn, &sql, params![signature], map),
        }
    }

    pub fn file_text(&self, path: &str) -> Result<Option<String>> {
        let lines = self.file_lines(path)?;
        if lines.is_empty() { return Ok(None); }
        let sep = match self.get_meta(&format!("eol:{path}"))? {
            Some(v) if v == "crlf" => "\r\n",
            _ => "\n",
        };
        Ok(Some(lines.iter().map(|(_, c)| c.as_str()).collect::<Vec<_>>().join(sep)))
    }
    pub fn file_lines(&self, path: &str) -> Result<Vec<(u32, String)>> {
        query_cached_map(
            &self.conn,
            "SELECT l.line_no, l.content FROM lines l JOIN files f ON f.id=l.file_id WHERE f.path=?1 ORDER BY l.line_no",
            params![path], |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }
    pub fn line_content(&self, path: &str, line: u32) -> Result<Option<String>> {
        optional_row(
            &self.conn,
            "SELECT l.content FROM lines l JOIN files f ON f.id=l.file_id WHERE f.path=?1 AND l.line_no=?2",
            &[&path, &line], |r| r.get(0),
        )
    }

    pub fn query_imports(
        &self, module: Option<&str>, lang: Option<&str>, limit: usize,
    ) -> Result<Vec<ImportQueryRow>> {
        let map = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?));
        if module.is_none_or(|m| m.is_empty()) {
            let mut parts = Vec::new();
            let mut bind = Vec::new();
            super::sql::append_lang_filter(&mut parts, &mut bind, lang);
            let w = super::sql::where_clause(&parts);
            let sql = format!("{IMPORT_SELECT}{w} LIMIT ?{}", bind.len() + 1);
            return query_limit_map(&self.conn, &sql, bind, limit, map);
        }
        let m = module.unwrap().to_string();
        let (w, bind) = like_terms_filter("i.module_path", &[m], lang);
        let sql = format!("{IMPORT_SELECT}{w} LIMIT ?{}", bind.len() + 1);
        query_limit_map(&self.conn, &sql, bind, limit, map)
    }

    pub fn all_legacy_embeddings(
        &self, lang: Option<&str>,
    ) -> Result<Vec<ast_sgrep_embed::SemanticChunkRow>> {
        let c = if lang.is_some() { " AND f.language = ?1" } else { "" };
        let sql = format!(
            "SELECT f.path, l.line_no, l.content, sc.symbol_name, e.vector FROM embeddings e
             JOIN lines l ON l.file_id=e.file_id AND l.line_no=e.line_no
             JOIN files f ON f.id=e.file_id
             LEFT JOIN semantic_chunks sc ON sc.file_id=f.id AND sc.line_start=l.line_no
             WHERE 1=1{c} LIMIT 5000"
        );
        query_map_rows(&self.conn, &sql, lang, read_legacy_emb)
    }

    pub fn file_exists(&self, path: &str) -> Result<bool> {
        Ok(self.conn.prepare_cached("SELECT 1 FROM files WHERE path=?1")?.exists(params![path])?)
    }

    fn embed_chunks(
        &self, chunks: &[crate::semantic_chunk::SemanticChunkInput],
        do_embed: bool, backend: ast_sgrep_embed::EmbedPreference,
    ) -> Result<EmbeddedChunks> {
        if !do_embed || chunks.is_empty() { return Ok(EmbeddedChunks::empty()); }
        let mid = cache_model_id_for_pref(backend);
        let (chunks, cache_entries, cache_hits) = self.embed_parallel(chunks, backend, &mid)?;
        Ok(EmbeddedChunks { chunks, cache_entries, cache_hits })
    }

    fn embed_parallel(
        &self, chunks: &[crate::semantic_chunk::SemanticChunkInput],
        backend: ast_sgrep_embed::EmbedPreference, expected_mid: &Option<String>,
    ) -> Result<(Vec<EmbeddedChunk>, Vec<CacheEntry>, Vec<CacheHit>)> {
        let texts: Vec<String> = chunks.iter().map(crate::semantic_chunk::render_chunk_text).collect();
        let mut cached: Vec<Option<CacheRow>> = vec![None; texts.len()];
        let mut hits = Vec::new();
        for (i, t) in texts.iter().enumerate() {
            let h = hash_text(t);
            if let Some(mid) = expected_mid {
                if let Some(row) = self.lookup_embed_cache(&h, mid)? {
                    cached[i] = Some(row);
                    hits.push(CacheHit { chunk_hash: h, model_id: mid.clone() });
                }
            }
        }
        if hits.len() == texts.len() {
            let out = texts.into_iter().zip(cached).map(|(text, row)| {
                let row = row.expect("hit");
                EmbeddedChunk { text, vector_bytes: row.vector, dim: row.dim, backend: row.backend }
            }).collect();
            return Ok((out, Vec::new(), hits));
        }
        let miss_idx: Vec<usize> = texts
            .iter()
            .enumerate()
            .filter(|(i, _)| cached[*i].is_none())
            .map(|(i, _)| i)
            .collect();
        // One chain attempt for the whole miss batch (avoids per-chunk backend probing).
        let miss_refs: Vec<&str> = miss_idx.iter().map(|&i| texts[i].as_str()).collect();
        let miss_res = ast_sgrep_embed::embed_batch_with_chain(&miss_refs, backend);
        if miss_res.len() != miss_idx.len() {
            return Err(crate::StoreError::Other("embedding result length mismatch".into()));
        }
        let mut out = Vec::with_capacity(texts.len());
        let mut entries = Vec::with_capacity(miss_res.len());
        let mut miss_it = miss_res.into_iter();
        for (i, text) in texts.into_iter().enumerate() {
            if let Some(row) = cached[i].take() {
                out.push(EmbeddedChunk {
                    text,
                    vector_bytes: row.vector,
                    dim: row.dim,
                    backend: row.backend,
                });
                continue;
            }
            let r = miss_it
                .next()
                .ok_or_else(|| crate::StoreError::Other("embedding result length mismatch".into()))?;
            let vb = ast_sgrep_embed::embed_to_bytes(&r.vector);
            let dim = r.vector.len();
            if let Some(mid) = cache_model_id_for_backend(r.backend) {
                entries.push(CacheEntry {
                    chunk_hash: hash_text(&text),
                    model_id: mid,
                    backend: r.backend,
                    dim,
                    vector: vb.clone(),
                });
            }
            out.push(EmbeddedChunk {
                text,
                vector_bytes: vb,
                dim,
                backend: r.backend,
            });
        }
        Ok((out, entries, hits))
    }
}
struct EmbeddedChunk {
    text: String, vector_bytes: Vec<u8>, dim: usize, backend: ast_sgrep_embed::EmbedBackendKind,
}
#[derive(Clone)]
struct CacheRow { vector: Vec<u8>, backend: ast_sgrep_embed::EmbedBackendKind, dim: usize, }
#[derive(Clone)]
struct CacheEntry {
    chunk_hash: String, model_id: String, backend: ast_sgrep_embed::EmbedBackendKind,
    dim: usize, vector: Vec<u8>,
}
#[derive(Clone)]
struct CacheHit { chunk_hash: String, model_id: String, }
struct EmbeddedChunks {
    chunks: Vec<EmbeddedChunk>, cache_entries: Vec<CacheEntry>, cache_hits: Vec<CacheHit>,
}
impl EmbeddedChunks {
    fn empty() -> Self {
        Self { chunks: Vec::new(), cache_entries: Vec::new(), cache_hits: Vec::new() }
    }
}
fn hash_text(t: &str) -> String {
    let mut h = Hasher::new();
    h.update(t.as_bytes());
    h.finalize().to_hex().to_string()
}
fn embed_cache_cap() -> usize {
    std::env::var("ASGREP_EMBED_CACHE_CAP").ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_EMBED_CACHE_CAP)
}
fn semantic_mid() -> String {
    format!("semantic:hashed-v1:{}", ast_sgrep_embed::default_semantic_dim())
}
fn cache_model_id_for_pref(p: ast_sgrep_embed::EmbedPreference) -> Option<String> {
    use ast_sgrep_embed::EmbedPreference::*;
    match p {
        Semantic => Some(semantic_mid()),
        Neural => Some(format!("neural:{}", ast_sgrep_embed::neural_configured_model_id())),
        Cloud | Ollama => None,
        Auto => {
            let skip = std::env::var_os("ASGREP_EMBED_API_KEY").is_some()
                || std::env::var_os("ASGREP_OLLAMA_EMBED").is_some()
                || std::env::var_os("ASGREP_OLLAMA_URL").is_some()
                || std::env::var("ASGREP_NEURAL_EMBED").is_ok_and(|v| v == "1");
            if skip { None } else { Some(semantic_mid()) }
        }
    }
}
fn cache_model_id_for_backend(b: ast_sgrep_embed::EmbedBackendKind) -> Option<String> {
    use ast_sgrep_embed::EmbedBackendKind::*;
    match b {
        Semantic => Some(semantic_mid()),
        Neural => Some(format!("neural:{}", ast_sgrep_embed::neural_configured_model_id())),
        Cloud => ast_sgrep_embed::CloudEmbeddingConfig::from_env().map(|c| format!("cloud:{}", c.model)),
        Ollama => ast_sgrep_embed::OllamaEmbeddingConfig::from_env().map(|c| format!("ollama:{}", c.model)),
    }
}
fn read_sym_loc(r: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolLocationRow> {
    Ok(SymbolLocationRow { path: r.get(0)?, name: r.get(1)?, language: r.get(2)?, line_start: r.get(3)?, line_end: r.get(4)? })
}
fn read_sym_file(r: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolFileRow> {
    Ok(SymbolFileRow { path: r.get(0)?, language: r.get(1)?, name: r.get(2)?, kind: r.get(3)?, line_start: r.get(4)?, line_end: r.get(5)? })
}
fn normalize_rel(path: &Path) -> String {
    let mut parts = Vec::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => { parts.pop(); }
            Component::Normal(p) => parts.push(p.to_string_lossy().into_owned()),
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    parts.join("/")
}
fn delete_file_children(conn: &Connection, file_id: i64) -> Result<()> {
    // Collect line rowids first; both FTS tables are keyed by lines.rowid.
    let rowids: Vec<i64> = conn
        .prepare_cached("SELECT rowid FROM lines WHERE file_id = ?1")?
        .query_map(params![file_id], |r| r.get::<_, i64>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    // Delete both FTS indexes by rowid before removing their source lines.
    for chunk in rowids.chunks(500) {
        let placeholders = std::iter::repeat_n("?", chunk.len()).collect::<Vec<_>>().join(",");
        for table in ["lines_trigram", "lines_fts"] {
            conn.execute(
                &format!("DELETE FROM {table} WHERE rowid IN ({placeholders})"),
                rusqlite::params_from_iter(chunk.iter()),
            )?;
        }
    }

    // Delete the remaining ordinary per-file tables by file_id.
    for t in ["lines", "symbols", "callers", "imports", "pattern_nodes", "embeddings", "semantic_chunks"] {
        conn.prepare_cached(&format!("DELETE FROM {t} WHERE file_id=?1"))?.execute(params![file_id])?;
    }
    Ok(())
}
fn read_legacy_emb(r: &rusqlite::Row<'_>) -> rusqlite::Result<ast_sgrep_embed::SemanticChunkRow> {
    let v: Vec<u8> = r.get(4)?;
    Ok((r.get(0)?, r.get(1)?, r.get(1)?, r.get::<_, Option<String>>(3)?.unwrap_or_default(), r.get(2)?,
        ast_sgrep_embed::embed_from_bytes(&v).unwrap_or_default()))
}
fn read_semantic_chunk_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ast_sgrep_embed::SemanticChunkRow> {
    let v: Vec<u8> = r.get(5)?;
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, Option<String>>(3)?.unwrap_or_default(), r.get(4)?,
        ast_sgrep_embed::embed_from_bytes(&v).unwrap_or_default()))
}
