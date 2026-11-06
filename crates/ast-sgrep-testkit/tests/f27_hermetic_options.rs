//! F27 regression: every hermetic `IndexOptions` / `SearchOptions` construction
//! site routes through `hermetic_index_options` / `hermetic_search_options`.
//!
//! Fails before the fix (missing constructors); passes after, with the
//! hermetic triple (explicit `index_path`, no tantivy, no embed) pinned on the
//! constructors and on the existing `core_*` builders, plus a real
//! index-then-search round trip through the new constructors.

use ast_sgrep_testkit::{
    build_and_quiet, core_index_options, core_search_options, corpus_session,
    hermetic_index_options, hermetic_search_options, search_parity_key,
};
use ast_sgrep_core::{Indexer, Searcher};
use std::path::PathBuf;

fn paths() -> (PathBuf, PathBuf) {
    (
        PathBuf::from("/tmp/testkit-f27-root"),
        PathBuf::from("/tmp/testkit-f27-root/index.db"),
    )
}

#[test]
fn hermetic_index_options_pins_explicit_path_no_tantivy_no_embed() {
    let (root, db) = paths();
    let opts = hermetic_index_options(&root, &db);
    assert_eq!(opts.root, root);
    assert_eq!(opts.index_path, Some(db));
    assert!(!opts.use_tantivy, "hermetic means no tantivy sidecar");
    assert!(!opts.embed_semantic, "hermetic means no embed");
}

#[test]
fn hermetic_search_options_pins_limit_and_no_embed_channel() {
    let (root, db) = paths();
    let opts = hermetic_search_options(&root, &db, 16);
    assert_eq!(opts.root, root);
    assert_eq!(opts.index_path, Some(db));
    assert_eq!(opts.limit, 16);
    assert!(!opts.use_embed, "hermetic means lexical only");
    assert!(!opts.use_tantivy, "hermetic means no tantivy sidecar");
}

#[test]
fn core_option_builders_delegate_to_the_hermetic_pair() {
    let (root, db) = paths();
    let index_opts = core_index_options(&root, &db);
    assert!(index_opts.force_reindex, "core build leg stays force-on");
    assert_eq!(index_opts.index_path, Some(db.clone()));
    assert!(!index_opts.use_tantivy);
    assert!(!index_opts.embed_semantic);

    let search_opts = core_search_options(&root, &db, 16);
    assert_eq!(search_opts.limit, 16);
    assert_eq!(search_opts.index_path, Some(db));
    assert!(!search_opts.use_embed);
    assert!(!search_opts.use_tantivy);
}

#[test]
fn hermetic_constructors_drive_a_real_index_then_search() {
    let corpus = tempfile::tempdir().expect("corpus");
    std::fs::create_dir_all(corpus.path().join("src")).expect("mkdirs");
    std::fs::write(
        corpus.path().join("src/lib.rs"),
        "fn hermetic_probe_token() {}\n",
    )
    .expect("write");
    let index = tempfile::tempdir().expect("index");
    let db = index.path().join("index.db");

    let mut indexer = Indexer::new(hermetic_index_options(corpus.path(), &db)).expect("indexer");
    indexer.index_all().expect("index_all");
    drop(indexer);

    let searcher =
        Searcher::new(hermetic_search_options(corpus.path(), &db, 8)).expect("searcher");
    let hits = searcher.search("hermetic_probe_token").expect("search").hits;
    assert_eq!(hits.len(), 1, "round trip must serve the planted token");
    assert_eq!(hits[0].file, "src/lib.rs");
}

#[test]
fn recovery_builders_stay_hermetic_end_to_end() {
    let session = corpus_session();
    build_and_quiet(&session.corpus_root, &session.index_path, false, false);
    let keys = search_parity_key(
        &session.corpus_root,
        &session.index_path,
        "alpha_needle",
        false,
    );
    assert!(
        !keys.is_empty(),
        "hermetic builders must serve the seeded corpus, not an empty parity set"
    );
}
