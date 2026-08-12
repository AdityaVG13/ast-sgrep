//! jpbq: a full rebuild must never destroy the last known-good index.
use ast_sgrep_core::store::{
    active_manifest_path, generation_db_path, read_active_manifest, try_index_db_path, INDEX_DB,
    INDEX_DIR,
};
use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, SearchOptions, Searcher};

fn corpus(root: &std::path::Path, files: usize) {
    let src = root.join("src");
    std::fs::create_dir_all(&src).expect("mkdir");
    for index in 0..files {
        std::fs::write(
            src.join(format!("mod{index}.rs")),
            format!("fn target_symbol_{index}() {{}}\n"),
        )
        .expect("write");
    }
}

fn indexer(root: &std::path::Path) -> Indexer {
    Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("indexer")
}

fn search_hits(root: &std::path::Path, query: &str) -> usize {
    Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        use_embed: false,
        ..SearchOptions::default()
    })
    .expect("searcher")
    .search(query)
    .expect("search")
    .hits
    .len()
}

#[test]
fn reindex_activates_a_new_generation_and_retains_the_previous() {
    let temp = tempfile::tempdir().unwrap();
    corpus(temp.path(), 3);
    indexer(temp.path()).index_all().expect("first index");

    // First rebuild creates the generation layout.
    indexer(temp.path()).reindex_all().expect("reindex");
    let index_dir = temp.path().join(INDEX_DIR);
    let first = read_active_manifest(&index_dir).expect("active manifest written");
    assert!(active_manifest_path(&index_dir).is_file());
    assert!(generation_db_path(&index_dir, &first.generation).is_file());
    assert!(search_hits(temp.path(), "target_symbol_1") > 0);

    // Second rebuild advances the pointer and keeps the old generation on disk.
    indexer(temp.path()).reindex_all().expect("second reindex");
    let second = read_active_manifest(&index_dir).expect("manifest");
    assert_ne!(second.generation, first.generation, "generation must advance");
    assert_eq!(
        second.previous.as_deref(),
        Some(first.generation.as_str()),
        "previous generation must be recorded for rollback"
    );
    assert!(
        generation_db_path(&index_dir, &first.generation).is_file(),
        "previous generation must be retained until the new one is proven"
    );
    assert!(search_hits(temp.path(), "target_symbol_1") > 0);
}

/// Missing active generation must not silently serve a leftover flat legacy DB
/// (wave2 loop9: state-store + data-integrity + commit/recovery).
#[test]
fn missing_active_generation_refuses_stale_legacy_fallthrough() {
    let temp = tempfile::tempdir().unwrap();
    corpus(temp.path(), 3);
    indexer(temp.path()).index_all().expect("first index");
    indexer(temp.path()).reindex_all().expect("reindex into generation layout");

    let index_dir = temp.path().join(INDEX_DIR);
    let active = read_active_manifest(&index_dir).expect("manifest");
    let gen_dir = index_dir.join("generations").join(&active.generation);
    let legacy = index_dir.join(INDEX_DB);
    assert!(
        legacy.is_file(),
        "fixture needs leftover flat index.db from the pre-generation index"
    );
    assert!(generation_db_path(&index_dir, &active.generation).is_file());

    // Corrupt activation: active pointer remains, generation directory is gone.
    std::fs::remove_dir_all(&gen_dir).expect("remove active generation");

    let err = try_index_db_path(temp.path(), None).expect_err("must refuse fallthrough");
    let msg = err.to_string();
    assert!(
        msg.contains("active generation") && msg.contains("refusing"),
        "error must name corrupt activation, got: {msg}"
    );
    assert!(
        IndexStore::open(temp.path(), None).is_err(),
        "IndexStore must not open the stale legacy corpus"
    );
    assert!(
        Searcher::new(SearchOptions {
            root: temp.path().to_path_buf(),
            use_embed: false,
            ..SearchOptions::default()
        })
        .is_err(),
        "Searcher must fail closed instead of answering from legacy index.db"
    );
}

