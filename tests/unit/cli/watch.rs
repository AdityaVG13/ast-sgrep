use super::{begin_full_scan, next_event_wait, queue_event, take_full_rescan};
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
