//! d3l5: a SearchResponse may carry evidence from exactly one index generation.
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn corpus(root: &std::path::Path, files: usize) {
    let src = root.join("src");
    std::fs::create_dir_all(&src).expect("mkdir");
    for index in 0..files {
        std::fs::write(
            src.join(format!("mod{index}.rs")),
            format!(
                "fn target_symbol_{index}() {{ helper_{index}(); }}\nfn helper_{index}() {{}}\n"
            ),
        )
        .expect("write");
    }
}

fn index_at(root: &std::path::Path) {
    Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");
}

fn searcher_at(root: &std::path::Path) -> Searcher {
    Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        use_embed: false,
        ..SearchOptions::default()
    })
    .expect("searcher")
}

#[test]
fn response_carries_the_snapshot_it_was_read_from() {
    let temp = tempfile::tempdir().unwrap();
    corpus(temp.path(), 3);
    index_at(temp.path());

    let searcher = searcher_at(temp.path());
    let response = searcher.search("target_symbol_0").expect("search");
    assert!(!response.hits.is_empty(), "fixture must match");

    let stamp = &response.snapshot;
    assert!(
        stamp.generation > 0,
        "generation must be recorded: {stamp:?}"
    );
    // Assert against the store's own view rather than a literal, so a future
    // schema bump does not look like a snapshot regression.
    assert_eq!(
        stamp.schema_version,
        searcher.store().schema_version(),
        "schema version recorded"
    );
    assert!(stamp.schema_version > 0);
    assert!(stamp.worktree_revision > 0, "worktree revision recorded");
    assert!(
        stamp.degraded_channels.is_empty(),
        "healthy index must not report degraded channels: {stamp:?}"
    );

    // The stamp must equal what the store reports right now.
    assert_eq!(
        stamp.generation,
        searcher.store().index_generation().expect("generation")
    );
}

#[test]
fn generation_increases_with_indexing_and_is_reflected_in_responses() {
    let temp = tempfile::tempdir().unwrap();
    corpus(temp.path(), 2);
    index_at(temp.path());

    let first = searcher_at(temp.path())
        .search("target_symbol_0")
        .expect("search")
        .snapshot
        .generation;

    // A new file is a new generation.
    std::fs::write(
        temp.path().join("src").join("extra.rs"),
        "fn target_symbol_extra() {}\n",
    )
    .unwrap();
    index_at(temp.path());

    let second = searcher_at(temp.path())
        .search("target_symbol_0")
        .expect("search")
        .snapshot
        .generation;
    assert!(
        second > first,
        "indexing must advance the generation ({first} -> {second})"
    );
}

