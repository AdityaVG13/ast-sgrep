//! I4 invalidation-contract oracles: END-TO-END change→detect→refresh→serve drills.
//!
//! Scope: FULL drills through public APIs only. Every test runs the same
//! five-phase loop against a populated index that is already serving queries:
//!
//! 1. SERVE baseline — the populated index answers queries with exact hit sets.
//! 2. CHANGE — a real filesystem mutation (add / modify / delete / rename).
//! 3. DETECT — staleness is proven WITHOUT refreshing: the stored
//!    `index_data_version`, the stored `status()` counts, and the
//!    `writer_generation` stamp are all byte-frozen while the live tree has
//!    moved on, and serve still answers the pre-change world (stale hits
//!    linger, new hits are invisible).
//! 4. REFRESH — one explicit `index_all` absorbs the pending delta(s).
//! 5. SERVE — search proves the EXACT new hit sets (exact file vectors,
//!    exact literal spots, emptiness of retired symbols), the generation
//!    moved by exactly the mutated-file count, and stored counts match the
//!    live tree.
//!
//! Non-duplication vs `invalidation_pass1/2/3.rs`: I1 pins staleness
//! DETECTION discriminants in isolation (epochs, mtime gates, schema refusal,
//! routing); I2 pins single-delta search CONSEQUENCES (before/after hit
//! sets); I3 pins cross-strategy RELATIONS (incremental/full parity, order
//! independence, idempotence, monotonicity). I4 pins the full LOOP: no test
//! here is meaningful with any phase removed — each asserts the frozen
//! detect-phase AND the exact serve-phase around the same change.
//!
//! Determinism: MODIFY changes forge explicit whole-second mtimes with
//! read-back preconditions (same technique as I1/I2/I3), so change detection
//! never depends on filesystem timestamp granularity. No wall-clock sleeps.
//! Hermeticity: every open uses an explicit `index_path` outside the corpus
//! root, so no test depends on ambient `ASGREP_*` routing. Assertions cover
//! hit sets, counts, and generations only — never message text.

