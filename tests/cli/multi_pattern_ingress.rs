//! EXP-013 (GA-21, pass 29): multi-pattern ingress contract.
//!
//! One `asgrep search` invocation carrying N `--pattern` flags must:
//! - run ONE process (one index open, one envelope) — the surface under test;
//! - return hits that are the exact union of the N sequential single-pattern
//!   invocations, grouped by pattern in flag order, each hit tagged with its
//!   pattern via `symbol`;
//! - apply `--limit` PER PATTERN;
//! - fail the WHOLE batch loudly when any pattern would fail singly
//!   (fail-closed, H-CONF-006 rule) — no partial envelope;
//! - refuse to run when a QUERY positional and `--pattern` are both given
//!   (the batch form passes the project root via the global `--root` flag).
//!
//! Failure-first: these tests were run against the pre-ingress tree and
//! failed (clap rejected `--pattern` on search entirely).

use ast_sgrep_testkit::CliSession;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

/// (file, line_start, line_end) multiset per pattern — the per-pattern hit set.
fn hits_by_pattern(envelope: &Value) -> BTreeMap<String, Vec<(String, i64, i64)>> {
    let mut by_pattern: BTreeMap<String, Vec<(String, i64, i64)>> = BTreeMap::new();
    for hit in envelope["hits"].as_array().unwrap_or(&vec![]) {
        let pattern = hit["symbol"].as_str().unwrap_or("").to_string();
        by_pattern
            .entry(pattern)
            .or_default()
            .push((
                hit["file"].as_str().unwrap_or("").to_string(),
                hit["line_start"].as_i64().unwrap_or_default(),
                hit["line_end"].as_i64().unwrap_or_default(),
            ));
    }
    for set in by_pattern.values_mut() {
        set.sort();
    }
    by_pattern
}

fn single_pattern_session(session: &CliSession, pattern: &str) -> Value {
    session.search_json(
        &format!("pattern:{pattern}"),
        &["--no-embed", "--no-auto-index"],
    )
}

fn batch_args(session: &CliSession, patterns: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "--index-path",
        session.index_path.to_str().unwrap(),
        "--json",
        "--no-embed",
        "--no-auto-index",
        "--root",
        session.root.to_str().unwrap(),
        "search",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for pattern in patterns {
        args.push("--pattern".into());
        args.push(pattern.to_string());
    }
    args
}

/// Batch with N --pattern flags == concat of N sequential searches, tagged.
#[test]
fn batch_is_tagged_union_of_sequential() {
    let session = CliSession::sample(asgrep_bin());
    let patterns = [
        "validate_input($$$A)",
        "store_token($$$A)",
        "console.log($$$A)",
    ];
    let args = batch_args(&session, &patterns);
    let out = session
        .run_success(&args.iter().map(String::as_str).collect::<Vec<_>>());
    let envelope: Value = serde_json::from_slice(&out.stdout).expect("batch envelope json");

    assert_eq!(envelope["command"], "search", "batch keeps the search command");
    assert_eq!(envelope["ok"], true, "batch envelope ok");
    let batch_by_pattern = hits_by_pattern(&envelope);
    for pattern in patterns {
        let sequential = single_pattern_session(&session, pattern);
        let sequential_sets = hits_by_pattern(&sequential);
        let batch_set = batch_by_pattern
            .get(pattern)
            .unwrap_or_else(|| panic!("batch hits missing tag for pattern {pattern}"));
        assert_eq!(
            batch_set,
            sequential_sets.get(pattern).map(Vec::as_slice).unwrap_or(&[]),
            "per-pattern set differs from sequential for {pattern}"
        );
        assert!(
            !batch_set.is_empty(),
            "pattern {pattern} unexpectedly empty in batch on the sample fixture"
        );
    }
    // Union shape: exactly the requested patterns are represented.
    assert_eq!(
        batch_by_pattern.len(),
        patterns.len(),
        "batch must carry exactly the requested pattern tags, got {batch_by_pattern:?}"
    );
    // Every validated pattern is non-empty on the sample fixture: the union
    // must be strictly more than any single pattern.
    let total: usize = batch_by_pattern.values().map(Vec::len).sum();
    assert!(total >= patterns.len(), "batch lost hits: total={total}");
}

