//! Keep-gate history integrity (PASS 56, H-AUDIT-55 keep-gate row).
//!
//! Contract: a CORRUPT `.bench-history.json` must fail the bench command
//! loudly (non-zero exit, message naming the file) — never be silently reset
//! to an empty baseline. The keep-gate ratchet compares against committed
//! history; silently re-baselining on a parse error would launder a real
//! regression (or a real corruption) into a fresh baseline.
//!
//! Failure-first: pre-fix the run exited 0 and rewrote the corrupt file with
//! a single fresh entry (silent reset).

use std::fs;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn bench_fails_loudly_on_corrupt_history_file() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("lib.rs"), "fn bench_target() { run(1); }\n").unwrap();

    let history = temp.path().join("bench-history.json");
    fs::write(&history, "{ this is not json ]").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_asgrep"))
        .env("ASGREP_BENCH_HISTORY_PATH", &history)
        .args([
            "--no-embed",
            "bench",
            "--query",
            "bench_target",
            "--iterations",
            "1",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("run asgrep bench");

    assert!(
        !output.status.success(),
        "bench must exit non-zero when the keep-gate history file is corrupt; \
         stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        combined.contains("corrupt"),
        "the failure must tell the human the history file is corrupt: {combined}"
    );
    assert!(
        combined.contains(history.to_str().unwrap()),
        "the failure must name the corrupt history file: {combined}"
    );

    // The corrupt file is the human's evidence: bench must NOT auto-delete or
    // auto-reset it. Only a successful rewrite with a VALID prior would be
    // acceptable, and that path is unreachable from a corrupt parse.
    let after = fs::read_to_string(&history).unwrap();
    assert_eq!(
        after, "{ this is not json ]",
        "bench must never silently overwrite a corrupt history file"
    );
}
