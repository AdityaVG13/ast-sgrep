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
        Self::spawn_env(bin, root, index_path, debounce_ms, &[])
    }

    fn spawn_env(
        bin: &Path,
        root: &Path,
        index_path: &Path,
        debounce_ms: u64,
        envs: &[(&str, &str)],
    ) -> Self {
        ast_sgrep_testkit::ensure_watch_helper(bin);
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
            .envs(envs.iter().copied())
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
    body["hits"]
        .as_array()
        .is_some_and(|hits| hits.iter().any(|hit| hit.to_string().contains(token)))
}

#[test]
fn watch_hit_oracle_does_not_accept_the_echoed_query() {
    let token = "planted_lbx18_watch";
    let miss = serde_json::json!({
        "ok": true,
        "command": "keyword",
        "query": token,
        "hits": []
    });
    assert!(
        !hit_mentions(&miss, token),
        "echoed query text is not evidence that watch indexed the token"
    );
    let hit = serde_json::json!({
        "ok": true,
        "command": "keyword",
        "query": token,
        "hits": [{"path": "planted.rs", "snippet": "pub fn planted_lbx18_watch() {}"}]
    });
    assert!(hit_mentions(&hit, token));
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
fn watch_failures_preserve_cli_exit_classes_and_machine_envelope() {
    use ast_sgrep_testkit::run_drain_timeout;

    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("project");
    let other = dir.path().join("other");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&other).unwrap();
    let missing = dir.path().join("missing");
    let blocked = dir.path().join("not-a-directory");
    fs::write(&blocked, "keep this file").unwrap();
    let index = blocked.join("index.db");
    let bin = asgrep_bin();
    ast_sgrep_testkit::ensure_watch_helper(&bin);
    let helper = ast_sgrep_testkit::watch_helper_path(&bin);
    assert!(helper.is_file(), "ensure_watch_helper must install the helper");
    let helper_env = [("ASGREP_WATCH_BIN", helper.to_str().unwrap())];
    let mut failures = Vec::new();
    // The invalid index also bounds the pre-fix ambiguous-root path: it must
    // return rather than starting an indefinitely running daemon.
    let cases = [
        (
            vec!["--json", "watch", missing.to_str().unwrap()],
            2,
            "operational",
        ),
        (
            vec![
                "--json",
                "--no-embed",
                "--index-path",
                index.to_str().unwrap(),
                "watch",
                root.to_str().unwrap(),
            ],
            2,
            "operational",
        ),
        (
            vec![
                "--json",
                "--root",
                other.to_str().unwrap(),
                "--index-path",
                index.to_str().unwrap(),
                "watch",
                root.to_str().unwrap(),
            ],
            1,
            "usage",
        ),
        (
            vec!["--json", "watch", "--debounce-ms", "invalid"],
            1,
            "usage",
        ),
    ];
    for executable in [&bin, &helper] {
        for (args, code, kind) in &cases {
            let output = run_drain_timeout(executable, args, &helper_env, Duration::from_secs(10));
            let body = serde_json::from_slice::<Value>(&output.stdout).unwrap_or(Value::Null);
            if output.status.code() != Some(*code)
                || body["ok"] != false
                || body["command"] != "watch"
                || body["error"]["kind"] != *kind
                || body["exit_code"] != *code
            {
                failures.push(format!(
                    "{} {args:?}: expected {kind}/{code}, got status {} stdout {:?} stderr {:?}",
                    executable.display(),
                    output.status,
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                ));
            }
        }
    }
    assert_eq!(fs::read_to_string(&blocked).unwrap(), "keep this file");
    assert!(!root.join(".asgrep").exists());
    assert!(!other.join(".asgrep").exists());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[cfg(target_os = "macos")]
fn watch_ps(args: &[&str]) -> String {
    let output = ast_sgrep_testkit::run_drain_timeout(
        Path::new("/bin/ps"),
        args,
        &[],
        Duration::from_secs(2),
    );
    // Darwin ps returns 1 with no rows when the selected PID is gone.
    assert!(
        output.status.success()
            || (output.status.code() == Some(1)
                && output.stdout.is_empty()
                && output.stderr.is_empty()),
        "cannot inspect owned watch processes: {:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("ps output utf8")
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn supervised_watch_exits_after_parent_death() {
    assert_supervised_watch_exits_after_parent_death(false);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn supervised_watch_exits_after_parent_death_even_while_stopped() {
    assert_supervised_watch_exits_after_parent_death(true);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn assert_supervised_watch_exits_after_parent_death(stop_before_kill: bool) {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("seed.rs"), "pub fn watch_lifecycle_seed() {}\n").unwrap();
    let bin = asgrep_bin();
    ast_sgrep_testkit::ensure_watch_helper(&bin);
    let helper = ast_sgrep_testkit::watch_helper_path(&bin)
        .canonicalize()
        .unwrap();
    let mut watch = WatchProcess::spawn_env(
        &bin,
        &root,
        &dir.path().join("index.db"),
        50,
        &[("ASGREP_CPU_LIMIT_PERCENT", "80")],
    );
    assert!(
        watch.wait_for("initial index", Duration::from_secs(20)),
        "{}",
        watch.log_text()
    );
    let supervisor = watch.child.id();
    #[cfg(target_os = "linux")]
    let worker = {
        let children =
            fs::read_to_string(format!("/proc/{supervisor}/task/{supervisor}/children")).unwrap();
        let children: Vec<i32> = children
            .split_whitespace()
            .map(|s| s.parse().unwrap())
            .collect();
        assert_eq!(
            children.len(),
            1,
            "supervised watch must own exactly one worker"
        );
        let worker = Pid::from_raw(children[0]);
        assert_eq!(
            fs::read_link(format!("/proc/{worker}/exe")).unwrap(),
            helper
        );
        worker
    };
    #[cfg(target_os = "macos")]
    let worker = {
        // A guardian may interpose between supervisor and helper. Discover
        // descendants, not a fixed child count, then verify the exec identity.
        let table = watch_ps(&["-axo", "pid=,ppid="]);
        let table: Vec<(i32, i32)> = table
            .lines()
            .map(|line| {
                let mut fields = line.split_whitespace();
                (
                    fields.next().unwrap().parse().unwrap(),
                    fields.next().unwrap().parse().unwrap(),
                )
            })
            .collect();
        let mut frontier = vec![supervisor as i32];
        let mut descendants = Vec::new();
        for _ in 0..8 {
            let next: Vec<i32> = table
                .iter()
                .filter(|(pid, parent)| frontier.contains(parent) && !descendants.contains(pid))
                .map(|(pid, _)| *pid)
                .collect();
            descendants.extend(&next);
            assert!(
                descendants.len() <= 32,
                "watch descendant observer limit exceeded"
            );
            frontier = next;
            if frontier.is_empty() {
                break;
            }
        }
        assert!(
            !table.iter().any(|(_, parent)| frontier.contains(parent)),
            "watch descendant depth exceeds eight generations"
        );
        let workers: Vec<i32> = descendants
            .into_iter()
            .filter(|pid| {
                let command = watch_ps(&["-ww", "-p", &pid.to_string(), "-o", "comm="]);
                Path::new(command.trim()).canonicalize().ok().as_ref() == Some(&helper)
                    && watch_ps(&["-ww", "-p", &pid.to_string(), "-o", "args="])
                        .trim_end()
                        .ends_with(root.to_str().unwrap())
            })
            .collect();
        assert_eq!(
            workers.len(),
            1,
            "expected one verified exec'd watch helper"
        );
        Pid::from_raw(workers[0])
    };

    // Kill only this verified helper on any assertion unwind; never signal a
    // process group or an unrelated PID that may have reused its number.
    #[cfg(target_os = "linux")]
    struct WorkerCleanup(Pid, PathBuf);
    #[cfg(target_os = "linux")]
    impl Drop for WorkerCleanup {
        fn drop(&mut self) {
            if fs::read_link(format!("/proc/{}/exe", self.0)).ok().as_ref() == Some(&self.1) {
                let _ = kill(self.0, Signal::SIGKILL);
            }
        }
    }
    #[cfg(target_os = "linux")]
    let _cleanup = WorkerCleanup(worker, helper);
    #[cfg(target_os = "macos")]
    struct WorkerCleanup(Pid, PathBuf, String);
    #[cfg(target_os = "macos")]
    impl Drop for WorkerCleanup {
        fn drop(&mut self) {
            // Include start time and full argv (with the unique fixture root)
            // in the recheck, rather than treating a reused PID as our worker.
            let still_owned = std::panic::catch_unwind(|| {
                let pid = self.0.to_string();
                let command = watch_ps(&["-ww", "-p", &pid, "-o", "comm="]);
                Path::new(command.trim()).canonicalize().ok().as_ref() == Some(&self.1)
                    && watch_ps(&["-ww", "-p", &pid, "-o", "lstart=,args="]).trim() == self.2
            })
            .unwrap_or(false);
            if still_owned {
                let _ = kill(self.0, Signal::SIGKILL);
            }
        }
    }
    #[cfg(target_os = "macos")]
    let _cleanup = WorkerCleanup(
        worker,
        helper,
        watch_ps(&["-ww", "-p", &worker.to_string(), "-o", "lstart=,args="])
            .trim()
            .to_owned(),
    );
    #[cfg(target_os = "linux")]
    let state = |pid: Pid| match fs::read_to_string(format!("/proc/{pid}/status")) {
        Ok(text) => Some(
            text.lines()
                .find(|line| line.starts_with("State:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.chars().next())
                .expect("kernel process state"),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("cannot inspect owned process {pid}: {error}"),
    };
    #[cfg(target_os = "macos")]
    let state = |pid: Pid| {
        watch_ps(&["-ww", "-p", &pid.to_string(), "-o", "stat="])
            .trim()
            .chars()
            .next()
    };
    let wait_stopped = |pid| {
        let deadline = Instant::now() + Duration::from_secs(2);
        while state(pid) != Some('T') {
            assert!(Instant::now() < deadline, "process {pid} never stopped");
            thread::sleep(Duration::from_millis(10));
        }
    };
    if stop_before_kill {
        // Freeze the original supervisor first so its duty cycle cannot
        // resume the worker. Leave any independent guardian running.
        kill(Pid::from_raw(supervisor as i32), Signal::SIGSTOP).unwrap();
        wait_stopped(Pid::from_raw(supervisor as i32));
        kill(worker, Signal::SIGSTOP).unwrap();
        wait_stopped(worker);
    }
    // Child::kill sends SIGKILL only to the original supervisor. No helper
    // signal (including SIGCONT) may make the termination assertion pass.
    watch.child.kill().unwrap();
    watch.child.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let current = state(worker);
        // An orphan zombie has terminated; reaping it belongs to its adopter.
        if matches!(current, None | Some('Z' | 'X')) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "watch helper survived supervisor death in state {current:?}"
        );
        thread::sleep(Duration::from_millis(10));
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
