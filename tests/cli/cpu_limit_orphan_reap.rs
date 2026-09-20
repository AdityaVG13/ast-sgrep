//! Regression: cpu-limit-exec.py payloads must never be orphaned under PPID 1.
//!
//! Incident 2026-09-05: 60 orphaned `/usr/bin/yes` CPU saturators (PPID 1,
//! cwd ast-sgrep) after a limiter/supervisor parent died without reaping its
//! payload process group. These tests kill the limiter parent (SIGKILL, which
//! no signal handler can catch) and assert the detached orphan reaper kills
//! the whole payload process group.
//!
//! Unix-only: the limiter is POSIX fork/setsid/killpg based.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn limiter_script() -> PathBuf {
    // ASGREP_CPU_LIMIT_EXEC override exists so failure-first runs can point
    // the test at a pre-fix copy of the script without touching the tree.
    if let Ok(override_path) = std::env::var("ASGREP_CPU_LIMIT_EXEC") {
        return PathBuf::from(override_path);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("scripts/cpu-limit-exec.py")
}

/// Run `cpu-limit-exec.py -- sh -c '<N background sleeps> sleep 30; wait'`.
/// The sh payload calls setsid via the limiter child, so payload pgid == the
/// sh pid; every background sleep lands in the same group (sh does not
/// setsid). Returns the wrapper handle plus the payload pgid.
fn spawn_limited_group(extra_saturators: usize) -> (Child, u32) {
    let mut inner = String::new();
    for _ in 0..extra_saturators {
        inner.push_str("sleep 30 & ");
    }
    inner.push_str("sleep 30 & wait");
    let mut child = Command::new("python3")
        .arg(limiter_script())
        .args(["--limit", "80", "--"])
        .args(["sh", "-c", &inner])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn cpu-limit-exec wrapper");
    let wrapper_pid = child.id();
    // Give the forked limiter child time to setsid + exec before lookup.
    let deadline = Instant::now() + Duration::from_secs(5);
    let pgid = loop {
        if let Some(pgid) = child_pgid_of(wrapper_pid) {
            break pgid;
        }
        if let Ok(Some(status)) = child.try_wait() {
            panic!("limiter wrapper exited early: {status}");
        }
        assert!(Instant::now() < deadline, "payload child never appeared");
        thread::sleep(Duration::from_millis(25));
    };
    (child, pgid)
}

fn child_pgid_of(parent: u32) -> Option<u32> {
    let out = Command::new("/bin/ps")
        .args(["-axo", "pgid=", "-o", "ppid="])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let pgid = it.next()?.parse::<u32>().ok()?;
            let ppid = it.next()?.parse::<u32>().ok()?;
            (ppid == parent).then_some(pgid)
        })
        .next()
}

fn pgroup_members(pgid: u32) -> Vec<u32> {
    let out = Command::new("/bin/ps")
        .args(["-axo", "pid=", "-o", "pgid="])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let pid = it.next()?.parse::<u32>().ok()?;
            let g = it.next()?.parse::<u32>().ok()?;
            (g == pgid).then_some(pid)
        })
        .collect()
}

fn signal(sig: &str, pid: u32) {
    let _ = Command::new("/bin/kill")
        .args([sig, &pid.to_string()])
        .status();
}

fn assert_group_dies(pgid: u32, timeout: Duration, ctx: &str) {
    let deadline = Instant::now() + timeout;
    loop {
        let survivors = pgroup_members(pgid);
        if survivors.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{ctx}: payload group {pgid} still alive: {survivors:?} (would orphan under PPID 1)"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn sigkill_of_limiter_parent_leaves_no_orphan_payload_group() {
    let saturators = 3;
    let (mut wrapper, pgid) = spawn_limited_group(saturators);
    let members = pgroup_members(pgid);
    assert!(
        members.len() > saturators,
        "payload group should hold sh + {saturators} background sleeps, got {members:?}"
    );

    signal("-9", wrapper.id());
    let _ = wrapper.wait(); // reap the wrapper zombie

    // Orphan reaper polls at 50 ms, waits up to 5 s for setsid, TERM grace
    // 0.5 s, then SIGKILL. 8 s is generous.
    assert_group_dies(
        pgid,
        Duration::from_secs(8),
        "orphan reaper failed after SIGKILL of limiter parent",
    );
}

#[test]
fn term_of_limiter_parent_reaps_payload_group() {
    let (mut wrapper, pgid) = spawn_limited_group(2);
    signal("-TERM", wrapper.id());
    let status = wrapper.wait().expect("wait on limiter wrapper");
    assert_eq!(status.code(), Some(143), "limiter must exit 128+SIGTERM");
    assert_group_dies(
        pgid,
        Duration::from_secs(10),
        "limiter TERM path left payload group alive",
    );
}
