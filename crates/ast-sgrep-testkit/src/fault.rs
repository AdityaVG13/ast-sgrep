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