use ast_sgrep_core::{
    HitKind, IndexOptions, IndexStats, IndexStore, Indexer, SearchOptions, Searcher,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

/// Fixed whole-second stamps (nanos = 0 survives every filesystem timestamp
/// granularity). MODIFY drills forge T0 at creation, T1 (then T2) after each
/// rewrite so every change is deterministically detectable.
const WHOLE_SECOND_T0: u64 = 1_700_000_000;
const WHOLE_SECOND_T1: u64 = 1_700_003_600;
const WHOLE_SECOND_T2: u64 = 1_700_007_200;

fn set_mtime_checked(path: &Path, secs: u64) {
    let time = UNIX_EPOCH + Duration::new(secs, 0);
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
    assert_eq!(
        path.metadata().unwrap().modified().unwrap(),
        time,
        "filesystem must store the forged mtime exactly"
    );
}

struct Fx {
    _corpus: tempfile::TempDir,
    _index: tempfile::TempDir,
    root: PathBuf,
    db: PathBuf,
}

/// Snapshot of the staleness-detection surface: stored generation, stored
/// row counts, and the cross-process writer stamp.
struct DetectionSnap {
    generation: i64,
    counts: (usize, usize, usize, usize, usize),
    writer: u64,
}

impl Fx {
    fn new() -> Self {
        let corpus = tempfile::tempdir().unwrap();
        let index = tempfile::tempdir().unwrap();
        let root = corpus.path().to_path_buf();
        std::fs::create_dir_all(root.join("src")).unwrap();
        let db = index.path().join("index.db");
        Self {
            _corpus: corpus,
            _index: index,
            root,
            db,
        }
    }

    fn write(&self, rel: &str, content: &str) -> PathBuf {
        let abs = self.root.join(rel);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&abs, content).unwrap();
        abs
    }

    fn reindex(&self) -> IndexStats {
        Indexer::new(IndexOptions {
            root: self.root.clone(),
            index_path: Some(self.db.clone()),
            use_tantivy: false,
            embed_semantic: false,
            ..IndexOptions::default()
        })
        .unwrap()
        .index_all()
        .unwrap()
    }

    fn searcher(&self) -> Searcher {
        Searcher::new(SearchOptions {
            root: self.root.clone(),
            index_path: Some(self.db.clone()),
            limit: 256,
            use_embed: false,
            ..SearchOptions::default()
        })
        .unwrap()
    }

    /// Sorted `file` values of Def hits whose symbol is exactly `symbol`.
    fn def_files(&self, symbol: &str) -> Vec<String> {
        let query = format!("defs:{symbol}");
        let mut files: Vec<String> = self
            .searcher()
            .search(&query)
            .unwrap()
            .hits
            .iter()
            .filter(|h| h.kind == HitKind::Def && h.symbol.as_deref() == Some(symbol))
            .map(|h| h.file.clone())
            .collect();
        files.sort();
        files
    }

    /// Sorted `(file, line_start)` spots for a `literal:` query (all hits).
    fn literal_spots(&self, token: &str) -> Vec<(String, u32)> {
        let query = format!("literal:{token}");
        let mut spots: Vec<(String, u32)> = self
            .searcher()
            .search(&query)
            .unwrap()
            .hits
            .iter()
            .map(|h| (h.file.clone(), h.line_start))
            .collect();
        spots.sort();
        spots
    }

    /// Sorted `file` values of Caller hits whose callee is exactly `callee`.
    fn caller_files(&self, callee: &str) -> Vec<String> {
        let query = format!("callers:{callee}");
        let mut files: Vec<String> = self
            .searcher()
            .search(&query)
            .unwrap()
            .hits
            .iter()
            .filter(|h| h.kind == HitKind::Caller && h.callee.as_deref() == Some(callee))
            .map(|h| h.file.clone())
            .collect();
        files.sort();
        files
    }

    fn generation(&self) -> i64 {
        IndexStore::open(&self.root, Some(&self.db))
            .unwrap()
            .index_data_version()
            .unwrap()
    }

    fn counts(&self) -> (usize, usize, usize, usize, usize) {
        let status = IndexStore::open(&self.root, Some(&self.db))
            .unwrap()
            .status()
            .unwrap();
        (
            status.file_count,
            status.line_count,
            status.symbol_count,
            status.caller_count,
            status.import_count,
        )
    }

    fn writer_epoch(&self) -> u64 {
        IndexStore::open(&self.root, Some(&self.db))
            .unwrap()
            .status()
            .unwrap()
            .writer_generation
    }

    fn snapshot(&self) -> DetectionSnap {
        DetectionSnap {
            generation: self.generation(),
            counts: self.counts(),
            writer: self.writer_epoch(),
        }
    }

    /// The DETECT phase: after a filesystem change but before any refresh,
    /// the whole stored detection surface must be frozen — the index has no
    /// idea the tree moved.
    fn assert_frozen(&self, snap: &DetectionSnap) {
        assert_eq!(
            self.generation(),
            snap.generation,
            "stored generation must be frozen until a refresh runs"
        );
        assert_eq!(
            self.counts(),
            snap.counts,
            "stored counts must be frozen until a refresh runs"
        );
        assert_eq!(
            self.writer_epoch(),
            snap.writer,
            "writer stamp must be frozen until a refresh runs"
        );
    }

    /// Sorted rel paths of live `.rs` files under the root (the ground truth
    /// the stored counts are stale against).
    fn live_rs_files(&self) -> Vec<String> {
        fn visit(dir: &Path, root: &Path, out: &mut Vec<String>) {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    visit(&path, root, out);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    out.push(path.strip_prefix(root).unwrap().display().to_string());
                }
            }
        }
        let mut out = Vec::new();
        visit(&self.root, &self.root, &mut out);
        out.sort();
        out
    }
}

// ---- drill: ADD ----

