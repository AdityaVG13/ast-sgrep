//! Watch command and debounce batching.

use crate::{index_options, Cli};
use anyhow::Context;
use ast_sgrep_core::{index_db_path, Indexer};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Default)]
struct WatchBatch {
    pending: std::collections::HashSet<PathBuf>,
    full: bool,
    first_pending: Option<Instant>,
}

enum WatchWork {
    Full,
    Paths(Vec<PathBuf>),
}

impl WatchBatch {
    fn record_paths(&mut self, paths: Vec<PathBuf>) {
        self.pending.extend(paths);
        self.start_deadline();
    }

    fn request_full(&mut self) {
        self.full = true;
        self.start_deadline();
    }

    fn start_deadline(&mut self) {
        if self.first_pending.is_none() {
            self.first_pending = Some(Instant::now());
        }
    }

    fn due(&self, max_latency: std::time::Duration) -> bool {
        self.first_pending
            .is_some_and(|started| started.elapsed() >= max_latency)
    }

    fn take(&mut self) -> Option<WatchWork> {
        self.first_pending = None;
        if std::mem::take(&mut self.full) {
            self.pending.clear();
            Some(WatchWork::Full)
        } else if self.pending.is_empty() {
            None
        } else {
            Some(WatchWork::Paths(self.pending.drain().collect()))
        }
    }
}

fn is_watch_self_event(paths: &[PathBuf], root: &Path, index_db: &Path) -> bool {
    if paths.is_empty() {
        return false;
    }
    let default_index_dir = root.join(".asgrep");
    let semantic_ivf = ast_sgrep_core::semantic_ivf::semantic_ivf_path(index_db);
    let sidecar = |base: &Path, suffix: &str| {
        let mut path = base.as_os_str().to_owned();
        path.push(suffix);
        PathBuf::from(path)
    };
    let wal = sidecar(index_db, "-wal");
    let shm = sidecar(index_db, "-shm");
    // Tantivy lexical.db (+ wal/shm) lives beside the sqlite index, including
    // under a custom --index-path parent (jsfn / thermos P2).
    let lexical = index_db
        .parent()
        .map(|p| p.join(ast_sgrep_core::tantivy_index::LEXICAL_DB));
    let lexical_wal = lexical.as_ref().map(|p| sidecar(p, "-wal"));
    let lexical_shm = lexical.as_ref().map(|p| sidecar(p, "-shm"));
    paths.iter().all(|path| {
        path.starts_with(&default_index_dir)
            || path == index_db
            || path == &semantic_ivf
            || path == &wal
            || path == &shm
            || lexical.as_ref().is_some_and(|p| path == p)
            || lexical_wal.as_ref().is_some_and(|p| path == p)
            || lexical_shm.as_ref().is_some_and(|p| path == p)
    })
}

fn flush_watch_work(indexer: &mut Indexer, work: WatchWork, reason: &str) -> anyhow::Result<()> {
    match work {
        WatchWork::Full => {
            let s = indexer.index_all()?;
            eprintln!(
                "[asgrep] full rescan ({reason}): {} updated, {} skipped, {} removed",
                s.files_indexed, s.files_skipped, s.files_removed
            );
        }
        WatchWork::Paths(paths) => {
            let t0 = Instant::now();
            let s = indexer.update_paths(&paths)?;
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            if s.files_indexed + s.files_removed + s.files_failed > 0 {
                eprintln!(
                    "[asgrep] updated {} file(s) ({} removed, {} skipped) in {ms:.3}ms ({reason})",
                    s.files_indexed, s.files_removed, s.files_skipped
                );
            }
        }
    }
    // update_paths may defer IVF/Tantivy sidecars. Complete them with the same
    // bounded batch so sustained events cannot postpone search freshness.
    if indexer.deferred_rebuilds_pending() {
        let t0 = Instant::now();
        indexer.flush_deferred_rebuilds()?;
        eprintln!(
            "[asgrep] deferred rebuilds done in {:.1}ms ({reason})",
            t0.elapsed().as_secs_f64() * 1000.0
        );
    }
    Ok(())
}

