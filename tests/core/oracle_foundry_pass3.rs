//! Pass 3 (oracle-foundry, Mission 1): L3 snapshot + runner discipline for
//! core search/index goldens and the mmap read-only boundary.
//!
//! Pass 1 pinned hand-computed values and pass 2 pinned mutant-killing
//! discriminants; this pass pins the snapshot machinery itself: the update
//! gate truth table, text canonicalization, compare-vs-update behavior,
//! mismatch sidecars, JSON value (not text) comparison, chain-response
//! canonical sorting, search-dump scrubbing, byte stability, and the
//! repo-level runner pins (nextest profile, compare-only CI, layout).
//!
//! Env discipline: tests that touch `ASGREP_UPDATE_GOLDENS` serialize on
//! `ENV_LOCK` and always restore the previous value, so no test in this
//! binary ever observes update mode by accident. Update-mode tests only write
//! inside fresh tempdirs, never to real goldens.

use ast_sgrep_core::chain::{ChainEdge, ChainNode, ChainResponse, EdgeLabel};
use ast_sgrep_mmap::map_readonly;
use ast_sgrep_testkit::{
    assert_golden_at, assert_golden_json_at, canonicalize_chain_response, canonicalize_text,
    updating_goldens, Scrubber,
};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Run `f` with `ASGREP_UPDATE_GOLDENS` set to `value` (`None` removes it),
/// restoring the previous value afterwards. Serialized across this binary.
fn with_update_env<R>(value: Option<&str>, f: impl FnOnce() -> R) -> R {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let prev = std::env::var("ASGREP_UPDATE_GOLDENS").ok();
    match value {
        Some(v) => std::env::set_var("ASGREP_UPDATE_GOLDENS", v),
        None => std::env::remove_var("ASGREP_UPDATE_GOLDENS"),
    }
    let out = f();
    match prev {
        Some(v) => std::env::set_var("ASGREP_UPDATE_GOLDENS", v),
        None => std::env::remove_var("ASGREP_UPDATE_GOLDENS"),
    }
    out
}

/// Run `f`, returning the panic message when it panics and `None` otherwise.
fn catch_message(f: impl FnOnce() -> ()) -> Option<String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .err()
        .map(|payload| {
            if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else {
                String::from("<non-string panic payload>")
            }
        })
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn node(file: &str, symbol: Option<&str>, line: u32, depth: u32) -> ChainNode {
    ChainNode {
        file: file.to_string(),
        line_start: line,
        line_end: line,
        symbol: symbol.map(str::to_string),
        language: None,
        score: 1.0,
        depth,
    }
}

fn edge(from: &str, to: &str, label: EdgeLabel, depth: u32) -> ChainEdge {
    ChainEdge {
        from_file: from.to_string(),
        from_symbol: None,
        to_file: to.to_string(),
        to_symbol: None,
        label,
        depth,
    }
}

#[test]
fn update_gate_truth_table_matches_sop() {
    // Golden-files SOP: unset/0/false/off compare; 1/true/yes/on update.
    for truthy in [
        "1", "true", "TRUE", "True", "yes", "YES", "on", "ON", " 1 ", " on\n",
    ] {
        with_update_env(Some(truthy), || {
            assert!(updating_goldens(), "truthy {truthy:?} must update");
        });
    }
    with_update_env(None, || assert!(!updating_goldens(), "unset must compare"));
    for falsy in [
        "0",
        "false",
        "FALSE",
        "off",
        "OFF",
        "",
        "2",
        "yes please",
        "update",
    ] {
        with_update_env(Some(falsy), || {
            assert!(!updating_goldens(), "falsy {falsy:?} must compare");
        });
    }
}

#[test]
fn canonicalize_text_matches_hand_table() {
    let cases: &[(&str, &str)] = &[
        ("a\r\nb\r\n", "a\nb\n"),
        ("a  \n b\t\n", "a\n b\n"),
        ("a\n\n\n", "a\n"),
        ("", ""),
        ("\n\n", ""),
        ("\r\n", ""),
        ("x", "x\n"),
        ("a\nb", "a\nb\n"),
        ("a \r\n \n", "a\n"),
        // Leading whitespace is content; trailing whitespace is not.
        ("  indented\n\ttabbed  \n", "  indented\n\ttabbed\n"),
    ];
    for (input, expected) in cases {
        assert_eq!(canonicalize_text(input), *expected, "input={input:?}");
    }
}

