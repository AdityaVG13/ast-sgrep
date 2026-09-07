//! Pass 65b (r15 finding 1) — an in-memory store has NO filesystem surface.
//!
//! Contract: `IndexStore::open_in_memory` (and the H-CONF-033 foreign-root
//! searcher swap that constructs it) must never resolve `:memory:` to a
//! filesystem path. Historically the fresh store's `user_version = 0` took
//! the legacy migration branch in `init_schema`, which called
//! `invalidate_semantic_ivf(Path::new(":memory:"))`; `semantic_ivf_path`
//! degenerated `":memory:".parent() == Some("")` into the CWD-relative
//! `./semantic.ivf` and unlinked it — every foreign-root search deleted
//! whatever file sat in the process working directory (reproduced by the
//! pass-64c audit; the engine names its own sidecars `semantic.ivf`, so the
//! collision is realistic, not synthetic).
//!
//! Determinism: both tests serialize on a process-local mutex because the
//! working directory is process-global; every assertion is on exact bytes or
//! exact hit sets, so a deleted sentinel or a broken walk fails loudly.
//! Mutation-load-bearing: removing the `is_in_memory_db` no-op fails the
//! sentinel assertions; skipping `init_schema` for `:memory:` instead of
//! no-oping the invalidation fails the schema-version assertion (and the
//! search itself), so the lazy "don't migrate at all" shape is rejected.
use ast_sgrep_core::{IndexOptions, IndexStore, SearchOptions, Searcher, INDEX_SCHEMA_VERSION};
use ast_sgrep_testkit::isolated_index_session;

/// CWD is process-global: serialize against any other cwd-switching test in
/// this binary. Poison is deliberately drained: a panicked predecessor must
/// not wedge the remaining tests.
static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Restores the process working directory on scope exit, including unwind —
/// a failing assertion mid-region must not strand later tests in the scratch
/// directory.
struct RestoreCwd(std::path::PathBuf);
impl Drop for RestoreCwd {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

const SENTINEL_BYTES: &[u8] = b"cwd-sentinel-do-not-delete";

/// End-to-end 64c reproduction: an index bound to root A, a query tree at
/// root B. The read-side root binding swaps to the EMPTY in-memory stand-in
/// whose fresh `user_version = 0` runs the legacy migration at open time.
/// The CWD sentinel must survive byte-for-byte AND the native walk must
/// still answer root B exactly — the fix must not be "the stand-in broke
/// search" any more than "the sentinel dies quietly".
#[test]
fn foreign_root_search_spares_cwd_semantic_ivf_and_answers_from_the_walk() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    // Root A owns the foreign index the search will be pointed at.
    let session_a = isolated_index_session();
    session_a.write("mod.py", "def alpha():\n    pass\n");
    session_a.index_all(IndexOptions {
        embed_semantic: false,
        ..session_a.index_options()
    });

    // Root B is the queried tree; only the native walk can answer for it.
    let session_b = isolated_index_session();
    session_b.write("mod.rs", "fn beta() {}\n");
    session_b.index_all(IndexOptions {
        embed_semantic: false,
        ..session_b.index_options()
    });

    // The trap: a CWD file named exactly like the derived sidecar.
    let scratch = tempfile::tempdir().expect("scratch cwd");
    let sentinel = scratch.path().join("semantic.ivf");
    std::fs::write(&sentinel, SENTINEL_BYTES).expect("write sentinel");

    let original = std::env::current_dir().expect("read cwd");
    std::env::set_current_dir(scratch.path()).expect("switch cwd");
    let _restore = RestoreCwd(original);

    let searcher = Searcher::new(SearchOptions {
        root: session_b.corpus_root.clone(),
        index_path: Some(session_a.index_path.clone()),
        use_embed: false,
        limit: 8,
        ..session_b.search_options()
    })
    .expect("foreign-root search must open through the inert in-memory store");

    // Walk-correctness: the passive stand-in must not degrade the answer.
    let response = searcher
        .search("pattern:fn $A() { $$$B }")
        .expect("search must answer over the stand-in store");
    assert_eq!(
        response.hits.len(),
        1,
        "the walk must serve exactly the root-B hit: {:?}",
        response.hits
    );
    assert!(
        response.hits[0].file.ends_with("mod.rs"),
        "hit must come from the queried root B, not the foreign index: {:?}",
        response.hits[0]
    );
    assert_eq!(response.hits[0].line_start, 1, "{:?}", response.hits[0]);
    assert!(
        response.hits[0].excerpt.contains("fn beta"),
        "hit must be the root-B function: {:?}",
        response.hits[0]
    );

    // DATA-LOSS contract: the sentinel survives, byte-for-byte.
    assert!(
        sentinel.exists(),
        "in-memory store migration deleted the CWD-relative semantic.ivf (data loss)"
    );
    assert_eq!(
        std::fs::read(&sentinel).expect("read sentinel"),
        SENTINEL_BYTES,
        "sentinel must survive byte-for-byte"
    );
}

/// Direct store seam: merely OPENING an in-memory store — before any
/// searcher exists — must be side-effect free on the filesystem while still
/// completing its schema migration. Pinning the stamped `user_version`
/// proves the legacy branch actually ran (so the no-op guard was exercised)
/// and rejects the mutant that "fixes" the data loss by skipping the
/// migration entirely for `:memory:`.
#[test]
fn open_in_memory_completes_migration_without_touching_the_filesystem() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    let session = isolated_index_session();
    session.write("mod.rs", "fn gamma() {}\n");

    let scratch = tempfile::tempdir().expect("scratch cwd");
    let sentinel = scratch.path().join("semantic.ivf");
    std::fs::write(&sentinel, SENTINEL_BYTES).expect("write sentinel");

    let original = std::env::current_dir().expect("read cwd");
    std::env::set_current_dir(scratch.path()).expect("switch cwd");
    let _restore = RestoreCwd(original);

    let store = IndexStore::open_in_memory(&session.corpus_root)
        .expect("in-memory store must open under a hostile CWD");
    assert_eq!(store.db_path(), std::path::Path::new(":memory:"));
    assert_eq!(
        store
            .on_disk_schema_version()
            .expect("in-memory store must expose its stamped schema version"),
        INDEX_SCHEMA_VERSION,
        "the legacy migration must have completed for the in-memory store"
    );

    assert!(
        sentinel.exists(),
        "opening an in-memory store deleted the CWD-relative semantic.ivf (data loss)"
    );
    assert_eq!(
        std::fs::read(&sentinel).expect("read sentinel"),
        SENTINEL_BYTES,
        "sentinel must survive byte-for-byte"
    );
}
