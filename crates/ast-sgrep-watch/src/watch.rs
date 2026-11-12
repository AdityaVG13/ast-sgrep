//! Incremental file-watch index updates.

use anyhow::Context;
use ast_sgrep_core::paths::canonicalize_path;
use ast_sgrep_core::semantic_ivf::SEMANTIC_IVF_FILE;
use ast_sgrep_core::store::{INDEX_DB, INDEX_DIR};
use ast_sgrep_core::tantivy_index::LEXICAL_DB;
use ast_sgrep_core::{IndexOptions, Indexer, MAX_INCREMENTAL_PATHS};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

const WATCH_QUEUE_CAPACITY: usize = 256;
const WATCH_MAX_LATENCY_WINDOWS: u32 = 3;

fn queue_event<T>(tx: &SyncSender<T>, full_rescan: &AtomicBool, event: T) {
    if matches!(
        tx.try_send(event),
        Err(TrySendError::Full(_) | TrySendError::Disconnected(_))
    ) {
        full_rescan.store(true, Ordering::SeqCst);
    }
}

/// A full scan covers every event already queued. Clear the old overflow marker
/// before draining so any event dropped concurrently with the scan requests a
/// follow-up scan instead of being mistaken for covered work.
fn begin_full_scan<T>(rx: &Receiver<T>, full_rescan: &AtomicBool) {
    full_rescan.store(false, Ordering::SeqCst);
    while rx.try_recv().is_ok() {}
}

fn take_full_rescan(full_rescan: &AtomicBool) -> bool {
    full_rescan.swap(false, Ordering::SeqCst)
}

fn schedule_deadline(deadline: &mut Option<Instant>, candidate: Instant) {
    *deadline = Some(deadline.map_or(candidate, |current| current.min(candidate)));
}

fn request_full_scan(full: &mut bool, deadline: &mut Option<Instant>, debounce: Duration) {
    *full = true;
    schedule_deadline(deadline, Instant::now() + debounce);
}

fn next_event_wait(
    debounce: Duration,
    full_deadline: Option<Instant>,
    now: Instant,
) -> Option<Duration> {
    match full_deadline {
        Some(deadline) if deadline <= now => None,
        // Flush after one quiet period, or at the first-event max-latency
        // deadline under sustained traffic, whichever arrives first.
        Some(deadline) => Some(debounce.min(deadline.duration_since(now))),
        None => Some(debounce),
    }
}

/// SQLite engine sidecars beside a database file: write-ahead log, shared
/// memory, legacy rollback journal. Stable SQLite contract; the recovery
/// lock suffix is core's (`index_recovery::recovery_lock` builds
/// `<db>.reindex.lock`, still private — one line here until core publishes
/// it). If core ever publishes its `SQLITE_SIDECAR_SUFFIXES` table, delete
/// this one and reference core's.
const SQLITE_SIDECAR_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];
const REINDEX_LOCK_SUFFIX: &str = ".reindex.lock";

fn sqlite_artifact(name: &str, database: &str) -> bool {
    name.strip_prefix(database).is_some_and(|suffix| {
        suffix.is_empty()
            || SQLITE_SIDECAR_SUFFIXES.contains(&suffix)
            || suffix == REINDEX_LOCK_SUFFIX
    })
}

/// Canonicalize for identity comparison: symlinked checkouts and macOS
/// `/tmp`→`/private/tmp` aliases must agree with whatever spelling `notify`
/// reports. `follow_symlinks = false` keeps not-yet-created index keys
/// stable instead of erroring (wave-1 `core::paths` semantics).
fn canonical_identity(path: &Path) -> PathBuf {
    canonicalize_path(path, false).unwrap_or_else(|_| path.to_path_buf())
}

