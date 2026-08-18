//! Real CLI `watch` process + filesystem edit (lbx1.8).
//! Does not replace `watch_incremental` (library `update_paths` only).
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

struct WatchProcess {
    child: Child,
    log: Arc<Mutex<String>>,
}

impl WatchProcess {
    fn spawn(bin: &Path, root: &Path, index_path: &Path, debounce_ms: u64) -> Self {
        let mut child = Command::new(bin)
            .args([
                "--no-embed",
                "--index-path",
                index_path.to_str().expect("index path utf8"),
                "watch",
                "--debounce-ms",
                &debounce_ms.to_string(),
                root.to_str().expect("root utf8"),
            ])
            .env("NO_COLOR", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn asgrep watch");
        let stderr = child.stderr.take().expect("piped stderr");
        let log = Arc::new(Mutex::new(String::new()));
        let log_writer = Arc::clone(&log);
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if let Ok(mut held) = log_writer.lock() {
                    held.push_str(&line);
                    held.push('\n');
                }
            }
        });
        Self { child, log }
    }

    fn log_text(&self) -> String {
        self.log.lock().map(|held| held.clone()).unwrap_or_default()
    }

    fn wait_for(&mut self, needle: &str, timeout: Duration) -> bool {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if self.log_text().contains(needle) {
                return true;
            }
            if let Ok(Some(_)) = self.child.try_wait() {
                return self.log_text().contains(needle);
            }
            thread::sleep(Duration::from_millis(50));
        }
        self.log_text().contains(needle)
    }
}

impl Drop for WatchProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn search_keyword(bin: &Path, root: &Path, index_path: &Path, query: &str) -> Value {
    let output = Command::new(bin)
        .args([
            "--json",
            "--no-embed",
            "--index-path",
            index_path.to_str().expect("index path utf8"),
            "keyword",
            query,
            root.to_str().expect("root utf8"),
        ])
        .env("NO_COLOR", "1")
        .output()
        .expect("keyword search");
    assert_eq!(
        output.status.code(),
        Some(0),
        "keyword failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "keyword stdout is not JSON: {error}\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn hit_mentions(body: &Value, token: &str) -> bool {
    let rendered = body.to_string();
    rendered.contains(token)
}

#[test]
fn cli_watch_reindexes_after_real_fs_create() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("proj");
    fs::create_dir_all(&root).expect("proj");
    fs::write(root.join("hello.rs"), "pub fn hello_lbx18() {}\n").expect("seed");
    let index_path = dir.path().join("idx").join("index.db");
    fs::create_dir_all(index_path.parent().expect("idx parent")).expect("idx");

    let bin = asgrep_bin();
    let mut watch = WatchProcess::spawn(&bin, &root, &index_path, 50);
    assert!(
        watch.wait_for("initial index", Duration::from_secs(20)),
        "watch never finished initial index.\nstderr:\n{}",
        watch.log_text()
    );

    let before = search_keyword(&bin, &root, &index_path, "hello_lbx18");
    assert!(
        hit_mentions(&before, "hello_lbx18"),
        "seed symbol missing after initial watch index: {before}"
    );
    assert!(
        !hit_mentions(&before, "planted_lbx18_watch"),
        "planted token must not exist before the fs edit: {before}"
    );

    fs::write(
        root.join("planted.rs"),
        "pub fn planted_lbx18_watch() -> u32 { 18 }\n",
    )
    .expect("create planted.rs");

    let started_watch = Instant::now();
    let watch_timeout = Duration::from_secs(15);
    loop {
        let log = watch.log_text();
        if log.contains("updated") || log.contains("full rescan") {
            break;
        }
        if started_watch.elapsed() > watch_timeout {
            panic!(
                "watch never logged an incremental update or full rescan after creating planted.rs.\nstderr:\n{log}"
            );
        }
        thread::sleep(Duration::from_millis(50));
    }

    let started = Instant::now();
    let timeout = Duration::from_secs(10);
    loop {
        let after = search_keyword(&bin, &root, &index_path, "planted_lbx18_watch");
        if hit_mentions(&after, "planted_lbx18_watch") {
            return;
        }
        if started.elapsed() > timeout {
            panic!(
                "watch logged a reindex but keyword search never saw planted_lbx18_watch within {:?}.\nstderr:\n{}\nlast search: {after}",
                timeout,
                watch.log_text()
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn cli_watch_reindexes_during_sustained_same_file_writes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("proj");
    fs::create_dir_all(&root).expect("proj");
    fs::write(root.join("busy.rs"), "pub fn seed_watch_file() {}\n").expect("seed");
    let index_path = dir.path().join("idx").join("index.db");
    fs::create_dir_all(index_path.parent().expect("idx parent")).expect("idx");

    let bin = asgrep_bin();
    let mut watch = WatchProcess::spawn(&bin, &root, &index_path, 100);
    assert!(
        watch.wait_for("initial index", Duration::from_secs(20)),
        "watch never finished initial index.\nstderr:\n{}",
        watch.log_text()
    );

    let writer_active = Arc::new(AtomicBool::new(true));
    let writer_state = Arc::clone(&writer_active);
    let busy_file = root.join("busy.rs");
    let writer = thread::spawn(move || {
        for revision in 0..240 {
            fs::write(
                &busy_file,
                format!("pub fn sustained_watch_token() -> usize {{ {revision} }}\n"),
            )
            .expect("rewrite busy.rs");
            thread::sleep(Duration::from_millis(25));
        }
        writer_state.store(false, Ordering::SeqCst);
    });

    let started = Instant::now();
    let timeout = Duration::from_secs(5);
    let observed_while_writing = loop {
        let result = search_keyword(&bin, &root, &index_path, "sustained_watch_token");
        if hit_mentions(&result, "sustained_watch_token") {
            break writer_active.load(Ordering::SeqCst);
        }
        if started.elapsed() > timeout {
            break false;
        }
        thread::sleep(Duration::from_millis(50));
    };

    writer.join().expect("sustained writer");
    assert!(
        observed_while_writing,
        "keyword search did not observe sustained_watch_token while writes were still arriving.\nstderr:\n{}",
        watch.log_text()
    );
}
