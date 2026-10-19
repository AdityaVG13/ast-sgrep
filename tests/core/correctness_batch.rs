//! Hard evidence for PR20 P1 correctness beads: 28vo, kqhp (+ public-API coverage).
use ast_sgrep_core::store::UpsertFileInput;
use ast_sgrep_core::{
    indexed_rel_path, IndexOptions, IndexStore, Indexer, SearchOptions, Searcher,
};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use tempfile::TempDir;

fn base<'a>(path: &'a str, lines: &'a [(u32, String)], hash: &'a str) -> UpsertFileInput<'a> {
    UpsertFileInput {
        rel_path: path,
        language: Some("python"),
        mtime_secs: 1,
        mtime_nanos: 0,
        content_hash: hash,
        lines,
        eol: "\n",
        symbols: &[],
        callers: &[],
        imports: &[],
        pattern_nodes: &[],
        depth_truncated: false,
        semantic_chunks: &[],
        embed_semantic: false,
    }
}

fn write_src(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

/// ast-sgrep-28vo — clear_all_data wipes embed_* fingerprints; keeps schema whitelist.
#[test]
fn clear_all_data_wipes_embed_meta_keeps_root_whitelist() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    store
        .set_meta("root", temp.path().to_string_lossy().as_ref())
        .unwrap();
    let lines = [(1, "print(1)".into())];
    store.upsert_file(base("a.py", &lines, "h1")).unwrap();
    store.set_meta("struct:a.py", "fp").unwrap();
    store.set_meta("body:a.py", "bh").unwrap();
    store.set_meta("embed_backend", "semantic-v2").unwrap();
    store.set_meta("embed_dim", "256").unwrap();
    store.set_meta("embed_model", "x").unwrap();
    store.set_meta("embed_cache_hits", "9").unwrap();
    store.set_meta("embed_cache_misses", "3").unwrap();
    store.clear_all_data().unwrap();
    assert!(store.get_meta("struct:a.py").unwrap().is_none());
    assert!(store.get_meta("body:a.py").unwrap().is_none());
    assert!(store.get_meta("embed_backend").unwrap().is_none());
    assert!(store.get_meta("embed_dim").unwrap().is_none());
    assert!(store.get_meta("embed_model").unwrap().is_none());
    assert!(store.get_meta("embed_cache_hits").unwrap().is_none());
    assert!(store.get_meta("embed_cache_misses").unwrap().is_none());
    assert!(
        store.get_meta("root").unwrap().is_some(),
        "schema whitelist must preserve root"
    );
    // Generations are whitelisted then bumped — still monotonic across clear.
    assert!(store.semantic_data_version().unwrap() >= 1);
}

/// ast-sgrep-28vo — a store embedded by a foreign (retired) model is never
/// treated as unchanged: the next index run rewrites it.
#[test]
fn is_unchanged_rejects_foreign_embed_model() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_src(root, "m.py", "def hello():\n    return 1\n");
    let mut first_indexer = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        embed_semantic: true,
        ..IndexOptions::default()
    })
    .unwrap();
    let first = first_indexer.index_all().unwrap();
    assert!(first.files_indexed >= 1);
    assert_eq!(
        first_indexer
            .store()
            .get_meta("embed_backend")
            .unwrap()
            .as_deref(),
        Some("static")
    );
    // Simulate a store written by a retired engine.
    first_indexer
        .store()
        .set_meta("embed_backend", "semantic-v2")
        .unwrap();
    first_indexer
        .store()
        .set_meta("embed_model", "semantic-v2:256")
        .unwrap();
    drop(first_indexer);

    let mut reopened = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        embed_semantic: true,
        ..IndexOptions::default()
    })
    .unwrap();
    let second = reopened.index_all().unwrap();
    assert!(
        second.files_indexed >= 1,
        "foreign embed identity must force a rewrite, not an unchanged skip; got skipped={}",
        second.files_skipped
    );
}

/// ast-sgrep-kqhp — non-UTF8 rel paths are rejected (no lossy DB key).
#[test]
fn indexed_rel_path_rejects_non_utf8() {
    let bytes = b"bad\x80name.py";
    let rel = Path::new(OsStr::from_bytes(bytes));
    let err = indexed_rel_path(rel).expect_err("non-UTF8 must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("non-UTF8") && msg.contains("asgrep-kqhp"),
        "machine error must name policy: {msg}"
    );
    // Distinct non-UTF8 paths that lossy-collide must each reject (no shared DB key).
    let a = Path::new(OsStr::from_bytes(b"x\x80.yml"));
    let b = Path::new(OsStr::from_bytes(b"x\x81.yml"));
    assert_eq!(a.to_string_lossy(), b.to_string_lossy());
    assert!(indexed_rel_path(a).is_err());
    assert!(indexed_rel_path(b).is_err());
}

/// Path-traversal / absolute keys must not enter the index (ubs security pass).
#[test]
fn indexed_rel_path_rejects_traversal_and_absolute() {
    for bad in [
        Path::new("../secret.rs"),
        Path::new("src/../../etc/passwd"),
        Path::new("/etc/passwd"),
        Path::new(""),
        Path::new("a\0b.rs"),
    ] {
        let err = indexed_rel_path(bad).expect_err("must reject unsafe rel");
        let msg = err.to_string();
        assert!(
            msg.contains("asgrep-kqhp"),
            "policy tag missing for {}: {msg}",
            bad.display()
        );
    }
    assert_eq!(
        indexed_rel_path(Path::new("src/main.rs")).unwrap(),
        "src/main.rs"
    );
    assert_eq!(
        indexed_rel_path(Path::new("./src/lib.rs")).unwrap(),
        "./src/lib.rs"
    );
}

#[cfg(unix)]
#[test]
fn indexed_rel_path_does_not_rewrite_unix_backslashes_into_separators() {
    assert_eq!(
        indexed_rel_path(Path::new("dir\\file.rs")).unwrap(),
        "dir\\file.rs"
    );
    assert_eq!(
        indexed_rel_path(Path::new("..\\escape.rs")).unwrap(),
        "..\\escape.rs"
    );
}

#[test]
fn index_content_rejects_parent_dir_keys() {
    let temp = TempDir::new().unwrap();
    let mut indexer = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    let err = indexer
        .index_content("../escape.rs", "fn evil() {}")
        .expect_err("parent-dir key must fail closed");
    assert!(
        err.to_string().contains("path traversal") || err.to_string().contains("asgrep-kqhp"),
        "got {}",
        err
    );
}

/// Prior durability: ResponseCache still invalidates on same-connection generation bump.
#[test]
fn prior_response_cache_invalidation_still_green() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let options = SearchOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(store.db_path().to_path_buf()),
        use_embed: false,
        ..SearchOptions::default()
    };
    let searcher = Searcher::with_store(store, options);
    let lines_a = [(1, "alpha sentinel".into())];
    searcher
        .store()
        .upsert_file(base("same.py", &lines_a, "a"))
        .unwrap();
    assert!(!searcher.search("alpha").unwrap().hits.is_empty());
    let lines_b = [(1, "beta sentinel".into())];
    searcher
        .store()
        .upsert_file(base("same.py", &lines_b, "b"))
        .unwrap();
    assert!(searcher.search("alpha").unwrap().hits.is_empty());
}