#[test]
fn drill_add_new_file_end_to_end() {
    let fx = Fx::new();
    fx.write("src/a.rs", "fn i4_add_alpha() { let _t = \"tokalphaa\"; }\n");
    assert_eq!(fx.reindex().files_indexed, 1);

    // SERVE baseline: exact hit sets from the populated index.
    assert_eq!(fx.def_files("i4_add_alpha"), vec!["src/a.rs".to_string()]);
    assert_eq!(
        fx.literal_spots("tokalphaa"),
        vec![("src/a.rs".to_string(), 1)]
    );
    let snap = fx.snapshot();
    assert_eq!(snap.counts.0, 1);

    // CHANGE: a real new file on disk.
    fx.write("src/b.rs", "fn i4_add_beta() { let _t = \"tokbetaab\"; }\n");

    // DETECT: stored state frozen while the live tree moved on.
    fx.assert_frozen(&snap);
    assert_eq!(
        fx.live_rs_files(),
        vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
    );
    assert_eq!(
        fx.live_rs_files().len(),
        snap.counts.0 + 1,
        "the live tree outruns the stored file count: stale"
    );
    assert!(
        fx.def_files("i4_add_beta").is_empty(),
        "stale serve: the new symbol is invisible before refresh"
    );
    assert!(fx.literal_spots("tokbetaab").is_empty());

    // REFRESH.
    let stats = fx.reindex();
    assert_eq!(stats.files_indexed, 1);
    assert_eq!(stats.files_removed, 0);

    // SERVE proves the exact new hit sets.
    assert_eq!(fx.generation(), snap.generation + 1);
    assert_ne!(fx.writer_epoch(), snap.writer);
    assert_eq!(fx.counts().0, 2);
    assert_eq!(fx.def_files("i4_add_beta"), vec!["src/b.rs".to_string()]);
    assert_eq!(
        fx.literal_spots("tokbetaab"),
        vec![("src/b.rs".to_string(), 1)]
    );
    assert_eq!(fx.def_files("i4_add_alpha"), vec!["src/a.rs".to_string()]);
    assert_eq!(
        fx.literal_spots("tokalphaa"),
        vec![("src/a.rs".to_string(), 1)]
    );
}

// ---- drill: MODIFY ----

#[test]
fn drill_modify_file_end_to_end() {
    let fx = Fx::new();
    let abs = fx.write("src/a.rs", "fn i4_mod_old() { let _t = \"tokmodold\"; }\n");
    set_mtime_checked(&abs, WHOLE_SECOND_T0);
    fx.write("src/b.rs", "fn i4_mod_sib() { let _t = \"tokmodstbl\"; }\n");
    assert_eq!(fx.reindex().files_indexed, 2);

    // SERVE baseline: exact hit sets, including the untouched sibling.
    assert_eq!(fx.def_files("i4_mod_old"), vec!["src/a.rs".to_string()]);
    assert_eq!(
        fx.literal_spots("tokmodold"),
        vec![("src/a.rs".to_string(), 1)]
    );
    assert_eq!(fx.def_files("i4_mod_sib"), vec!["src/b.rs".to_string()]);
    assert_eq!(
        fx.literal_spots("tokmodstbl"),
        vec![("src/b.rs".to_string(), 1)]
    );
    let snap = fx.snapshot();

    // CHANGE: real new bytes on disk with a forged newer mtime.
    fx.write("src/a.rs", "fn i4_mod_new() { let _t = \"tokmodnew\"; }\n");
    set_mtime_checked(&abs, WHOLE_SECOND_T1);

    // DETECT: same file count, so staleness shows in frozen state + stale
    // serve — the retired symbol lingers, the new one is invisible.
    fx.assert_frozen(&snap);
    assert_eq!(
        fx.live_rs_files().len(),
        snap.counts.0,
        "MODIFY keeps the file count, so counts alone cannot detect it"
    );
    assert_eq!(
        fx.def_files("i4_mod_old"),
        vec!["src/a.rs".to_string()],
        "stale serve: the retired symbol is still visible"
    );
    assert!(
        fx.def_files("i4_mod_new").is_empty(),
        "stale serve: the new symbol is invisible before refresh"
    );
    assert!(fx.literal_spots("tokmodnew").is_empty());

    // REFRESH.
    let stats = fx.reindex();
    assert_eq!(stats.files_indexed, 1);
    assert_eq!(stats.files_removed, 0);

    // SERVE proves the exact new hit sets.
    assert_eq!(fx.generation(), snap.generation + 1);
    assert_ne!(fx.writer_epoch(), snap.writer);
    assert_eq!(fx.counts().0, 2);
    assert!(fx.def_files("i4_mod_old").is_empty());
    assert!(fx.literal_spots("tokmodold").is_empty());
    assert_eq!(fx.def_files("i4_mod_new"), vec!["src/a.rs".to_string()]);
    assert_eq!(
        fx.literal_spots("tokmodnew"),
        vec![("src/a.rs".to_string(), 1)]
    );
    assert_eq!(fx.def_files("i4_mod_sib"), vec!["src/b.rs".to_string()]);
    assert_eq!(
        fx.literal_spots("tokmodstbl"),
        vec![("src/b.rs".to_string(), 1)]
    );
}

// ---- drill: DELETE ----