fn is_watch_self_event(paths: &[PathBuf], root: &Path, index_db: &Path) -> bool {
    if paths.is_empty() {
        return false;
    }
    let root = canonical_identity(root);
    let index_db = canonical_identity(index_db);
    // Default index home comes from core's `INDEX_DIR`, not a hardcoded
    // ".asgrep" (H2): if core moves the default, the filter follows.
    let default_index_dir = root.join(INDEX_DIR);
    let index_dir = index_db.parent();
    let index_name = index_db.file_name().and_then(|name| name.to_str());
    // Lexical sidecar basename mirrors `tantivy_index::sidecar_path`: the
    // default `index.db` shares one `lexical.db`; a custom basename owns
    // `<name>.lexical.db` so two indexes in one directory never share one.
    // (H1: the old hardcoded `"lexical.db"` misclassified both directions
    // for custom DB names.)
    let lexical_name = index_db
        .file_name()
        .and_then(|name| name.to_str())
        .map(|db| {
            if db == INDEX_DB {
                LEXICAL_DB.to_owned()
            } else {
                format!("{db}.lexical.db")
            }
        })
        .unwrap_or_else(|| LEXICAL_DB.to_owned());
    // Tmp-file prefixes derive from core's artifact constants, not from
    // hardcoded `".semantic.ivf."` / `".writer_generation."` literals (H1).
    let semantic_tmp_prefix = format!(".{SEMANTIC_IVF_FILE}.");
    let generation_tmp_prefix = format!(".{}.", ast_sgrep_core::WRITER_GENERATION_FILE);
    // Chunk-id sidecar beside the IVF file (`semantic.ivf` -> `.ids`,
    // mirroring `semantic_ivf::semantic_ivf_ids_path`).
    let semantic_ids = format!("{SEMANTIC_IVF_FILE}.ids");
    paths.iter().all(|path| {
        let path = canonical_identity(path);
        if path.starts_with(&default_index_dir) {
            return true;
        }
        if path.parent() != index_dir {
            return false;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return path == index_db;
        };
        index_name.is_some_and(|database| sqlite_artifact(name, database))
            || sqlite_artifact(name, &lexical_name)
            || name == SEMANTIC_IVF_FILE
            || name == semantic_ids
            || (name.starts_with(semantic_tmp_prefix.as_str()) && name.ends_with(".tmp"))
            || name == ast_sgrep_core::WRITER_GENERATION_FILE
            || (name.starts_with(generation_tmp_prefix.as_str()) && name.ends_with(".tmp"))
    })
}

