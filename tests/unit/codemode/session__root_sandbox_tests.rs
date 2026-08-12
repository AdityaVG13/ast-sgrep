use super::*;
use tempfile::TempDir;

#[test]
fn foreign_root_is_rejected_under_session_workspace() {
    let workspace = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let root = workspace.path().canonicalize().unwrap();
    std::fs::write(root.join("ok.rs"), "fn ok() {}\n").unwrap();
    let index_path = root.join("index.db");
    {
        let mut indexer = Indexer::new(IndexOptions {
            root: root.clone(),
            index_path: Some(index_path.clone()),
            embed_semantic: false,
            ..IndexOptions::default()
        })
        .expect("indexer");
        indexer.index_all().expect("seed index");
    }
    let before = std::fs::metadata(&index_path).expect("seeded index").len();

    let mut session = CodeModeSession::new(SessionConfig {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        limit: 8,
        use_embed: false,
        ..SessionConfig::default()
    });

    let foreign = outside.path().canonicalize().unwrap();
    std::fs::write(foreign.join("evil.rs"), "fn evil() {}\n").unwrap();
    let err = session
        .index_repo(&json!({ "root": foreign.to_string_lossy() }))
        .expect_err("foreign root must be refused");
    assert!(
        err.to_string().contains("outside")
            || err.to_string().contains("escapes")
            || err.to_string().contains("configured"),
        "unexpected error: {err}"
    );
    let after = std::fs::metadata(&index_path)
        .expect("index must remain")
        .len();
    assert_eq!(before, after, "foreign root must not rewrite pinned index");
}