#[test]
fn drill_delete_file_end_to_end() {
    let fx = Fx::new();
    fx.write("src/a.rs", "fn i4_doomed() { let _t = \"tokdoomed\"; }\n");
    fx.write("src/b.rs", "fn i4_survivor() { let _t = \"toksurvive\"; }\n");
    assert_eq!(fx.reindex().files_indexed, 2);

    // SERVE baseline.
    assert_eq!(fx.def_files("i4_doomed"), vec!["src/a.rs".to_string()]);
    assert_eq!(
        fx.literal_spots("tokdoomed"),
        vec![("src/a.rs".to_string(), 1)]
    );
    let snap = fx.snapshot();
    assert_eq!(snap.counts.0, 2);

    // CHANGE: the file really leaves the disk.
    std::fs::remove_file(fx.root.join("src/a.rs")).unwrap();

    // DETECT: stored state frozen; stale serve still answers the dead file.
    fx.assert_frozen(&snap);
    assert_eq!(fx.live_rs_files(), vec!["src/b.rs".to_string()]);
    assert_eq!(
        fx.def_files("i4_doomed"),
        vec!["src/a.rs".to_string()],
        "stale serve: the deleted file's defs are still visible"
    );
    assert!(
        !fx.literal_spots("tokdoomed").is_empty(),
        "stale serve: the deleted file's literals are still visible"
    );

    // REFRESH.
    let stats = fx.reindex();
    assert_eq!(stats.files_removed, 1);
    assert_eq!(stats.files_indexed, 0);

    // SERVE proves the exact new hit sets.
    assert_eq!(fx.generation(), snap.generation + 1);
    assert_ne!(fx.writer_epoch(), snap.writer);
    assert_eq!(fx.counts().0, 1);
    assert!(fx.def_files("i4_doomed").is_empty());
    assert!(fx.literal_spots("tokdoomed").is_empty());
    assert_eq!(fx.def_files("i4_survivor"), vec!["src/b.rs".to_string()]);
    assert_eq!(
        fx.literal_spots("toksurvive"),
        vec![("src/b.rs".to_string(), 1)]
    );
}

// ---- drill: RENAME ----

#[test]
fn drill_rename_file_end_to_end() {
    let fx = Fx::new();
    fx.write(
        "src/old_name.rs",
        "fn i4_roamer() { let _t = \"tokroamer\"; }\n",
    );
    fx.write("src/sib.rs", "fn i4_ren_sib() {}\n");
    assert_eq!(fx.reindex().files_indexed, 2);

    // SERVE baseline: hits live under the old path.
    assert_eq!(
        fx.def_files("i4_roamer"),
        vec!["src/old_name.rs".to_string()]
    );
    assert_eq!(
        fx.literal_spots("tokroamer"),
        vec![("src/old_name.rs".to_string(), 1)]
    );
    let snap = fx.snapshot();

    // CHANGE: a real filesystem rename.
    std::fs::rename(
        fx.root.join("src/old_name.rs"),
        fx.root.join("src/new_name.rs"),
    )
    .unwrap();

    // DETECT: frozen state; stale serve still answers the dead path.
    fx.assert_frozen(&snap);
    assert!(!fx.root.join("src/old_name.rs").exists());
    assert!(fx.root.join("src/new_name.rs").exists());
    assert_eq!(
        fx.def_files("i4_roamer"),
        vec!["src/old_name.rs".to_string()],
        "stale serve: hits still sit under the dead path"
    );

    // REFRESH.
    let stats = fx.reindex();
    assert_eq!(stats.files_removed, 1);
    assert_eq!(stats.files_indexed, 1);

    // SERVE proves the exact new hit sets.
    assert_eq!(
        fx.generation(),
        snap.generation + 2,
        "a rename mutates two rows: one removal + one upsert"
    );
    assert_ne!(fx.writer_epoch(), snap.writer);
    assert_eq!(fx.counts().0, 2);
    assert_eq!(
        fx.def_files("i4_roamer"),
        vec!["src/new_name.rs".to_string()]
    );
    assert_eq!(
        fx.literal_spots("tokroamer"),
        vec![("src/new_name.rs".to_string(), 1)]
    );
    assert_eq!(fx.def_files("i4_ren_sib"), vec!["src/sib.rs".to_string()]);
}

// ---- drill: chained multi-change absorbed by one refresh ----

