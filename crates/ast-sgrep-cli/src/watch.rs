//! Incremental file-watch index updates.

use crate::{index_options, Cli};
use anyhow::Context;
use ast_sgrep_core::{Indexer, MAX_INCREMENTAL_PATHS};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

const WATCH_QUEUE_CAPACITY: usize = 256;

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

fn request_full_scan(full: &mut bool, deadline: &mut Option<Instant>, debounce: Duration) {
    if !*full {
        *full = true;
        *deadline = Some(Instant::now() + debounce);
    }
}

fn next_event_wait(
    debounce: Duration,
    full_deadline: Option<Instant>,
    now: Instant,
) -> Option<Duration> {
    match full_deadline {
        Some(deadline) if deadline <= now => None,
        Some(deadline) => Some(deadline.duration_since(now)),
        None => Some(debounce),
    }
}

pub(crate) fn run_watch(root: &Path, cli: &Cli, debounce_ms: u64) -> anyhow::Result<()> {
    use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
    use std::collections::HashSet;
    use std::sync::mpsc::{self, RecvTimeoutError};
    let opts = index_options(root, cli);
    let root = opts.root.clone();
    let (tx, rx) = mpsc::sync_channel(WATCH_QUEUE_CAPACITY);
    let full_rescan = Arc::new(AtomicBool::new(false));
    let callback_rescan = Arc::clone(&full_rescan);
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            // notify callbacks must not block behind indexing. A bounded queue
            // caps memory; overflow coalesces into one correctness scan.
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
    let mut pending = HashSet::new();
    let mut full = take_full_rescan(&full_rescan);
    let mut full_deadline = full.then(|| Instant::now() + debounce);
    loop {
        if take_full_rescan(&full_rescan) {
            pending.clear();
            request_full_scan(&mut full, &mut full_deadline, debounce);
        }
        // Once exact events have been lost, do not let a permanently busy
        // queue postpone the corrective scan forever. Ordinary incremental
        // events retain quiet-period debounce semantics.
        let event = match next_event_wait(debounce, full_deadline, Instant::now()) {
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
                        request_full_scan(&mut full, &mut full_deadline, debounce);
                    } else if !full {
                        pending.extend(ev.paths);
                    }
                }
                EventKind::Other | EventKind::Any => {
                    pending.clear();
                    request_full_scan(&mut full, &mut full_deadline, debounce);
                }
                _ => {}
            },
            Ok(Err(e)) => {
                eprintln!("[asgrep] watch error: {e}; scheduling a full rescan");
                pending.clear();
                request_full_scan(&mut full, &mut full_deadline, debounce);
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
                full_deadline = full.then(|| Instant::now() + debounce);
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
#[path = "../../../tests/unit/cli/watch.rs"]
mod tests;