/// The batch query field is the D-03 multi-token shape.
#[test]
fn batch_envelope_query_lists_all_pattern_tokens() {
    let session = CliSession::sample(asgrep_bin());
    let args = batch_args(&session, &["validate_input($$$A)", "console.log($$$A)"]);
    let out = session
        .run_success(&args.iter().map(String::as_str).collect::<Vec<_>>());
    let envelope: Value = serde_json::from_slice(&out.stdout).expect("batch envelope json");
    let query = envelope["query"].as_str().expect("query field");
    assert!(
        query.contains("pattern:validate_input($$$A)")
            && query.contains("pattern:console.log($$$A)"),
        "query must list every pattern token, got: {query}"
    );
}

/// A leading `pattern:` token on a --pattern value is tolerated (D-03 shape).
#[test]
fn batch_accepts_pattern_prefixed_values() {
    let session = CliSession::sample(asgrep_bin());
    let run = |pattern: &str| {
        let args = batch_args(&session, &[pattern]);
        let out = session
            .run_success(&args.iter().map(String::as_str).collect::<Vec<_>>());
        let envelope: Value = serde_json::from_slice(&out.stdout).unwrap();
        hits_by_pattern(&envelope)
    };
    assert_eq!(
        run("console.log($$$A)"),
        run("pattern:console.log($$$A)"),
        "pattern:-prefixed value must behave identically"
    );
}

/// `--limit` applies PER PATTERN (sequential equivalence), not to the batch.
#[test]
fn batch_limit_applies_per_pattern() {
    let session = CliSession::sample(asgrep_bin());
    // Vehicle: `def $A` matches all ten defs on the sample fixture
    // (main.py + app.rb). (The previous vehicle, `def $A($$$B):`, matches
    // nothing — verified against ast-grep 0.45.3, which also returns zero
    // for the trailing-colon form — so the limit was never exercised.)
    let uncut = single_pattern_session(&session, "def $A");
    assert!(
        uncut["hits"].as_array().map_or(0, Vec::len) > 1,
        "vehicle must match several hits or the limit assertion is vacuous"
    );
    let mut args = batch_args(&session, &["def $A", "console.log($$$A)"]);
    // Insert a global --limit 1 in front of the subcommand.
    let pos = args.iter().position(|a| a == "--no-auto-index").unwrap() + 1;
    args.insert(pos, "1".into());
    args.insert(pos, "--limit".into());
    let out = session
        .run_success(&args.iter().map(String::as_str).collect::<Vec<_>>());
    let envelope: Value = serde_json::from_slice(&out.stdout).expect("batch envelope json");
    let by_pattern = hits_by_pattern(&envelope);
    let def_hits = by_pattern.get("def $A").expect("decl pattern tag");
    assert_eq!(
        def_hits.len(),
        1,
        "--limit 1 must cap EACH pattern at 1 hit, got {def_hits:?}"
    );
    // A batch-global limit would have starved the second pattern entirely.
    assert!(
        by_pattern.contains_key("console.log($$$A)"),
        "second pattern must keep its own limit budget, got {by_pattern:?}"
    );
}

/// Fail-closed: one failing pattern fails the WHOLE batch (no partial envelope).
#[test]
fn batch_fails_closed_on_fallback_pattern() {
    let session = CliSession::sample(asgrep_bin());
    let args = batch_args(&session, &["console.log($$$A)", "$A += $B"]);
    let out = session
        .run_failure(&args.iter().map(String::as_str).collect::<Vec<_>>());
    let envelope: Value = serde_json::from_slice(&out.stdout).expect("error envelope json");
    assert_eq!(envelope["ok"], false, "batch with a fallback pattern is loud");
    let message = envelope["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("structural fallback"),
        "error must name the failing pattern class, got: {message}"
    );
    // And the same shape fails when run singly (contract mirrored, not invented).
    let single_args = batch_args(&session, &["$A += $B"]);
    let single = session.run_failure(
        &single_args.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    let single_envelope: Value = serde_json::from_slice(&single.stdout).unwrap();
    assert_eq!(single_envelope["ok"], false);
}

/// QUERY positional and --pattern are mutually exclusive (loud usage error).
#[test]
fn query_and_pattern_flags_are_exclusive() {
    let session = CliSession::sample(asgrep_bin());
    let out = session.run_failure(&[
        "--index-path",
        session.index_path.to_str().unwrap(),
        "--json",
        "search",
        "pattern:console.log($$$A)",
        "--pattern",
        "store_token($$$A)",
        session.root.to_str().unwrap(),
    ]);
    // Usage errors surface as a machine envelope on stdout (empty stderr).
    let envelope: Value = serde_json::from_slice(&out.stdout).expect("usage envelope json");
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["kind"], "usage");
    let message = envelope["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.to_lowercase().contains("pattern"),
        "usage error should mention the conflicting flag, got: {message}"
    );
}