#[test]
fn drill_chained_multi_change_single_refresh() {
    let fx = Fx::new();
    let abs_a = fx.write(
        "src/a.rs",
        "fn i4_chain_old() { let _t = \"tokchainold\"; }\n",
    );
    set_mtime_checked(&abs_a, WHOLE_SECOND_T0);
    fx.write("src/b.rs", "fn i4_chain_gone() {}\n");
    fx.write("src/c.rs", "fn i4_chain_roam() { i4_chain_old(); }\n");
    assert_eq!(fx.reindex().files_indexed, 3);

    // SERVE baseline: exact pre-chain hit sets, including the caller edge.
    assert_eq!(fx.def_files("i4_chain_old"), vec!["src/a.rs".to_string()]);
    assert_eq!(fx.def_files("i4_chain_gone"), vec!["src/b.rs".to_string()]);
    assert_eq!(
        fx.def_files("i4_chain_roam"),
        vec!["src/c.rs".to_string()]
    );
    assert_eq!(
        fx.caller_files("i4_chain_old"),
        vec!["src/c.rs".to_string()]
    );
    let snap = fx.snapshot();

    // CHAIN: one of every delta class lands before any refresh runs.
    fx.write("src/a.rs", "fn i4_chain_new() { let _t = \"tokchainnew\"; }\n");
    set_mtime_checked(&abs_a, WHOLE_SECOND_T1);
    std::fs::remove_file(fx.root.join("src/b.rs")).unwrap();
    std::fs::rename(fx.root.join("src/c.rs"), fx.root.join("src/z.rs")).unwrap();
    fx.write("src/d.rs", "fn i4_chain_added() {}\n");

    // DETECT: everything frozen; stale serve shows the pre-chain world.
    fx.assert_frozen(&snap);
    assert_eq!(
        fx.live_rs_files(),
        vec![
            "src/a.rs".to_string(),
            "src/d.rs".to_string(),
            "src/z.rs".to_string()
        ]
    );
    assert_eq!(fx.def_files("i4_chain_old"), vec!["src/a.rs".to_string()]);
    assert_eq!(fx.def_files("i4_chain_gone"), vec!["src/b.rs".to_string()]);
    assert!(fx.def_files("i4_chain_new").is_empty());
    assert!(fx.def_files("i4_chain_added").is_empty());

    // One refresh absorbs the whole chain.
    let stats = fx.reindex();
    assert_eq!(stats.files_indexed, 3, "a-modified + z-added + d-added");
    assert_eq!(stats.files_removed, 2, "b-deleted + c-moved");
    assert_eq!(fx.generation(), snap.generation + 5);
    assert_ne!(fx.writer_epoch(), snap.writer);
    assert_eq!(fx.counts().0, 3);

    // SERVE proves the exact converged hit sets.
    assert!(fx.def_files("i4_chain_old").is_empty());
    assert!(fx.literal_spots("tokchainold").is_empty());
    assert!(fx.def_files("i4_chain_gone").is_empty());
    assert_eq!(fx.def_files("i4_chain_new"), vec!["src/a.rs".to_string()]);
    assert_eq!(
        fx.literal_spots("tokchainnew"),
        vec![("src/a.rs".to_string(), 1)]
    );
    assert_eq!(
        fx.def_files("i4_chain_added"),
        vec!["src/d.rs".to_string()]
    );
    // The renamed file carries its caller edge to the new path, still
    // pointing at the (now retired) callee name.
    assert_eq!(
        fx.def_files("i4_chain_roam"),
        vec!["src/z.rs".to_string()]
    );
    assert_eq!(
        fx.caller_files("i4_chain_old"),
        vec!["src/z.rs".to_string()]
    );
}

// ---- drill: rapid succession converges ----

