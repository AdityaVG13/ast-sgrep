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

fn helper_bin_name() -> &'static str {
    if cfg!(windows) {
        "asgrep-watch.exe"
    } else {
        "asgrep-watch"
    }
}

/// Newest mtime under `dir` (helper/cli/core sources feed the helper
/// binary; a sibling older than any of them is stale). Missing dirs and
/// unreadable entries are ignored: the fallback build below is always safe.
fn newest_source_mtime(dir: &Path) -> std::time::SystemTime {
    let mut newest = std::time::SystemTime::UNIX_EPOCH;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(meta) = entry.metadata() {
                if let Ok(mtime) = meta.modified() {
                    newest = newest.max(mtime);
                }
            }
        }
    }
    newest
}

/// Guarantee a FRESH helper next to the under-test binary. Sibling-first
/// when it is newer than every source that feeds it (workspace builds and
/// prebuilt dev flows: zero overhead, production resolution path).
/// Targeted `-p ast-sgrep-cli` runs never build the helper (separate
/// package; stable cargo has no binary artifact deps), so a missing or stale
/// sibling falls back to an isolated-target-dir `cargo build` plus a copy
/// next to `asgrep` — isolated because the enclosing `cargo test` holds the
/// shared target-dir lock, copied (not `ASGREP_WATCH_BIN`-injected) so every
/// run exercises the production sibling-resolution path.
///
/// Serialized within the process: the tests in this binary run on parallel
/// threads, and concurrent build+copy pairs corrupt the sibling (two
/// writers) or exec a truncated copy. The install is atomic (copy to
/// `*.tmp` + rename) so a concurrent cross-process spawn sees old or new,
/// never partial.
static ENSURE_HELPER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn ensure_watch_helper(bin: &Path) {
    let _held = ENSURE_HELPER_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let sibling = bin
        .parent()
        .map(|dir| dir.join(helper_bin_name()))
        .unwrap_or_else(|| PathBuf::from(helper_bin_name()));
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(|dir| dir.parent())
        .expect("workspace root above crates/ast-sgrep-cli");
    let sibling_fresh = sibling.is_file()
        && fs::metadata(&sibling)
            .and_then(|meta| meta.modified())
            .map(|built| {
                [
                    "crates/ast-sgrep-watch",
                    "crates/ast-sgrep-cli",
                    "crates/ast-sgrep-core",
                ]
                .iter()
                .all(|tree| newest_source_mtime(&root.join(tree)) <= built)
            })
            .unwrap_or(false);
    if sibling_fresh {
        return;
    }
    let target = root.join("target").join("watch-helper-test");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    eprintln!(
        "watch e2e: building asgrep-watch into {} …",
        target.display()
    );
    let status = Command::new(cargo)
        .args(["build", "--offline", "-p", "ast-sgrep-watch"])
        .env("CARGO_TARGET_DIR", &target)
        .current_dir(root)
        .status()
        .expect("spawn cargo build for asgrep-watch");
    assert!(
        status.success(),
        "cargo build -p ast-sgrep-watch failed; build it once or run cargo test --workspace"
    );
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let helper = target.join(profile).join(helper_bin_name());
    assert!(
        helper.is_file(),
        "asgrep-watch missing after build: {}",
        helper.display()
    );
    // Atomic install: a concurrent spawn (another process) must see the old
    // file or the new file, never a truncated copy.
    let staging = sibling.with_extension("tmp");
    fs::copy(&helper, &staging).expect("stage asgrep-watch next to asgrep");
    // Windows rename cannot replace an existing file; drop the stale
    // sibling first (the in-process mutex above is the real race guard).
    #[cfg(windows)]
    let _ = fs::remove_file(&sibling);
    fs::rename(&staging, &sibling).expect("install asgrep-watch next to asgrep");
}

struct WatchProcess {
    child: Child,
    log: Arc<Mutex<String>>,
}

impl WatchProcess {
    fn spawn(bin: &Path, root: &Path, index_path: &Path, debounce_ms: u64) -> Self {
        ensure_watch_helper(bin);
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