#[test]
fn golden_compare_ignores_canonical_differences() {
    with_update_env(Some("0"), || {
        let dir = tempfile::tempdir().expect("tempdir");
        let golden = dir.path().join("text.txt");
        std::fs::write(&golden, "a  \r\nb\n").expect("write golden");
        // Trailing whitespace + CRLF canonicalize away: must NOT panic.
        assert_golden_at(&golden, "a\nb  \r\n");
        // No sidecar on match.
        assert!(!dir.path().join("text.txt.actual").exists());
    });
}

#[test]
fn golden_mismatch_writes_actual_and_panics() {
    with_update_env(Some("0"), || {
        let dir = tempfile::tempdir().expect("tempdir");
        let golden = dir.path().join("want.txt");
        std::fs::write(&golden, "want\n").expect("write");
        let message = catch_message(|| assert_golden_at(&golden, "got  \r\n"))
            .expect("mismatch must panic in compare mode");
        assert!(message.contains("golden mismatch"), "message: {message}");
        assert!(message.contains("want.txt.actual"), "message: {message}");
        assert!(
            message.contains("ASGREP_UPDATE_GOLDENS=1"),
            "message: {message}"
        );
        assert!(message.contains("--- golden"), "diff header: {message}");
        let actual = std::fs::read_to_string(dir.path().join("want.txt.actual")).expect("sidecar");
        assert_eq!(actual, "got\n", "sidecar holds canonicalized actual");
        // The golden itself is untouched in compare mode.
        assert_eq!(std::fs::read_to_string(&golden).expect("golden"), "want\n");
    });
}

#[test]
fn missing_golden_fails_loudly_without_creating_files() {
    with_update_env(Some("0"), || {
        let dir = tempfile::tempdir().expect("tempdir");
        let golden = dir.path().join("absent.txt");
        let message = catch_message(|| assert_golden_at(&golden, "anything"))
            .expect("missing golden must panic in compare mode");
        assert!(message.contains("missing golden"), "message: {message}");
        assert!(
            message.contains("ASGREP_UPDATE_GOLDENS=1"),
            "message: {message}"
        );
        assert!(!golden.exists(), "compare mode must not create goldens");
        assert!(!dir.path().join("absent.txt.actual").exists());
    });
}

#[test]
fn update_mode_writes_nested_goldens_but_compare_never_does() {
    let dir = tempfile::tempdir().expect("tempdir");
    let golden = dir.path().join("sub").join("dir").join("new.txt");
    with_update_env(Some("1"), || {
        assert_golden_at(&golden, "fresh  \r\n");
    });
    assert_eq!(
        std::fs::read_to_string(&golden).expect("written"),
        "fresh\n"
    );
    // Back in compare mode the same content matches and new content panics.
    with_update_env(Some("0"), || {
        assert_golden_at(&golden, "fresh\n");
        let message = catch_message(|| assert_golden_at(&golden, "changed\n"))
            .expect("changed content must panic");
        assert!(message.contains("golden mismatch"), "message: {message}");
    });
    assert_eq!(std::fs::read_to_string(&golden).expect("golden"), "fresh\n");
}

