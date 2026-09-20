//! Dedup identity contracts (pass 18, EXP-012 / GA-12).
//!
//! GA-12 (H-CONF-016): the bulk-index skip decision must be
//! content-hash-pure whenever it could be ambiguous. The mtime fast path
//! (index_prepare.rs) is only sound for rows whose identity was produced
//! from a file at the SAME root as the current walk; a second root sharing
//! the index db must never be silently skipped on mtime agreement alone —
//! different content under equal mtimes must be re-indexed, and identical
//! content must skip deterministically regardless of mtimes or run order.
//!
//! EXP-012 (H-CONF-017): the pattern channel must emit strict multisets —
//! the envelope hit sequence may not contain duplicate
//! (file, line_start, line_end) keys. The canonical repro was
//! `pattern:main` over a multi-language corpus (12 ordered / 7 distinct).
//!
//! Failure-first: both GA-12 assertions failed against the pre-fix mtime
//! fast path (cross-root run reported `Indexed 0 files (1 skipped)` and
//! search served the OTHER root's stale content); the distinctness test
//! fails if the `seen`-guard merge dedup is removed (mutation-verified).

use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, SearchOptions, Searcher};
use std::fs::File;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

fn indexer(root: &Path, db: &Path) -> Indexer {
    Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap()
}

fn searcher(root: &Path, db: &Path) -> Searcher {
    let store = IndexStore::open(root, Some(db)).unwrap();
    Searcher::with_store(
        store,
        SearchOptions {
            root: root.to_path_buf(),
            use_embed: false,
            ..SearchOptions::default()
        },
    )
}

fn pin_mtime(path: &Path, offset: Duration) {
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(UNIX_EPOCH + offset)
        .unwrap();
}

/// GA-12: two roots share one index db; same rel path, same mtime, DIFFERENT
/// content. The second root's content must be re-indexed (never silently
/// skipped on mtime agreement), and search from the second root must serve
/// the second root's content.
#[test]
fn cross_root_same_mtime_different_content_is_reindexed() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("shared.db");
    let root_a = temp.path().join("rootA");
    let root_b = temp.path().join("rootB");
    std::fs::create_dir_all(&root_a).unwrap();
    std::fs::create_dir_all(&root_b).unwrap();
    std::fs::write(root_a.join("greet.py"), "def greet():\n    pass\n").unwrap();
    std::fs::write(
        root_b.join("greet.py"),
        "def greet(name):\n    return name\n",
    )
    .unwrap();
    let pinned = Duration::from_secs(1_760_000_000);
    pin_mtime(&root_a.join("greet.py"), pinned);
    pin_mtime(&root_b.join("greet.py"), pinned);

    let stats = indexer(&root_a, &db).index_all().unwrap();
    assert_eq!(stats.files_indexed, 1, "first root must index its file");

    // THE CONTRACT: the second root's differing content is not skipped.
    // (Failure-first: the mtime fast path skipped this run entirely —
    // `Indexed 0 files (1 skipped)` — and search then served root A's stale
    // content from root B's invocation.)
    let stats = indexer(&root_b, &db).index_all().unwrap();
    assert_eq!(
        stats.files_indexed, 1,
        "cross-root content difference must be re-indexed even under equal mtimes"
    );

    let hits = searcher(&root_b, &db).search("pattern:greet").unwrap();
    assert_eq!(hits.hits.len(), 1);
    assert!(
        hits.hits[0].excerpt.contains("greet(name)"),
        "search from root B must serve root B's content, got: {:?}",
        hits.hits[0].excerpt
    );
}

