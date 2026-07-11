use std::time::Duration;
use rusqlite::Connection;
use crate::Result;
pub fn configure_connection(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.set_prepared_statement_cache_capacity(128);

    let journal_mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    }

    conn.execute_batch(
        "PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL; PRAGMA wal_autocheckpoint = 1000;",
    )?;
    if std::env::var_os("ASGREP_SQLITE_DEFAULTS").is_none() {
        conn.execute_batch("PRAGMA mmap_size = 268435456; PRAGMA cache_size = -16384;")?;
    }
    Ok(())
}
pub fn integrity_check(conn: &Connection) -> Result<String> {
    conn.query_row("PRAGMA integrity_check", [], |row| row.get(0)).map_err(Into::into)
}