#[test]
fn golden_json_compares_values_not_text() {
    with_update_env(Some("0"), || {
        let dir = tempfile::tempdir().expect("tempdir");
        let golden = dir.path().join("v.json");
        // Compact, reversed keys, no trailing newline: equal by Value.
        std::fs::write(&golden, r#"{"b":2,"a":[1,2]}"#).expect("write");
        assert_golden_json_at(&golden, &serde_json::json!({"a": [1, 2], "b": 2}));
        assert!(!dir.path().join("v.json.actual").exists());
        // A real value change fails and dumps pretty actual + newline.
        let message = catch_message(|| {
            assert_golden_json_at(&golden, &serde_json::json!({"a": [1, 2], "b": 3}))
        })
        .expect("value change must panic");
        assert!(message.contains("golden mismatch"), "message: {message}");
        let actual = std::fs::read_to_string(dir.path().join("v.json.actual")).expect("sidecar");
        assert_eq!(
            actual,
            "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"b\": 3\n}\n"
        );
    });
}

#[test]
fn chain_canonicalization_sorts_seeds_nodes_edges() {
    let response = ChainResponse {
        query: "q".to_string(),
        seeds: vec![node("b.rs", Some("b"), 1, 0), node("a.rs", Some("a"), 1, 0)],
        nodes: vec![
            node("a.rs", Some("b"), 9, 0),
            node("a.rs", Some("a"), 9, 0),
            node("a.rs", Some("a"), 3, 0),
        ],
        edges: vec![
            edge("a.rs", "b.rs", EdgeLabel::Imports, 1),
            edge("a.rs", "b.rs", EdgeLabel::Calls, 1),
        ],
        max_depth: 2,
        decay_factor: 0.5,
        node_count: 3,
        edge_count: 2,
    };
    let sorted = canonicalize_chain_response(response);
    let seed_files: Vec<&str> = sorted.seeds.iter().map(|n| n.file.as_str()).collect();
    assert_eq!(seed_files, vec!["a.rs", "b.rs"]);
    let node_keys: Vec<(&str, &str, u32)> = sorted
        .nodes
        .iter()
        .map(|n| {
            (
                n.file.as_str(),
                n.symbol.as_deref().unwrap_or(""),
                n.line_start,
            )
        })
        .collect();
    assert_eq!(
        node_keys,
        vec![("a.rs", "a", 3), ("a.rs", "a", 9), ("a.rs", "b", 9)]
    );
    let labels: Vec<EdgeLabel> = sorted.edges.iter().map(|e| e.label).collect();
    // Edge key stringifies the label: "Calls" < "Imports".
    assert_eq!(labels, vec![EdgeLabel::Calls, EdgeLabel::Imports]);
    // Canonicalization only reorders; counts and query survive.
    assert_eq!(sorted.query, "q");
    assert_eq!((sorted.node_count, sorted.edge_count), (3, 2));
    // Fixpoint: already-sorted input serializes identically.
    let again = canonicalize_chain_response(sorted.clone());
    assert_eq!(
        serde_json::to_value(&again).expect("json"),
        serde_json::to_value(&sorted).expect("json")
    );
}

#[test]
fn search_dump_scrub_removes_paths_but_keeps_scores() {
    let root = Path::new("/tmp/asgrep-work/tree");
    let scrubbed = Scrubber::search_dump(root).apply(
        r#"{"file": "/tmp/asgrep-work/tree/src/main.rs", "home": "/Users/adana", "score": 9.5, "rank": 3, "id": "01234567-89ab-cdef-0123-456789abcdef"}"#,
    );
    assert!(
        scrubbed.contains("<ROOT>/src/main.rs"),
        "scrubbed: {scrubbed}"
    );
    assert!(
        !scrubbed.contains("/tmp/asgrep-work"),
        "scrubbed: {scrubbed}"
    );
    assert!(scrubbed.contains("<HOME>"), "scrubbed: {scrubbed}");
    assert!(!scrubbed.contains("/Users/adana"), "scrubbed: {scrubbed}");
    assert!(scrubbed.contains("<UUID>"), "scrubbed: {scrubbed}");
    // Scores and ranks are product signal: never scrubbed.
    assert!(scrubbed.contains("9.5"), "scrubbed: {scrubbed}");
    assert!(scrubbed.contains("\"rank\": 3"), "scrubbed: {scrubbed}");
}

#[test]
fn mmap_and_serde_views_are_byte_stable() {
    // Mmap: two mappings of the same file agree byte-exact (pass 1 pinned
    // the content; this pins the stability a snapshot needs).
    let bytes: &[u8] = b"snapshot-stable \x00 bytes \xc3\xa9\n";
    let mut file = tempfile::NamedTempFile::new().expect("temp file");
    file.write_all(bytes).expect("write");
    file.flush().expect("flush");
    let first = map_readonly(file.as_file()).expect("map");
    let second = map_readonly(file.as_file()).expect("remap");
    assert_eq!(&first[..], &second[..]);
    assert_eq!(&first[..], bytes);
    // Serde: the same chain value serializes identically on repeat.
    let response = ChainResponse {
        query: "stable".to_string(),
        seeds: vec![node("a.rs", None, 1, 0)],
        nodes: vec![],
        edges: vec![],
        max_depth: 2,
        decay_factor: 0.5,
        node_count: 1,
        edge_count: 0,
    };
    let a = serde_json::to_string(&response).expect("json");
    let b = serde_json::to_string(&response).expect("json");
    assert_eq!(a, b);
    assert!(a.contains("\"query\":\"stable\""), "json: {a}");
}

#[test]
fn nextest_profile_forbids_retries_and_forced_green() {
    let path = workspace().join(".config/nextest.toml");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("missing {}: {err}", path.display()));
    assert!(raw.contains("[profile.ci]"), "needs a ci profile:\n{raw}");
    assert!(raw.contains("fail-fast = false"), "no fail-fast:\n{raw}");
    assert!(raw.contains("retries = 0"), "no retries:\n{raw}");
    assert!(raw.contains("failure-output"), "raw exits:\n{raw}");
    assert!(raw.contains("success-output"), "quiet success:\n{raw}");
    for line in raw.lines() {
        let code = line.split('#').next().unwrap_or("").trim();
        if let Some(value) = code.strip_prefix("retries") {
            assert_eq!(value.trim(), "= 0", "retries must stay 0: {line}");
        }
        assert!(
            !code.contains("force-pass") && !code.contains("force_pass"),
            "no forced green: {line}"
        );
    }
}

