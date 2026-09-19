use std::path::{Path, PathBuf};
pub fn sample_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/sample")
        .canonicalize()
        .expect("sample fixture")
}
pub fn sample_file(rel: &str) -> String {
    std::fs::read_to_string(sample_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Write `body` to `path`, creating parent directories (`mkdir -p`).
/// Panics on IO failure. Deterministic: fixed bytes in, fixed bytes out.
pub fn write_file(path: &Path, body: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("create parent of {}: {e}", path.display()));
    }
    std::fs::write(path, body).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// Forge a whole-second mtime (`secs` since the Unix epoch, nanos 0) and
/// read it back: asserts the stored time round-trips exactly, so change
/// detection never depends on filesystem timestamp granularity and no
/// wall-clock sleeps are needed. Panics on IO failure or read-back mismatch.
/// Units: whole seconds since `UNIX_EPOCH`.
pub fn set_mtime_secs(path: &Path, secs: u64) {
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap_or_else(|e| panic!("open {} for mtime: {e}", path.display()))
        .set_modified(time)
        .unwrap_or_else(|e| panic!("set mtime of {}: {e}", path.display()));
    let back = std::fs::metadata(path)
        .unwrap_or_else(|e| panic!("stat {}: {e}", path.display()))
        .modified()
        .unwrap_or_else(|e| panic!("mtime of {}: {e}", path.display()));
    assert_eq!(back, time, "mtime read-back for {}", path.display());
}

/// INTENT: byte-exact temp file for capped-read, SCIP-degrade, and mmap
/// facets: the caller keeps the [`tempfile::NamedTempFile`] alive and reads
/// via `.path()` / `.as_file()`. Panics on IO failure.
pub fn write_temp(bytes: &[u8]) -> tempfile::NamedTempFile {
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new().expect("temp file");
    file.write_all(bytes).expect("write");
    file.flush().expect("flush");
    file
}

/// Temp tree with `files` (`(relative path, body)` pairs; parents created).
/// The caller keeps the [`tempfile::TempDir`] alive. Panics on IO failure.
pub fn file_tree(files: &[(&str, &str)]) -> tempfile::TempDir {
    let temp = tempfile::TempDir::new().expect("file_tree tempdir");
    for (rel, body) in files {
        write_file(&temp.path().join(rel), body.as_bytes());
    }
    temp
}

/// INTENT: sorted directory listing (file names, UTF-8 lossy) for
/// fail-closed fs-delta pins: identical trees list identically. Pure
/// projection; panics on IO failure.
pub fn dir_listing(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .expect("read dir")
        .map(|entry| {
            entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// INTENT: the default on-disk state path `<root>/.asgrep/index.db` — the
/// ONE canonical join shared by the CLI and MCP error suites (previously
/// inlined in both). Pure constructor; hermetic by design. Delta vs
/// [`ast_sgrep_core::index_db_path`]: that resolver consults
/// `ASGREP_INDEX_PATH` / `ASGREP_USE_CACHE` and the filesystem, so it cannot
/// pin a hermetic layout; this is the fixed default arm only.
pub fn index_db_path(root: &Path) -> PathBuf {
    root.join(".asgrep").join("index.db")
}