pub(crate) fn run_watch(root: &Path, cli: &Cli, debounce_ms: u64) -> anyhow::Result<()> {
    use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::Duration;
    let opts = index_options(root, cli);
    let root = opts.root.clone();
    let index_db = index_db_path(&root, opts.index_path.as_deref());
    let (tx, rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
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
    let initial = indexer.index_all()?;
    eprintln!(
        "[asgrep] initial index: {} files indexed, {} skipped",
        initial.files_indexed, initial.files_skipped
    );
    let debounce = Duration::from_millis(debounce_ms);
    // Max-latency bound: force a flush after this much wall time regardless of
    // event arrival rate. Without it, rx.recv_timeout(debounce) resets on every
    // event, so an event stream faster than debounce means Timeout never fires,
    // pending grows unbounded, and no indexing ever happens (bead ast-sgrep-jsfn).
    // k=3 gives the quiet-gap coalescer room to batch while bounding staleness.
    let max_latency = debounce * 3;
    let mut batch = WatchBatch::default();
    // Filter writes to both the default index directory and custom index files.
    // Empty-path Any/Other events are full-rescan signals, never self-events.
    let is_self_event = |paths: &[PathBuf]| is_watch_self_event(paths, &root, &index_db);
    loop {
        match rx.recv_timeout(debounce) {
            Ok(event) => {
                match event {
                    Ok(ev) if !is_self_event(&ev.paths) => match ev.kind {
                        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {
                            batch.record_paths(ev.paths);
                        }
                        EventKind::Other | EventKind::Any => batch.request_full(),
                        _ => {}
                    },
                    Ok(_) => {}
                    Err(e) => eprintln!("[asgrep] watch error: {e}"),
                }
                // Check after every event, including watcher errors and ignored
                // self-events, so no sustained event stream can starve old work.
                if batch.due(max_latency) {
                    if let Some(work) = batch.take() {
                        flush_watch_work(&mut indexer, work, "max-latency bound")?;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some(work) = batch.take() {
                    flush_watch_work(&mut indexer, work, "debounce quiet period")?;
                } else if indexer.deferred_rebuilds_pending() {
                    let t0 = Instant::now();
                    indexer.flush_deferred_rebuilds()?;
                    eprintln!(
                        "[asgrep] deferred rebuilds done in {:.1}ms",
                        t0.elapsed().as_secs_f64() * 1000.0
                    );
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                // The producer is gone, but all work already accepted by the
                // receiver must become searchable before watch exits.
                if let Some(work) = batch.take() {
                    flush_watch_work(&mut indexer, work, "watcher disconnect")?;
                }
                if indexer.deferred_rebuilds_pending() {
                    indexer.flush_deferred_rebuilds()?;
                }
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod watch_batch_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn empty_paths_are_not_self_events() {
        assert!(!is_watch_self_event(
            &[],
            Path::new("/repo"),
            Path::new("/repo/custom/index.sqlite")
        ));
    }

    #[test]
    fn custom_index_and_sqlite_sidecars_are_self_events() {
        let root = Path::new("/repo");
        let db = Path::new("/repo/custom/index.sqlite");
        assert!(is_watch_self_event(
            &[PathBuf::from("/repo/custom/index.sqlite-wal")],
            root,
            db
        ));
        assert!(is_watch_self_event(
            &[PathBuf::from("/repo/custom/lexical.db")],
            root,
            db
        ));
        assert!(is_watch_self_event(
            &[PathBuf::from("/repo/custom/lexical.db-wal")],
            root,
            db
        ));
        assert!(is_watch_self_event(
            &[PathBuf::from("/repo/custom/semantic.ivf")],
            root,
            db
        ));
        assert!(!is_watch_self_event(
            &[PathBuf::from("/repo/src/lib.rs")],
            root,
            db
        ));
    }

    #[test]
    fn full_rescan_events_start_and_obey_max_latency() {
        let mut batch = WatchBatch::default();
        batch.request_full();
        batch.first_pending = Some(Instant::now() - Duration::from_millis(10));
        assert!(batch.due(Duration::from_millis(5)));
        assert!(matches!(batch.take(), Some(WatchWork::Full)));
        assert!(batch.take().is_none());
    }

    #[test]
    fn full_rescan_supersedes_incremental_paths() {
        let mut batch = WatchBatch::default();
        batch.record_paths(vec![PathBuf::from("src/lib.rs")]);
        batch.request_full();
        assert!(matches!(batch.take(), Some(WatchWork::Full)));
        assert!(batch.pending.is_empty());
    }

    #[test]
    fn disconnect_take_preserves_pending_paths() {
        let mut batch = WatchBatch::default();
        batch.record_paths(vec![PathBuf::from("src/lib.rs")]);
        let Some(WatchWork::Paths(paths)) = batch.take() else {
            panic!("pending paths must be flushable before disconnect");
        };
        assert_eq!(paths, [PathBuf::from("src/lib.rs")]);
    }
}