pub fn run_watch(opts: IndexOptions, debounce_ms: u64) -> anyhow::Result<()> {
    use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
    use std::collections::HashSet;
    use std::sync::mpsc::{self, RecvTimeoutError};
    let root = opts.root.clone();
    let index_db = ast_sgrep_core::index_db_path(&root, opts.index_path.as_deref());
    let (tx, rx) = mpsc::sync_channel(WATCH_QUEUE_CAPACITY);
    let full_rescan = Arc::new(AtomicBool::new(false));
    let callback_rescan = Arc::clone(&full_rescan);
    let callback_root = root.clone();
    let callback_index_db = index_db.clone();
    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<notify::Event>| {
            // notify callbacks must not block behind indexing. A bounded queue
            // caps memory; overflow coalesces into one correctness scan.
            if res.as_ref().is_ok_and(|event| {
                is_watch_self_event(&event.paths, &callback_root, &callback_index_db)
            }) {
                return;
            }
            queue_event(&tx, &callback_rescan, res);
        },
        Config::default(),
    )
    .context("failed to create file watcher")?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .context("failed to watch project root")?;
    eprintln!(
        "[asgrep] watching {} (debounce {debounce_ms}ms)",
        root.display()
    );
    let mut indexer = Indexer::new(opts)?;
    begin_full_scan(&rx, &full_rescan);
    let initial = indexer.index_all()?;
    eprintln!(
        "[asgrep] initial index: {} files indexed, {} skipped",
        initial.files_indexed, initial.files_skipped
    );
    let debounce = Duration::from_millis(debounce_ms);
    let max_latency = debounce.saturating_mul(WATCH_MAX_LATENCY_WINDOWS);
    let mut pending = HashSet::new();
    let mut full = take_full_rescan(&full_rescan);
    let mut flush_deadline = full.then(|| Instant::now() + debounce);
    loop {
        if take_full_rescan(&full_rescan) {
            pending.clear();
            request_full_scan(&mut full, &mut flush_deadline, debounce);
        }
        // Quiet-period debounce remains the fast path. The first pending event
        // also starts a wall-clock deadline so sustained traffic cannot defer
        // indexing forever, even when one path is repeatedly overwritten.
        let event = match next_event_wait(debounce, flush_deadline, Instant::now()) {
            Some(wait) => rx.recv_timeout(wait),
            None => Err(RecvTimeoutError::Timeout),
        };
        match event {
            Ok(Ok(ev)) => match ev.kind {
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {
                    // Existing directories may arrive populated after a rename,
                    // and oversized event bursts may have dropped intermediate
                    // events. In either case exact-path updates cannot prove a
                    // complete state, so collapse to one correctness scan.
                    if ev.paths.iter().any(|path| path.is_dir())
                        || pending.len().saturating_add(ev.paths.len()) > MAX_INCREMENTAL_PATHS
                    {
                        pending.clear();
                        request_full_scan(&mut full, &mut flush_deadline, debounce);
                    } else if !full {
                        let was_empty = pending.is_empty();
                        pending.extend(ev.paths);
                        if was_empty && !pending.is_empty() {
                            schedule_deadline(&mut flush_deadline, Instant::now() + max_latency);
                        }
                    }
                }
                EventKind::Other | EventKind::Any => {
                    pending.clear();
                    request_full_scan(&mut full, &mut flush_deadline, debounce);
                }
                _ => {}
            },
            Ok(Err(e)) => {
                eprintln!("[asgrep] watch error: {e}; scheduling a full rescan");
                pending.clear();
                request_full_scan(&mut full, &mut flush_deadline, debounce);
            }
            Err(RecvTimeoutError::Timeout) if full => {
                begin_full_scan(&rx, &full_rescan);
                let s = indexer.index_all()?;
                eprintln!(
                    "[asgrep] full rescan: {} updated, {} skipped, {} removed",
                    s.files_indexed, s.files_skipped, s.files_removed
                );
                // Overflow during this scan cannot be represented by the
                // drained queue and therefore requires one more scan.
                full = take_full_rescan(&full_rescan);
                flush_deadline = full.then(|| Instant::now() + debounce);
            }
            Err(RecvTimeoutError::Timeout) if !pending.is_empty() => {
                let paths: Vec<_> = pending.drain().collect();
                let t0 = Instant::now();
                let s = indexer.update_paths(&paths)?;
                let ms = t0.elapsed().as_secs_f64() * 1000.0;
                if s.files_indexed + s.files_removed + s.files_failed > 0 {
                    eprintln!(
                        "[asgrep] updated {} file(s) ({} removed, {} skipped) in {ms:.3}ms",
                        s.files_indexed, s.files_removed, s.files_skipped
                    );
                }
                // Publish the sidecars inline, not only at quiescence. A
                // sustained storm never yields a quiet window, so a
                // quiescence-only flush leaves every mid-storm read failing
                // closed on a stale bundle forever. Backpressure is unchanged:
                // when batches arrive faster than update+flush, the bounded
                // queue overflows into a coalesced full rescan as before.
                if indexer.deferred_rebuilds_pending() {
                    indexer.flush_deferred_rebuilds()?;
                }
                flush_deadline = None;
            }
            Err(RecvTimeoutError::Timeout) if indexer.deferred_rebuilds_pending() => {
                let t0 = Instant::now();
                indexer.flush_deferred_rebuilds()?;
                eprintln!(
                    "[asgrep] deferred rebuilds done in {:.1}ms",
                    t0.elapsed().as_secs_f64() * 1000.0
                );
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                anyhow::bail!("file watcher event channel disconnected")
            }
        }
    }
}

#[cfg(test)]
fn case_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("asgrep-watch-{}-{name}", std::process::id()))
}

#[cfg(test)]
mod self_event_tests {
    use super::{case_root, is_watch_self_event, sqlite_artifact};
    use std::path::PathBuf;

    #[test]
    fn empty_paths_are_not_self_events() {
        let root = case_root("empty");
        let index_db = root.join(".asgrep").join("index.db");
        assert!(!is_watch_self_event(&[], &root, &index_db));
    }

    #[test]
    fn main_db_and_sqlite_sidecars_are_self_events() {
        let root = case_root("main");
        let dir = root.join(".asgrep");
        let index_db = dir.join("index.db");
        for name in [
            "index.db",
            "index.db-wal",
            "index.db-shm",
            "index.db-journal",
            "index.db.reindex.lock",
        ] {
            assert!(
                is_watch_self_event(std::slice::from_ref(&dir.join(name)), &root, &index_db),
                "{name} should be a self event"
            );
        }
    }

    #[test]
    fn source_files_are_not_self_events() {
        let root = case_root("src");
        let index_db = root.join(".asgrep").join("index.db");
        assert!(!is_watch_self_event(
            std::slice::from_ref(&root.join("src").join("main.rs")),
            &root,
            &index_db
        ));
    }

