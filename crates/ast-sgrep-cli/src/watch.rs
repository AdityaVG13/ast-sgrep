//! Incremental file-watch index updates.

use crate::{index_options, Cli};
use anyhow::Context;
use ast_sgrep_core::{Indexer, MAX_INCREMENTAL_PATHS};
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

fn sqlite_artifact(name: &str, database: &str) -> bool {
    name.strip_prefix(database)
        .is_some_and(|suffix| matches!(suffix, "" | "-wal" | "-shm" | "-journal" | ".reindex.lock"))
}

fn is_watch_self_event(paths: &[PathBuf], root: &Path, index_db: &Path) -> bool {
    if paths.is_empty() {
        return false;
    }
    let default_index_dir = root.join(".asgrep");
    let index_dir = index_db.parent();
    let index_name = index_db.file_name().and_then(|name| name.to_str());
    paths.iter().all(|path| {
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
            || sqlite_artifact(name, "lexical.db")
            || name == "semantic.ivf"
            || (name.starts_with(".semantic.ivf.") && name.ends_with(".tmp"))
            || name == ast_sgrep_core::WRITER_GENERATION_FILE
            || (name.starts_with(".writer_generation.") && name.ends_with(".tmp"))
    })
}

pub(crate) fn run_watch(root: &Path, cli: &Cli, debounce_ms: u64) -> anyhow::Result<()> {
    use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
    use std::collections::HashSet;
    use std::sync::mpsc::{self, RecvTimeoutError};
    let opts = index_options(root, cli);
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

