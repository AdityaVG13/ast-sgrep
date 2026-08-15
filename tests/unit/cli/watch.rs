use super::{
    begin_full_scan, is_watch_self_event, next_event_wait, queue_event, schedule_deadline,
    take_full_rescan,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[test]
fn bounded_queue_overflow_requests_a_full_scan() {
    let (tx, rx) = mpsc::sync_channel(1);
    let full = AtomicBool::new(false);
    queue_event(&tx, &full, 1);
    queue_event(&tx, &full, 2);

    assert_eq!(rx.try_recv().unwrap(), 1);
    assert!(take_full_rescan(&full));
    assert!(!take_full_rescan(&full), "overflow marker must coalesce");
}

#[test]
fn events_dropped_during_a_full_scan_request_a_follow_up() {
    let (tx, rx) = mpsc::sync_channel(1);
    let full = AtomicBool::new(true);
    queue_event(&tx, &full, 1);
    begin_full_scan(&rx, &full);
    assert!(rx.try_recv().is_err(), "covered events must be drained");

    // Deterministically model two callback events while indexing: one is
    // retained and the next overflows the bounded queue.
    queue_event(&tx, &full, 2);
    queue_event(&tx, &full, 3);
    assert!(take_full_rescan(&full));
    assert_eq!(rx.try_recv().unwrap(), 2);
}

#[test]
fn a_busy_queue_cannot_postpone_a_required_full_scan() {
    let now = Instant::now();
    let debounce = Duration::from_millis(300);
    let deadline = now + debounce;

    assert_eq!(
        next_event_wait(debounce, Some(deadline), now),
        Some(debounce)
    );
    assert_eq!(next_event_wait(debounce, Some(deadline), deadline), None);
    assert_eq!(
        next_event_wait(debounce, Some(deadline), deadline + debounce),
        None
    );
}

#[test]
fn sustained_incremental_events_keep_the_first_wall_clock_deadline() {
    let now = Instant::now();
    let debounce = Duration::from_millis(300);
    let first_deadline = now + debounce.saturating_mul(3);
    let mut deadline = None;
    schedule_deadline(&mut deadline, first_deadline);

    // A later event may restart the quiet-period wait, but must not move the
    // first event's max-latency deadline.
    schedule_deadline(&mut deadline, first_deadline + debounce);
    assert_eq!(deadline, Some(first_deadline));
    assert_eq!(next_event_wait(debounce, deadline, first_deadline), None);
}

#[test]
fn index_artifacts_do_not_retrigger_watch() {
    let root = Path::new("/repo");
    let default_db = root.join(".asgrep/index.db");
    assert!(is_watch_self_event(
        &[root.join(".asgrep/index.db-wal")],
        root,
        &default_db
    ));

    let custom_db = root.join("custom/index.db");
    assert!(is_watch_self_event(
        &[
            root.join("custom/index.db-shm"),
            root.join("custom/lexical.db-wal"),
            root.join("custom/semantic.ivf"),
            root.join("custom/writer_generation"),
        ],
        root,
        &custom_db
    ));
    assert!(!is_watch_self_event(
        &[PathBuf::from("/repo/src/lib.rs")],
        root,
        &custom_db
    ));
    assert!(!is_watch_self_event(&[], root, &custom_db));
}
