//! File-fault builders and error extraction for recovery / error-API suites.
//!
//! # Contract
//!
//! - Fault builders are deterministic: fixed sentinel bytes, fixed offsets,
//!   no randomness, no wall-clock. The same builder calls produce byte-identical
//!   fixtures on every run and platform.
//! - Builders panic (never `Result`) on IO failure: an unplantable fault is a
//!   test failure, not a fallible op. [`flip_bytes`] panics on out-of-bounds
//!   ranges; [`truncate_file`] panics when the target does not exist.
//! - [`remove_sqlite_sidecars`] is the only non-panicking builder: missing
//!   sidecars are ignored (`let _ =`), since WAL presence is state-dependent.
//! - [`err_of`] panics on `Ok`: it pins the fail-closed direction, so an
//!   unexpected success is the failure under test.

use ast_sgrep_core::StoreError;
use rusqlite::ErrorCode;
use std::ffi::OsString;
use std::path::Path;

/// Deterministic non-SQLite payload: a text sentinel repeated 128x (~3.5KB).
/// Fails SQLite header recognition on the first page; byte-identical every run.
fn garbage_bytes() -> Vec<u8> {
    b"ast-sgrep-testkit-corrupt-sentinel;".repeat(128)
}

/// Overwrite (or create) `path` with deterministic non-SQLite bytes.
/// Returns the bytes written for post-drill comparison. Panics on IO failure.
pub fn write_garbage(path: &Path) -> Vec<u8> {
    let garbage = garbage_bytes();
    std::fs::write(path, &garbage).unwrap_or_else(|e| {
        panic!("write garbage to {}: {e}", path.display());
    });
    garbage
}

/// Truncate `path` to `len` bytes (torn-file fault). Panics when the file
/// does not exist or the length cannot be set. Units: bytes.
pub fn truncate_file(path: &Path, len: u64) {
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap_or_else(|e| panic!("open {} for truncation: {e}", path.display()))
        .set_len(len)
        .unwrap_or_else(|e| panic!("truncate {} to {len}: {e}", path.display()));
}

/// XOR `len` bytes with `0xFF` starting at `offset` (torn-byte fault).
/// Operates on an in-memory buffer; callers read the file, flip, write back.
/// Panics when `offset + len` exceeds the buffer. Units: bytes.
pub fn flip_bytes(bytes: &mut [u8], offset: usize, len: usize) {
    assert!(
        offset.saturating_add(len) <= bytes.len(),
        "flip range {offset}+{len} exceeds {} bytes",
        bytes.len()
    );
    for byte in &mut bytes[offset..offset + len] {
        *byte ^= 0xFF;
    }
}

/// Remove SQLite sidecars (`-wal`, `-shm`, `-journal`) next to `db`, keeping
/// `db` itself. Missing sidecars are ignored. Callers composing a total-corrupt
/// fault pair this with [`write_garbage`].
pub fn remove_sqlite_sidecars(db: &Path) {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar: OsString = db.as_os_str().to_owned();
        sidecar.push(suffix);
        let _ = std::fs::remove_file(sidecar);
    }
}

/// Extract the [`StoreError`] from a `Result` that must fail closed.
/// Panics on `Ok` — the unexpected success is the failure under test.
pub fn err_of<T>(result: Result<T, StoreError>) -> StoreError {
    match result {
        Ok(_) => panic!("expected Err, got Ok"),
        Err(err) => err,
    }
}

/// INTENT: total `StoreError` discriminant projection (0 = Database,
/// 1 = Io, 2 = Other): the cross-cutting error-comparison primitive —
/// equal discriminants mean the same failure layer reached the caller.
/// Message text never participates. Pure projection.
pub fn store_error_discriminant(err: &StoreError) -> u8 {
    match err {
        StoreError::Database(_) => 0,
        StoreError::Io(_) => 1,
        StoreError::Other(_) => 2,
    }
}

/// INTENT: sqlite [`ErrorCode`] carried by a `Database` error, if any —
/// the kind projection beneath the discriminant. Pure projection.
pub fn sqlite_code(err: &StoreError) -> Option<ErrorCode> {
    match err {
        StoreError::Database(rusqlite::Error::SqliteFailure(code, _)) => Some(code.code),
        _ => None,
    }
}

/// INTENT: caller-side replica of the crate-private
/// `StoreError::is_corrupt_database` predicate (`DatabaseCorrupt` |
/// `NotADatabase` under the `Database` discriminant). It MUST stay a
/// replica: the predicate is `pub(crate)` in `ast-sgrep-core`, unnameable
/// from any test target, so suites prove corruption stays detectable
/// through the public `Database` discriminant + rusqlite kind alone, at
/// every layer. Keep the match arms in sync with core by hand. Pure
/// projection.
pub fn is_corrupt_kind(err: &StoreError) -> bool {
    matches!(
        err,
        StoreError::Database(rusqlite::Error::SqliteFailure(code, _))
            if matches!(
                code.code,
                ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase
            )
    )
}

/// INTENT: WAL-total corruption fault — deterministic garbage over `db`
/// plus removal of every sqlite sidecar (`-wal`, `-shm`, `-journal`), so
/// no page survives anywhere. Returns the bytes written for post-drill
/// comparison. Delta vs [`crate::corrupt_index_db`]: that takes a
/// workspace root and leaves sidecars behind, so it cannot express total
/// corruption; this takes the db path itself. Panics on IO failure.
pub fn corrupt_db_total(db: &Path) -> Vec<u8> {
    let bytes = write_garbage(db);
    remove_sqlite_sidecars(db);
    bytes
}