#[test]
fn ci_is_compare_only_without_snapshot_accept() {
    let path = workspace().join(".github/workflows/ci.yml");
    let raw = std::fs::read_to_string(&path).expect("ci.yml");
    assert!(
        raw.contains("ASGREP_UPDATE_GOLDENS"),
        "ci must pin the golden gate"
    );
    let mut pinned = 0;
    for line in raw.lines() {
        if line.trim().starts_with('#') {
            continue;
        }
        if line.contains("ASGREP_UPDATE_GOLDENS") {
            assert!(line.contains("\"0\""), "compare-only pin required: {line}");
            pinned += 1;
        }
        let lower = line.to_ascii_lowercase();
        assert!(
            !line.contains("INSTA_UPDATE"),
            "forbidden insta gate: {line}"
        );
        // The custom gate is ASGREP_-prefixed; a bare UPDATE_GOLDENS is rejected.
        let stripped = line.replace("ASGREP_UPDATE_GOLDENS", "");
        assert!(
            !stripped.contains("UPDATE_GOLDENS"),
            "unprefixed golden gate: {line}"
        );
        assert!(
            !lower.contains("insta accept") && !lower.contains("--accept"),
            "no snapshot accept: {line}"
        );
        assert!(
            !line.contains("INSTA_FORCE_PASS")
                && !lower.contains("force-pass")
                && !lower.contains("force_pass"),
            "no forced green: {line}"
        );
        assert!(
            !lower.contains("--retries") && !lower.contains("rerun-failed"),
            "no retry-away: {line}"
        );
    }
    assert!(
        pinned >= 2,
        "expected golden pins on test jobs, found {pinned}"
    );
}

#[test]
fn workspace_layout_pins_and_gitignore() {
    let root = workspace();
    assert!(root.join("Cargo.toml").is_file());
    // Crate-local test dirs and tests/unit are forbidden (mirror of the
    // layout half of the CI gate; the inline-#[test] half is CI-owned).
    let mut members = 0;
    for entry in std::fs::read_dir(root.join("crates")).expect("crates dir") {
        let entry = entry.expect("entry");
        if !entry.file_type().expect("type").is_dir() {
            continue;
        }
        members += 1;
        assert!(
            !entry.path().join("tests").exists(),
            "crate-local tests dir forbidden: {}",
            entry.path().display()
        );
    }
    assert!(
        members >= 11,
        "expected 11 workspace crates, found {members}"
    );
    assert!(!root.join("tests/unit").exists(), "tests/unit is forbidden");
    let gitignore = std::fs::read_to_string(root.join(".gitignore")).expect(".gitignore");
    assert!(
        gitignore.lines().any(|line| line.trim() == "*.actual"),
        "gitignore must cover *.actual"
    );
}

#[test]
fn pi_launcher_and_validation_ledgers_present() {
    let root = workspace();
    let launcher = root.join("tests/pi/launcher");
    for name in [
        "asgrep-search-mode-matrix.test.mjs",
        "binary-env-alias.test.mjs",
        "extension-package.test.mjs",
        "npm-native-packages.test.mjs",
        "package-security.test.mjs",
        "skill-security.test.mjs",
    ] {
        assert!(launcher.join(name).is_file(), "missing pi launcher {name}");
    }
    for ledger in [
        "golden-files.md",
        "machine-json-schema.md",
        "negative-ledgers.md",
        "neural-trust.md",
        "semantic-ivf-mmap.md",
        "compact-output.md",
    ] {
        assert!(
            root.join("docs/validation").join(ledger).is_file(),
            "missing ledger {ledger}"
        );
    }
    assert!(
        root.join("docs/validation/audits").is_dir(),
        "missing audits dir"
    );
}
