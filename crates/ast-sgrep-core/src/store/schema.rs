use rusqlite::{params, Connection};

use crate::Result;

pub const SCHEMA_SQL: &str = "
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
            ";

pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

pub fn delete_file_children(conn: &Connection, file_id: i64) -> Result<()> {
    for table in [
        "lines",
        "lines_fts",
        "symbols",
        "callers",
        "imports",
        "embeddings",
        "semantic_chunks",
    ] {
        conn.execute(
            &format!("DELETE FROM {table} WHERE file_id = ?1"),
            params![file_id],
        )?;
    }
    Ok(())
}
