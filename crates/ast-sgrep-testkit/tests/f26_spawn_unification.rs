//! F26 regression: the three hermetic spawn paths share one preamble, one
//! bounds table, one wait core (`spawn_hermetic` + `SpawnMode`).
//!
//! Fails before the fix (missing `spawn_hermetic` / `SpawnMode` / `SPAWN_*`
//! bounds); passes after, with the parent-env scrub, extras-win, NO_COLOR
//! pin, Collect/Drain equivalence, and timeout kill pinned behaviorally.

use ast_sgrep_testkit::{
    RUN_TIMEOUT, SPAWN_POLL_INTERVAL, SPAWN_RECV_TIMEOUT, SPAWN_RUN_TIMEOUT, SPAWN_WAIT_TIMEOUT,
    SpawnMode, spawn_hermetic,
};
use std::time::Duration;

#[test]
fn bounds_table_is_single_sourced() {
    assert_eq!(SPAWN_RUN_TIMEOUT, Duration::from_secs(60));
    assert_eq!(RUN_TIMEOUT, SPAWN_RUN_TIMEOUT);
    assert_eq!(SPAWN_RECV_TIMEOUT, Duration::from_secs(15));
    assert_eq!(SPAWN_WAIT_TIMEOUT, Duration::from_secs(15));
    assert_eq!(SPAWN_POLL_INTERVAL, Duration::from_millis(25));
}

#[cfg(unix)]
#[test]
fn hermetic_spawn_scrubs_parent_asgrep_env() {
    std::env::set_var("ASGREP_TESTKIT_PROBE_F26", "leaked");
    let output = spawn_hermetic(
        std::path::Path::new("/bin/sh"),
        &["-c", "echo ${ASGREP_TESTKIT_PROBE_F26:-scrubbed}"],
        &[],
        None,
        SpawnMode::Collect,
        None,
    );
    std::env::remove_var("ASGREP_TESTKIT_PROBE_F26");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "scrubbed",
        "parent ASGREP_* must not reach the child"
    );
}

#[cfg(unix)]
#[test]
fn hermetic_spawn_extras_win_and_pins_no_color() {
    let output = spawn_hermetic(
        std::path::Path::new("/bin/sh"),
        &["-c", "echo $ASGREP_TESTKIT_PROBE_F26:$NO_COLOR:$HF_HUB_OFFLINE"],
        &[("ASGREP_TESTKIT_PROBE_F26", "planted")],
        None,
        SpawnMode::Collect,
        None,
    );
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "planted:1:1",
        "explicit envs win over the scrub; NO_COLOR/HF_HUB_OFFLINE pinned"
    );
}

#[cfg(unix)]
#[test]
fn collect_and_drain_agree_on_small_output() {
    let run = |mode| {
        spawn_hermetic(
            std::path::Path::new("/bin/sh"),
            &["-c", "echo hello-f26"],
            &[],
            None,
            mode,
            Some(Duration::from_secs(60)),
        )
    };
    let collect = run(SpawnMode::Collect);
    let drain = run(SpawnMode::Drain);
    assert!(collect.status.success());
    assert_eq!(collect.stdout, drain.stdout);
    assert_eq!(
        String::from_utf8_lossy(&collect.stdout).trim(),
        "hello-f26"
    );
}

#[cfg(unix)]
#[test]
fn drain_mode_survives_output_past_the_pipe_buffer() {
    // The >64KB pipe-deadlock facet: try_wait-without-drain hangs here.
    let output = spawn_hermetic(
        std::path::Path::new("/bin/sh"),
        &["-c", "yes | head -c 300000"],
        &[],
        None,
        SpawnMode::Drain,
        Some(Duration::from_secs(60)),
    );
    assert!(output.status.success());
    assert_eq!(output.stdout.len(), 300_000);
}

#[cfg(unix)]
#[test]
#[should_panic(expected = "exceeded")]
fn collect_mode_kills_past_the_timeout() {
    let _ = spawn_hermetic(
        std::path::Path::new("/bin/sh"),
        &["-c", "sleep 30"],
        &[],
        None,
        SpawnMode::Collect,
        Some(Duration::from_millis(100)),
    );
}
