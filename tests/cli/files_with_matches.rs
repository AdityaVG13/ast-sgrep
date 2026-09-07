//! F-SG-RUN-FILES-WITH-MATCHES (pass 31, matrix rev 7): boolean-listing output
//! mode — `--files-with-matches` on the search family prints the matching file
//! paths (sorted, deduped, one per line) instead of hit rows; machine (--json)
//! envelopes carry the same set as a top-level `files` array (P3 decision: a
//! paths ARRAY, not a boolean — the listing IS the result set; hits stay in the
//! envelope, the field is additive so the per-format schema snapshots are
//! untouched).
//!
//! Exit semantics: zero matches prints nothing and exits 0 per the subject's
//! own exit contract (0=ok 1=usage 2=fail, F-AS-EXIT-CONTRACT). sg exits 1 on
//! no matches (grep convention); the gauntlet oracle comparator already
//! normalizes oracle exit-1 + empty stdout + empty stderr to success_empty
//! (run_oracle.py oracle_search_canonical), so the zero-match face is
//! comparator-agreed, not a hidden divergence.
//!
//! Failure-first: these tests were run against the pre-flag tree and failed
//! (clap rejected `--files-with-matches` with an unknown-flag error).

use ast_sgrep_testkit::CliSession;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

/// Corpus with a multi-match file (dedup face) and a non-matching file:
/// src/a.rs matches the rust fn template TWICE, src/b.rs once, c.txt never.
fn corpus_session() -> CliSession {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("corpus");
    fs::create_dir_all(root.join("src")).expect("mkdir corpus");
    fs::write(
        root.join("src/a.rs"),
        // No return types: the native fn template is return-type-strict
        // (D1 closed pass 14, H-CONF-005), so `fn $A($$$B) { $$$C }` matches
        // return-type-less fns only — a.rs matches TWICE, b.rs once.
        "fn one(x: i32) {\n    println!(\"{} \", x);\n}\n\nfn two(y: i32) {\n    println!(\"{} \", y);\n}\n",
    )
    .expect("write a.rs");
    fs::write(
        root.join("src/b.rs"),
        "fn three(z: i32) {\n    println!(\"{} \", z);\n}\n",
    )
    .expect("write b.rs");
    fs::write(root.join("c.txt"), "plain text, no rust here\n").expect("write c.txt");
    let index_path = temp.path().join("index.db");
    let index_out = std::process::Command::new(asgrep_bin())
        .args([
            "--index-path",
            index_path.to_str().unwrap(),
            "index",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("index corpus");
    assert!(
        index_out.status.success(),
        "indexing failed: {}",
        String::from_utf8_lossy(&index_out.stderr)
    );
    CliSession {
        _temp: temp,
        root,
        index_path,
        bin: asgrep_bin(),
    }
}

fn run_search(session: &CliSession, patterns: &[&str], extra: &[&str]) -> (i32, String, String) {
    let mut args: Vec<String> = vec![
        "--index-path".into(),
        session.index_path.to_str().unwrap().into(),
        "--no-embed".into(),
        "--no-auto-index".into(),
        "--root".into(),
        session.root.to_str().unwrap().into(),
        "search".into(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    for pattern in patterns {
        args.push("--pattern".into());
        args.push(pattern.to_string());
    }
    args.push("--lang".into());
    args.push("rust".into());
    match session.run(&args.iter().map(String::as_str).collect::<Vec<_>>()) {
        Ok(o) => (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        ),
        Err(e) => panic!("run failed: {e}"),
    }
}

#[test]
fn files_with_matches_lists_sorted_deduped_paths() {
    let session = corpus_session();
    let (code, stdout, stderr) =
        run_search(&session, &["fn $A($$$B) { $$$C }"], &["--files-with-matches"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec!["src/a.rs", "src/b.rs"],
        "one path per matching file, sorted, deduped despite a.rs's two hits; got {stdout:?}"
    );
}

#[test]
fn files_with_matches_zero_matches_is_empty_output_exit_ok() {
    let session = corpus_session();
    let (code, stdout, stderr) = run_search(
        &session,
        &["fn no_such_function_name($$$B) { $$$C }"],
        &["--files-with-matches"],
    );
    assert_eq!(
        code, 0,
        "zero matches is ok:true per the exit contract; stderr={stderr}"
    );
    assert!(
        stdout.trim().is_empty(),
        "zero matches prints no paths; got {stdout:?}"
    );
    assert!(
        stderr.trim().is_empty(),
        "no diagnostics on a clean miss: {stderr}"
    );
}

#[test]
fn files_with_matches_json_envelope_carries_files_array() {
    let session = corpus_session();
    let (code, stdout, stderr) = run_search(
        &session,
        &["fn $A($$$B) { $$$C }"],
        &["--files-with-matches", "--json"],
    );
    assert_eq!(code, 0, "stderr={stderr}");
    let envelope: Value = serde_json::from_str(&stdout).expect("envelope json");
    assert_eq!(envelope["ok"], true);
    assert_eq!(
        envelope["files"],
        Value::Array(vec![Value::from("src/a.rs"), Value::from("src/b.rs")]),
        "files array sorted+deduped; got {}",
        envelope["files"]
    );
    // P3 decision: additive field — hits stay in the envelope untouched.
    let hits = envelope["hits"].as_array().expect("hits array");
    assert!(
        hits.len() >= 3,
        "hits preserved alongside files; got {}",
        hits.len()
    );
}

#[test]
fn files_with_matches_multi_pattern_dedupes_across_patterns() {
    let session = corpus_session();
    let (code, stdout, stderr) = run_search(
        &session,
        &["fn $A($$$B) { $$$C }", "fn one($$$B) { $$$C }"],
        &["--files-with-matches"],
    );
    assert_eq!(code, 0, "stderr={stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec!["src/a.rs", "src/b.rs"],
        "a.rs matched by BOTH patterns is listed once; got {stdout:?}"
    );
}
