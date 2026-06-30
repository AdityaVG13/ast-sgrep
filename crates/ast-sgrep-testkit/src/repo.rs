use std::path::PathBuf;

use ast_sgrep_core::{IndexOptions, Indexer};
use tempfile::TempDir;

/// Temporary repository root with pre-written files.
pub struct TempRepo {
    pub _temp: TempDir,
    pub root: PathBuf,
}

/// Create a temp directory and write `files` as `(relative_path, content)` pairs.
pub fn temp_repo(files: &[(&str, &str)]) -> TempRepo {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().to_path_buf();
    for (rel, content) in files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, content).expect("write");
    }
    TempRepo { _temp: temp, root }
}

/// Index a temporary repository.
pub fn index_repo(repo: &TempRepo, mut opts: IndexOptions) -> (TempDir, Indexer) {
    let temp = TempDir::new().expect("tempdir");
    opts.root = repo.root.clone();
    opts.index_path = Some(temp.path().join("index.db"));
    let mut indexer = Indexer::new(opts).expect("indexer");
    indexer.index_all().expect("index");
    (temp, indexer)
}