    #[test]
    fn nested_index_subdir_is_not_self_event() {
        // Flat layout (P3): only the index dir itself is covered, so with a
        // custom index home a nested subdirectory is a user path.
        let root = case_root("nested");
        let dir = root.join("idx");
        let index_db = dir.join("store.db");
        assert!(!is_watch_self_event(
            std::slice::from_ref(&dir.join("nested").join("x.db")),
            &root,
            &index_db
        ));
    }

    #[test]
    fn custom_db_lexical_sidecar_is_self_event() {
        // `tantivy_index::sidecar_path`: a custom `custom.db` owns
        // `custom.db.lexical.db`, not the default `lexical.db`.
        let root = case_root("custom");
        let dir = root.join("idx");
        let index_db = dir.join("custom.db");
        for name in ["custom.db.lexical.db", "custom.db.lexical.db-wal"] {
            assert!(
                is_watch_self_event(std::slice::from_ref(&dir.join(name)), &root, &index_db),
                "{name} should be a self event"
            );
        }
    }

    #[test]
    fn bare_lexical_name_with_custom_db_is_not_self_event() {
        // With a custom DB basename, a bare `lexical.db` is a user file.
        let root = case_root("custom-bare");
        let dir = root.join("idx");
        let index_db = dir.join("custom.db");
        assert!(!is_watch_self_event(
            std::slice::from_ref(&dir.join("lexical.db")),
            &root,
            &index_db
        ));
    }

    #[test]
    fn semantic_ids_sidecar_is_self_event() {
        // Custom index home so the name allowlist (not the default-dir arm)
        // decides: the fingerprint-bound `.ids` sidecar is a core artifact.
        let root = case_root("ids");
        let dir = root.join("idx");
        let index_db = dir.join("index.db");
        for name in ["semantic.ivf", "semantic.ivf.ids"] {
            assert!(
                is_watch_self_event(std::slice::from_ref(&dir.join(name)), &root, &index_db),
                "{name} should be a self event"
            );
        }
    }

    #[test]
    fn unclean_index_db_spelling_still_matches() {
        // Identity is canonical (`core::paths`), not spelling: an index_db
        // spelled with `.` segments still owns its sidecars.
        let root = case_root("unclean");
        let index_db = root.join("custom").join(".").join("store.db");
        assert!(is_watch_self_event(
            std::slice::from_ref(&root.join("custom").join("store.db-wal")),
            &root,
            &index_db
        ));
    }

    #[test]
    fn writer_generation_files_are_self_events() {
        let root = case_root("wg");
        let dir = root.join(".asgrep");
        let index_db = dir.join("index.db");
        assert!(is_watch_self_event(
            std::slice::from_ref(&dir.join("writer_generation")),
            &root,
            &index_db
        ));
        assert!(is_watch_self_event(
            std::slice::from_ref(&dir.join(".writer_generation.123.4.tmp")),
            &root,
            &index_db
        ));
    }