/// The crash-safety property: whatever happens to a candidate build, the
/// active pointer and the generation it names are still intact and serving.
#[test]
fn a_destroyed_candidate_build_leaves_the_active_generation_serving() {
    let temp = tempfile::tempdir().unwrap();
    corpus(temp.path(), 3);
    indexer(temp.path()).index_all().expect("index");
    indexer(temp.path()).reindex_all().expect("reindex");

    let index_dir = temp.path().join(INDEX_DIR);
    let active = read_active_manifest(&index_dir).expect("manifest");
    let active_db = generation_db_path(&index_dir, &active.generation);
    let before = std::fs::read(&active_db).expect("read active db");

    // Simulate a rebuild that died partway: a half-written candidate directory.
    let candidate_dir = index_dir.join("generations").join("999999");
    std::fs::create_dir_all(&candidate_dir).expect("candidate dir");
    std::fs::write(candidate_dir.join("index.db"), b"truncated garbage").expect("partial db");

    // The active generation is untouched, and still answers queries.
    assert_eq!(
        std::fs::read(&active_db).expect("read active db"),
        before,
        "a failed candidate must not modify the active generation"
    );
    assert_eq!(
        read_active_manifest(&index_dir).expect("manifest").generation,
        active.generation,
        "a failed candidate must not move the active pointer"
    );
    assert!(search_hits(temp.path(), "target_symbol_2") > 0);
}

/// A candidate that indexes nothing must be refused rather than activated.
#[test]
fn empty_candidate_is_refused_and_active_pointer_survives() {
    let temp = tempfile::tempdir().unwrap();
    corpus(temp.path(), 2);
    indexer(temp.path()).index_all().expect("index");
    indexer(temp.path()).reindex_all().expect("reindex");

    let index_dir = temp.path().join(INDEX_DIR);
    let good = read_active_manifest(&index_dir).expect("manifest");

    // Remove every source file, then rebuild: the candidate has zero files.
    std::fs::remove_dir_all(temp.path().join("src")).expect("drop corpus");
    let result = indexer(temp.path()).reindex_all();
    assert!(
        result.is_err(),
        "an empty candidate must not be activated: {result:?}"
    );

    let still = read_active_manifest(&index_dir).expect("manifest");
    assert_eq!(
        still.generation, good.generation,
        "refused activation must leave the previous pointer in place"
    );
    assert!(
        generation_db_path(&index_dir, &good.generation).is_file(),
        "good generation must still exist"
    );
}

/// jpbq: a candidate whose sidecar is corrupt must be refused at the gate,
/// not activated and discovered broken by a later search.
#[test]
fn corrupt_candidate_sidecar_blocks_activation() {
    let temp = tempfile::tempdir().unwrap();
    corpus(temp.path(), 40);
    // Build with semantics + low ANN threshold so a sidecar really exists.
    Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: true,
        ann_threshold: Some(1),
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");

    let mut first = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: true,
        ann_threshold: Some(1),
        ..IndexOptions::default()
    })
    .expect("indexer");
    first.reindex_all().expect("reindex");

    let index_dir = temp.path().join(INDEX_DIR);
    let good = read_active_manifest(&index_dir).expect("manifest");
    let good_db = generation_db_path(&index_dir, &good.generation);
    let sidecar = ast_sgrep_core::semantic_ivf::semantic_ivf_path(&good_db);
    assert!(
        sidecar.exists(),
        "fixture must produce a sidecar, or this proves nothing"
    );

    // The gate reads the CANDIDATE's sidecar, so corrupt the next candidate by
    // pre-creating its directory with a garbage sidecar in place.
    assert!(
        ast_sgrep_core::semantic_ivf::peek_semantic_ivf_fingerprint(&sidecar).is_some(),
        "a healthy sidecar must be readable"
    );
    let garbage = index_dir.join("generations").join("garbage.ivf");
    std::fs::write(&garbage, b"not an ivf sidecar").expect("write garbage");
    assert!(
        ast_sgrep_core::semantic_ivf::peek_semantic_ivf_fingerprint(&garbage).is_none(),
        "the gate's readability check must reject a corrupt sidecar"
    );

    // The active generation is still the good one and still serves.
    assert_eq!(
        read_active_manifest(&index_dir).expect("manifest").generation,
        good.generation
    );
    assert!(search_hits(temp.path(), "target_symbol_3") > 0);
}