/// GA-12 surface form: identical content under two roots must skip on the
/// content hash alone, deterministically in BOTH run orders and regardless
/// of the files' mtimes (the H-CONF-016 `(6 skipped)` vs `(1 skipped)`
/// run-order nondeterminism).
#[test]
fn cross_root_identical_content_skips_deterministically() {
    let make_case = |order_b_first: bool| {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("shared.db");
        let root_a = temp.path().join("rootA");
        let root_b = temp.path().join("rootB");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&root_b).unwrap();
        let body = "def greet():\n    pass\n";
        std::fs::write(root_a.join("greet.py"), body).unwrap();
        std::fs::write(root_b.join("greet.py"), body).unwrap();
        pin_mtime(&root_a.join("greet.py"), Duration::from_secs(1_760_000_000));
        pin_mtime(&root_b.join("greet.py"), Duration::from_secs(1_760_000_100));

        let (first, second) = if order_b_first {
            (&root_b, &root_a)
        } else {
            (&root_a, &root_b)
        };
        let stats = indexer(first, &db).index_all().unwrap();
        assert_eq!(stats.files_indexed, 1);
        assert_eq!(stats.files_skipped, 0);

        // Identical content: the second root must skip on hash equality —
        // pure content decision, independent of mtime and run order.
        let stats = indexer(second, &db).index_all().unwrap();
        assert_eq!(
            stats.files_indexed, 0,
            "identical content must not re-index (order_b_first={order_b_first})"
        );
        assert_eq!(
            stats.files_skipped, 1,
            "identical content must count as one content-hash skip (order_b_first={order_b_first})"
        );

        // A third pass over the first root is a pure noop either way.
        let stats = indexer(first, &db).index_all().unwrap();
        assert_eq!(stats.files_indexed, 0);
        assert_eq!(stats.files_skipped, 1);
    };
    make_case(false);
    make_case(true);
}

/// EXP-012 (H-CONF-017, re-keyed per PASS 131/f131): the pattern channel
/// emits span-distinct rows — no duplicate (file, line_start, line_end,
/// byte_span) keys. Two INDEPENDENT same-line nodes are two sg rows and
/// must both survive (f131, sg receipt); the merge-time dedup collapses
/// the SAME node found by overlapping query arms (equal spans), which is
/// what this pins. Line-only keys contradicted f131 on the `$F($$$A)`
/// sibling-call face; one-row-per-line presentation lives downstream
/// (RRF line-fusion on the hybrid path), not in this union.
#[test]
fn pattern_channel_hit_keys_are_distinct() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(root.join("a.rs"), "fn main() {}\nfn aux() {} fn aux() {}\n").unwrap();
    std::fs::write(
        root.join("b.rs"),
        "fn main() {}\nfn caller() { aux(); aux(); }\n",
    )
    .unwrap();
    std::fs::write(root.join("c.py"), "def main():\n    pass\n").unwrap();
    let db = temp.path().join(".asgrep").join("index.db");
    indexer(root, &db).index_all().unwrap();
    let searcher = searcher(root, &db);

    for query in [
        "pattern:main",
        "pattern:greet",
        "pattern:aux",
        "pattern:$F($$$A)",
    ] {
        let response = searcher.search(query).unwrap();
        let keys: Vec<_> = response
            .hits
            .iter()
            .map(|hit| {
                (
                    hit.file.clone(),
                    hit.line_start,
                    hit.line_end,
                    hit.byte_span,
                )
            })
            .collect();
        let distinct = keys.iter().collect::<std::collections::HashSet<_>>();
        assert_eq!(
            keys.len(),
            distinct.len(),
            "pattern channel emitted duplicate hit keys for {query}: {keys:?}"
        );
    }
    // The sibling-call face keeps BOTH rows (f131 contract at this layer).
    let sibs = searcher.search("pattern:$F($$$A)").unwrap();
    let brow: Vec<_> = sibs
        .hits
        .iter()
        .filter(|hit| hit.file == "b.rs" && hit.line_start == 2)
        .collect();
    assert_eq!(brow.len(), 2, "sibling calls survive: {brow:?}");
    assert_ne!(
        brow[0].byte_span, brow[1].byte_span,
        "surviving rows are span-distinct nodes: {brow:?}"
    );
}
