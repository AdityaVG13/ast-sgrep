use std::path::PathBuf;

/// Canonical polyglot sample repo used across integration tests.
pub fn sample_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/sample")
        .canonicalize()
        .expect("sample fixture")
}

/// Load a file from the sample fixture by relative path.
pub fn sample_file(rel: &str) -> String {
    let path = sample_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Resolve a path relative to the sample fixture.
pub fn sample_path(rel: &str) -> PathBuf {
    sample_root().join(rel)
}
