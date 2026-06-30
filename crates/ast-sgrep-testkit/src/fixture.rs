use std::path::PathBuf;

pub fn sample_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/sample")
        .canonicalize()
        .expect("sample fixture")
}

pub fn sample_file(rel: &str) -> String {
    std::fs::read_to_string(sample_root().join(rel))
        .unwrap_or_else(|e| panic!("read {rel}: {e}"))
}
