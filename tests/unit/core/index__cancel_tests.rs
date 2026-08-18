use super::{IndexOptions, Indexer, INDEX_CANCELLED};
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn index_all_returns_cancelled_before_commit_when_flag_is_set() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    fs::write(corpus.path().join("main.ts"), "export const value = 1;\n").unwrap();
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_dir.path().join("index.db")),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    let cancel = Arc::new(AtomicBool::new(true));
    indexer.set_cancel(Arc::clone(&cancel));
    let error = indexer
        .index_all()
        .expect_err("pre-set cancel must fail closed");
    assert!(
        error.to_string().contains(INDEX_CANCELLED),
        "unexpected error: {error}"
    );
    assert_eq!(indexer.store().status().unwrap().file_count, 0);
}

#[test]
fn index_all_stops_mid_walk_when_cancel_is_signaled() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    for i in 0..240 {
        fs::write(
            corpus.path().join(format!("file-{i}.ts")),
            format!("export function value{i}() {{ return {i}; }}\n"),
        )
        .unwrap();
    }
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_dir.path().join("index.db")),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.set_thread_limit(1);
    let cancel = Arc::new(AtomicBool::new(false));
    indexer.set_cancel(Arc::clone(&cancel));
    let started = Instant::now();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(15));
        cancel.store(true, Ordering::Release);
    });
    let error = indexer
        .index_all()
        .expect_err("mid-index cancel must not commit");
    worker.join().unwrap();
    assert!(
        error.to_string().contains(INDEX_CANCELLED),
        "unexpected error: {error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "cancelled index kept running: {:?}",
        started.elapsed()
    );
    assert_eq!(indexer.store().status().unwrap().file_count, 0);
}