    #[test]
    fn semantic_tmp_files_are_self_events() {
        assert!(sqlite_artifact("semantic.ivf", "semantic.ivf"));
        let root = case_root("semtmp");
        let dir = root.join(".asgrep");
        let index_db = dir.join("index.db");
        assert!(is_watch_self_event(
            std::slice::from_ref(&dir.join(".semantic.ivf.123.4.tmp")),
            &root,
            &index_db
        ));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_falls_back_to_identity() {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::ffi::OsStringExt;
        // Custom index home so the default-dir arm cannot swallow the case:
        // undecodability is decided at the parent/name step (P2).
        let root = case_root("nonutf8");
        let dir = root.join("idx");
        let mut raw = dir.join("index.db").into_os_string().into_vec();
        raw.push(0xFF);
        let undecodable_db = PathBuf::from(std::ffi::OsStr::from_bytes(&raw));
        // The undecodable path IS the db spelling: identity holds.
        assert!(is_watch_self_event(
            std::slice::from_ref(&undecodable_db),
            &root,
            &undecodable_db
        ));
        raw.push(b'x');
        let other = PathBuf::from(std::ffi::OsStr::from_bytes(&raw));
        assert!(!is_watch_self_event(
            std::slice::from_ref(&other),
            &root,
            &undecodable_db
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_root_spellings_agree() {
        // Identity is canonical (`core::paths`), not spelling: an event
        // reported via the physical root matches an index_db recorded via a
        // symlinked root spelling.
        let base = case_root("symlink");
        let real = base.join("real");
        std::fs::create_dir_all(real.join(".asgrep")).expect("seed dirs");
        let link = base.join("link");
        std::os::unix::fs::symlink(&real, &link).expect("symlink test setup");
        let via_link = link.join(".asgrep").join("index.db");
        let via_real = real.join(".asgrep").join("index.db-wal");
        assert!(is_watch_self_event(
            std::slice::from_ref(&via_real),
            &link,
            &via_link
        ));
    }
}

#[cfg(test)]
mod debounce_tests {
    use super::{next_event_wait, request_full_scan, schedule_deadline};
    use std::time::{Duration, Instant};

    #[test]
    fn schedule_deadline_sets_first_candidate() {
        let base = Instant::now();
        let mut deadline = None;
        schedule_deadline(&mut deadline, base);
        assert_eq!(deadline, Some(base));
    }

    #[test]
    fn schedule_deadline_keeps_earliest() {
        let early = Instant::now();
        let late = early + Duration::from_secs(10);
        let mut deadline = Some(early);
        schedule_deadline(&mut deadline, late);
        assert_eq!(deadline, Some(early));
    }

    #[test]
    fn schedule_deadline_adopts_earlier_candidate() {
        let late = Instant::now() + Duration::from_secs(10);
        let mut deadline = Some(late);
        let early = Instant::now();
        schedule_deadline(&mut deadline, early);
        assert_eq!(deadline, Some(early));
    }

    #[test]
    fn request_full_scan_sets_flag_and_deadline() {
        let debounce = Duration::from_millis(50);
        let mut full = false;
        let mut deadline = None;
        let before = Instant::now();
        request_full_scan(&mut full, &mut deadline, debounce);
        let after = Instant::now();
        assert!(full);
        let scheduled = deadline.expect("full scan must schedule a deadline");
        assert!(scheduled >= before + debounce);
        assert!(scheduled <= after + debounce);
    }

    #[test]
    fn request_full_scan_keeps_earlier_deadline() {
        // Under sustained traffic the first-event max-latency deadline must
        // survive a later full-scan request (min semantics).
        let debounce = Duration::from_secs(60);
        let mut full = false;
        let early = Instant::now() + Duration::from_millis(10);
        let mut deadline = Some(early);
        request_full_scan(&mut full, &mut deadline, debounce);
        assert!(full);
        assert_eq!(deadline, Some(early));
    }

    #[test]
    fn next_event_wait_without_deadline_waits_debounce() {
        let debounce = Duration::from_millis(100);
        let now = Instant::now();
        assert_eq!(next_event_wait(debounce, None, now), Some(debounce));
    }

    #[test]
    fn next_event_wait_at_or_past_deadline_flushes() {
        let debounce = Duration::from_millis(100);
        let now = Instant::now();
        assert_eq!(next_event_wait(debounce, Some(now), now), None);
        let past = now.checked_sub(Duration::from_millis(1)).unwrap();
        assert_eq!(next_event_wait(debounce, Some(past), now), None);
    }

    #[test]
    fn next_event_wait_caps_at_max_latency_deadline() {
        // Sustained traffic: flush at the first-event deadline even though
        // the quiet period has not elapsed.
        let debounce = Duration::from_millis(100);
        let now = Instant::now();
        let soon = now + Duration::from_millis(30);
        assert_eq!(
            next_event_wait(debounce, Some(soon), now),
            Some(Duration::from_millis(30))
        );
    }

    #[test]
    fn next_event_wait_caps_at_debounce_when_deadline_far() {
        let debounce = Duration::from_millis(100);
        let now = Instant::now();
        let far = now + Duration::from_millis(500);
        assert_eq!(next_event_wait(debounce, Some(far), now), Some(debounce));
    }
}

#[cfg(test)]
mod overflow_tests {
    use super::{begin_full_scan, queue_event, take_full_rescan};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;

    #[test]
    fn queue_event_delivers_without_rescan() {
        let (tx, rx) = mpsc::sync_channel(8);
        let full_rescan = AtomicBool::new(false);
        queue_event(&tx, &full_rescan, 1u32);
        queue_event(&tx, &full_rescan, 2u32);
        assert!(!full_rescan.load(Ordering::SeqCst));
        assert_eq!(rx.try_recv().unwrap(), 1);
        assert_eq!(rx.try_recv().unwrap(), 2);
    }

    #[test]
    fn queue_event_full_coalesces_to_rescan() {
        // A bounded queue caps memory; overflow collapses into one
        // correctness scan while preserving what already queued.
        let (tx, rx) = mpsc::sync_channel(1);
        let full_rescan = AtomicBool::new(false);
        tx.try_send(1u32).expect("seed the single slot");
        queue_event(&tx, &full_rescan, 2u32);
        assert!(full_rescan.load(Ordering::SeqCst));
        assert_eq!(rx.try_recv().unwrap(), 1);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn queue_event_disconnected_coalesces_to_rescan() {
        let (tx, rx) = mpsc::sync_channel(8);
        drop(rx);
        let full_rescan = AtomicBool::new(false);
        queue_event(&tx, &full_rescan, 1u32);
        assert!(full_rescan.load(Ordering::SeqCst));
    }

    #[test]
    fn begin_full_scan_clears_flag_and_drains() {
        // A full scan covers every queued event, so the old overflow marker
        // is cleared and the queue drained together.
        let (tx, rx) = mpsc::sync_channel(8);
        tx.try_send(1u32).unwrap();
        tx.try_send(2u32).unwrap();
        let full_rescan = AtomicBool::new(true);
        begin_full_scan(&rx, &full_rescan);
        assert!(!full_rescan.load(Ordering::SeqCst));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn overflow_after_begin_requests_followup() {
        // The marker is cleared before draining, so an event dropped
        // concurrently with the scan is not mistaken for covered work.
        let (tx, rx) = mpsc::sync_channel::<u32>(1);
        let full_rescan = AtomicBool::new(true);
        begin_full_scan(&rx, &full_rescan);
        assert!(!full_rescan.load(Ordering::SeqCst));
        tx.try_send(1u32).unwrap();
        queue_event(&tx, &full_rescan, 2u32);
        assert!(take_full_rescan(&full_rescan));
    }

    #[test]
    fn take_full_rescan_swaps_once() {
        let set = AtomicBool::new(true);
        assert!(take_full_rescan(&set));
        assert!(!take_full_rescan(&set));
        let unset = AtomicBool::new(false);
        assert!(!take_full_rescan(&unset));
    }
}

#[cfg(test)]
mod self_event_gap_tests {
    use super::{case_root, is_watch_self_event, sqlite_artifact};

    #[test]
    fn mixed_self_and_user_paths_are_not_self_events() {
        // The filter requires every path to be an index artifact (`all`):
        // one user path keeps the batch visible.
        let root = case_root("mixed");
        let dir = root.join(".asgrep");
        let index_db = dir.join("index.db");
        assert!(!is_watch_self_event(
            &[index_db.clone(), root.join("src").join("main.rs")],
            &root,
            &index_db
        ));
    }

    #[test]
    fn default_lexical_sidecar_is_self_event() {
        // Default `index.db` owns the bare `lexical.db` sidecar (contrast
        // with the custom-basename case pinned in `self_event_tests`).
        let root = case_root("lex");
        let dir = root.join(".asgrep");
        let index_db = dir.join("index.db");
        for name in ["lexical.db", "lexical.db-wal", "lexical.db-shm"] {
            assert!(
                is_watch_self_event(std::slice::from_ref(&dir.join(name)), &root, &index_db),
                "{name} should be a self event"
            );
        }
    }

    #[test]
    fn default_index_dir_covers_unknown_names() {
        // Anything under the default index home is ours, even without an
        // allowlisted name.
        let root = case_root("dir");
        let dir = root.join(".asgrep");
        let index_db = dir.join("index.db");
        assert!(is_watch_self_event(
            std::slice::from_ref(&dir.join("scratch.tmp")),
            &root,
            &index_db
        ));
    }

    #[test]
    fn sqlite_artifact_rejects_lookalikes() {
        assert!(sqlite_artifact("index.db", "index.db"));
        assert!(sqlite_artifact("index.db-wal", "index.db"));
        assert!(sqlite_artifact("index.db.reindex.lock", "index.db"));
        assert!(!sqlite_artifact("index.db-foo", "index.db"));
        assert!(!sqlite_artifact("other.db", "index.db"));
        assert!(!sqlite_artifact("lexical.db", "index.db"));
    }
}