#[test]
fn drill_rapid_succession_change_refresh_change_refresh() {
    let fx = Fx::new();
    let abs = fx.write("src/a.rs", "fn i4_rapid_v1() { let _t = \"tokrapv1\"; }\n");
    set_mtime_checked(&abs, WHOLE_SECOND_T0);
    fx.write("src/b.rs", "fn i4_rapid_anchor() {}\n");
    assert_eq!(fx.reindex().files_indexed, 2);
    assert_eq!(fx.def_files("i4_rapid_v1"), vec!["src/a.rs".to_string()]);
    let snap0 = fx.snapshot();

    // Cycle 1: modify -> detect -> refresh -> serve.
    fx.write("src/a.rs", "fn i4_rapid_v2() { let _t = \"tokrapv2\"; }\n");
    set_mtime_checked(&abs, WHOLE_SECOND_T1);
    fx.assert_frozen(&snap0);
    assert_eq!(fx.def_files("i4_rapid_v1"), vec!["src/a.rs".to_string()]);
    assert!(fx.def_files("i4_rapid_v2").is_empty());
    assert_eq!(fx.reindex().files_indexed, 1);
    assert_eq!(fx.generation(), snap0.generation + 1);
    assert!(fx.def_files("i4_rapid_v1").is_empty());
    assert_eq!(fx.def_files("i4_rapid_v2"), vec!["src/a.rs".to_string()]);
    assert_eq!(
        fx.literal_spots("tokrapv2"),
        vec![("src/a.rs".to_string(), 1)]
    );
    let snap1 = fx.snapshot();

    // Cycle 2, immediately: modify again -> detect -> refresh -> serve.
    fx.write("src/a.rs", "fn i4_rapid_v3() { let _t = \"tokrapv3\"; }\n");
    set_mtime_checked(&abs, WHOLE_SECOND_T2);
    fx.assert_frozen(&snap1);
    assert_eq!(
        fx.def_files("i4_rapid_v2"),
        vec!["src/a.rs".to_string()],
        "stale serve: cycle-1 content lingers before the cycle-2 refresh"
    );
    assert!(fx.def_files("i4_rapid_v3").is_empty());
    assert_eq!(fx.reindex().files_indexed, 1);
    assert_eq!(fx.generation(), snap1.generation + 1);

    // SERVE proves convergence on the latest content only.
    assert!(fx.def_files("i4_rapid_v1").is_empty());
    assert!(fx.def_files("i4_rapid_v2").is_empty());
    assert_eq!(fx.def_files("i4_rapid_v3"), vec!["src/a.rs".to_string()]);
    assert_eq!(
        fx.literal_spots("tokrapv3"),
        vec![("src/a.rs".to_string(), 1)]
    );
    assert!(fx.literal_spots("tokrapv1").is_empty());
    assert!(fx.literal_spots("tokrapv2").is_empty());
    assert_eq!(
        fx.def_files("i4_rapid_anchor"),
        vec!["src/b.rs".to_string()]
    );
    assert_eq!(fx.counts().0, 2);
}

// ---- drill: delete then re-add the same path ----

#[test]
fn drill_delete_then_readd_same_path_end_to_end() {
    let fx = Fx::new();
    fx.write("src/a.rs", "fn i4_first_life() {}\n");
    fx.write("src/b.rs", "fn i4_res_anchor() {}\n");
    assert_eq!(fx.reindex().files_indexed, 2);
    assert_eq!(
        fx.def_files("i4_first_life"),
        vec!["src/a.rs".to_string()]
    );
    let snap0 = fx.snapshot();

    // First life ends: delete -> detect -> refresh -> serve proves removal.
    std::fs::remove_file(fx.root.join("src/a.rs")).unwrap();
    fx.assert_frozen(&snap0);
    let stats = fx.reindex();
    assert_eq!(stats.files_removed, 1);
    assert_eq!(stats.files_indexed, 0);
    assert_eq!(fx.generation(), snap0.generation + 1);
    assert!(fx.def_files("i4_first_life").is_empty());
    assert_eq!(fx.counts().0, 1);
    let snap1 = fx.snapshot();

    // Second life: re-add the SAME path with different content.
    fx.write("src/a.rs", "fn i4_second_life() { let _t = \"toksecond\"; }\n");
    fx.assert_frozen(&snap1);
    assert_eq!(
        fx.live_rs_files().len(),
        snap1.counts.0 + 1,
        "the live tree outruns the stored file count: stale"
    );
    assert!(
        fx.def_files("i4_second_life").is_empty(),
        "stale serve: the resurrected path is invisible before refresh"
    );

    // REFRESH + SERVE prove the exact reborn hit sets.
    let stats = fx.reindex();
    assert_eq!(stats.files_indexed, 1);
    assert_eq!(stats.files_removed, 0);
    assert_eq!(fx.generation(), snap1.generation + 1);
    assert_ne!(fx.writer_epoch(), snap1.writer);
    assert_eq!(fx.counts().0, 2);
    assert_eq!(
        fx.def_files("i4_second_life"),
        vec!["src/a.rs".to_string()]
    );
    assert_eq!(
        fx.literal_spots("toksecond"),
        vec![("src/a.rs".to_string(), 1)]
    );
    assert!(fx.def_files("i4_first_life").is_empty());
    assert_eq!(fx.def_files("i4_res_anchor"), vec!["src/b.rs".to_string()]);
}
