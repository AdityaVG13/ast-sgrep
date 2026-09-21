//! `asgrep watch` helper split: the main binary carries no file-watcher
//! link (no notify/CoreFoundation/CoreServices dyld tax); `watch` re-execs
//! the `asgrep-watch` helper sitting next to it (or on PATH, or via
//! `ASGREP_WATCH_BIN`). When no helper is reachable the failure must name
//! the helper and how to fix it — never a hang or a bare spawn error.
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

/// `ASGREP_WATCH_BIN` wins over sibling/PATH resolution and the shim execs
/// it with the verbatim watch argv (echo proves forwarding + stdio/exit
/// inheritance without needing the real helper).
#[cfg(unix)]
#[test]
fn watch_helper_override_execs_with_forwarded_argv() {
    let output = Command::new(asgrep_bin())
        .args(["watch", "--debounce-ms", "321", "some-root"])
        .env("ASGREP_WATCH_BIN", "/bin/echo")
        .env("NO_COLOR", "1")
        .output()
        .expect("spawn asgrep watch via echo");
    assert!(
        output.status.success(),
        "echo-backed watch must exit 0; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for needle in ["watch", "--debounce-ms", "321", "some-root"] {
        assert!(
            stdout.contains(needle),
            "forwarded argv must contain {needle}; stdout:\n{stdout}"
        );
    }
}

#[test]
fn watch_without_reachable_helper_fails_with_actionable_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Lone copy: no asgrep-watch sibling. PATH is scrubbed to the empty dir
    // so no ambient helper (or ambient asgrep) can satisfy the lookup, and
    // the override env is removed for the same reason.
    let lone = dir.path().join(exe_name("asgrep"));
    fs::copy(asgrep_bin(), &lone).expect("copy asgrep");
    let root = dir.path().join("proj");
    fs::create_dir_all(&root).expect("proj");

    let mut child = Command::new(&lone)
        .args(["watch", root.to_str().expect("root utf8")])
        .env("PATH", dir.path())
        .env_remove("ASGREP_WATCH_BIN")
        .env("NO_COLOR", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn lone asgrep watch");

    // The shim must fail fast. If watch is still alive after a grace
    // period it ran in-process (pre-split behavior) — kill and fail.
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break Some(status);
        }
        if Instant::now() > deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let Some(status) = status else {
        let _ = child.kill();
        let _ = child.wait();
        panic!("lone `asgrep watch` stayed alive: expected a fast missing-helper error");
    };
    assert!(
        !status.success(),
        "lone `asgrep watch` must exit non-zero without a helper"
    );
    let stderr = child
        .stderr
        .take()
        .map(|mut pipe| {
            use std::io::Read;
            let mut text = String::new();
            pipe.read_to_string(&mut text).expect("read stderr");
            text
        })
        .unwrap_or_default();
    for needle in ["asgrep-watch", "ASGREP_WATCH_BIN"] {
        assert!(
            stderr.contains(needle),
            "missing-helper error must name {needle}; stderr:\n{stderr}"
        );
    }
}