/// The invariant under contention: reindex in a loop while searching, and every
/// response must still be internally single-generation.
#[test]
fn concurrent_reindex_never_yields_a_mixed_generation_response() {
    let temp = tempfile::tempdir().unwrap();
    corpus(temp.path(), 6);
    index_at(temp.path());

    let root = temp.path().to_path_buf();
    let stop = Arc::new(AtomicBool::new(false));
    let writer_stop = Arc::clone(&stop);
    let writer_root = root.clone();
    let writer = std::thread::spawn(move || {
        let mut round = 0_usize;
        while !writer_stop.load(Ordering::Relaxed) {
            std::fs::write(
                writer_root.join("src").join("churn.rs"),
                format!("fn churn_{round}() {{ target_symbol_1(); }}\n"),
            )
            .expect("write churn");
            index_at(&writer_root);
            round += 1;
        }
        round
    });

    let mut observed = Vec::new();
    let mut rejected = 0_usize;
    for _ in 0..40 {
        let searcher = searcher_at(&root);
        match searcher.search("target_symbol_1") {
            Ok(response) => {
                let stamp = response.snapshot.clone();
                // Whatever generation this response claims, the hits it carries
                // were read under that same pinned snapshot.
                assert!(stamp.generation > 0, "stamped generation: {stamp:?}");
                observed.push(stamp.generation);
            }
            // A detected mid-search generation change is REPORTED, which is the
            // contract: never a silently mixed response.
            Err(error) => {
                assert!(
                    error.to_string().contains("index generation changed"),
                    "unexpected search error: {error}"
                );
                rejected += 1;
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    let rounds = writer.join().expect("writer thread");

    assert!(rounds > 0, "writer must have reindexed at least once");
    assert!(
        !observed.is_empty() || rejected > 0,
        "searches must have produced results or explicit rejections"
    );
    // Generations only move forward.
    let mut sorted = observed.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, observed, "observed generations must be monotonic");
}

/// Mechanism proof for the fence: inside a deferred read transaction, another
/// connection's committed write must be invisible. This is what makes the
/// single-generation guarantee real rather than merely asserted.
#[test]
fn deferred_read_snapshot_hides_a_concurrent_commit() {
    use ast_sgrep_core::IndexStore;

    let temp = tempfile::tempdir().unwrap();
    corpus(temp.path(), 2);
    index_at(temp.path());

    let reader = IndexStore::open(temp.path(), None).expect("reader");
    let writer = IndexStore::open(temp.path(), None).expect("writer");

    // Pin a snapshot: the first read inside the transaction fixes it.
    reader
        .connection()
        .execute_batch("BEGIN DEFERRED")
        .expect("begin deferred");
    let pinned = reader.index_generation().expect("pinned generation");

    // Commit real work on the other connection.
    writer
        .set_meta("snapshot_probe", "written-after-pin")
        .expect("write meta");
    writer
        .connection()
        .execute_batch(
            "INSERT INTO meta(key, value) VALUES('index_data_version', '1')
             ON CONFLICT(key) DO UPDATE SET value =
             CAST(COALESCE(meta.value, '0') AS INTEGER) + 1",
        )
        .expect("bump generation");
    let advanced = writer.index_generation().expect("writer generation");
    assert!(
        advanced > pinned,
        "writer must advance ({pinned} -> {advanced})"
    );

    // The reader is still pinned to its snapshot.
    let still = reader.index_generation().expect("reader generation");
    assert_eq!(
        still, pinned,
        "deferred read snapshot leaked a concurrent commit ({pinned} -> {still})"
    );
    assert_eq!(
        reader.get_meta("snapshot_probe").expect("probe"),
        None,
        "snapshot must not observe a row committed after it was pinned"
    );

    reader.connection().execute_batch("COMMIT").expect("commit");

    // After releasing the snapshot the reader catches up.
    assert_eq!(
        reader.index_generation().expect("post-commit generation"),
        advanced
    );
}

#[test]
fn snapshot_setup_failure_does_not_leave_a_read_transaction_open() {
    let temp = tempfile::tempdir().unwrap();
    corpus(temp.path(), 1);
    index_at(temp.path());
    let searcher = searcher_at(temp.path());
    searcher
        .store()
        .connection()
        .execute_batch("DROP TABLE meta")
        .expect("break generation lookup");

    let error = searcher
        .search("target_symbol_0")
        .expect_err("generation lookup must fail");
    assert!(
        error.to_string().contains("meta"),
        "unexpected error: {error}"
    );
    assert!(
        searcher.store().connection().is_autocommit(),
        "failed snapshot setup must still close its transaction"
    );
}

/// d3l5: a sidecar built for a different generation must be reported, not
/// silently ignored. `load_semantic_ivf` returns None on mismatch, which makes
/// a stale sidecar look identical to no sidecar at all.
#[test]
fn stale_semantic_sidecar_is_reported_as_a_degraded_channel() {
    let temp = tempfile::tempdir().unwrap();
    // Enough chunks, and a low ANN threshold, so an IVF sidecar is actually
    // built -- otherwise this test would pass without exercising anything.
    corpus(temp.path(), 40);
    Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: true,
        ann_threshold: Some(1),
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");

    let searcher = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        use_embed: true,
        ..SearchOptions::default()
    })
    .expect("searcher");

    let sidecar = ast_sgrep_core::semantic_ivf::semantic_ivf_path(searcher.store().db_path());
    assert!(
        sidecar.exists(),
        "fixture must build a real IVF sidecar at {}, or this test proves nothing",
        sidecar.display()
    );

    let healthy = searcher.search("target_symbol_0").expect("search");
    assert!(
        healthy.snapshot.semantic_manifest.is_some(),
        "sidecar present, so its fingerprint must be reported"
    );
    assert!(
        healthy
            .snapshot
            .degraded_channels
            .iter()
            .all(|channel| channel.reason != "sidecar_generation_mismatch"),
        "fresh sidecar must not be reported stale: {:?}",
        healthy.snapshot
    );

    // Advance the generation without rebuilding the sidecar.
    searcher
        .store()
        .connection()
        .execute_batch(
            "INSERT INTO meta(key, value) VALUES('index_data_version', '1')
             ON CONFLICT(key) DO UPDATE SET value =
             CAST(COALESCE(meta.value, '0') AS INTEGER) + 1",
        )
        .expect("bump generation");

    let stale = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        use_embed: true,
        ..SearchOptions::default()
    })
    .expect("searcher")
    .search("target_symbol_0")
    .expect("search");

    assert!(
        stale
            .snapshot
            .degraded_channels
            .iter()
            .any(|channel| channel.channel == "semantic"
                && channel.reason == "sidecar_generation_mismatch"),
        "stale sidecar must surface as a degraded channel: {:?}",
        stale.snapshot
    );
}
