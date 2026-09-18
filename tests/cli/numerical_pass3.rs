//! N3 metamorphic-numeric relations, pass 3: orderings and invariances that
//! must hold BETWEEN runs/flags/renderings, not absolute oracle values.
//!
//! Relations under test (all probed against the real binary):
//!
//! - Search `--limit` growth never shrinks the hit list, and the reported
//!   hits never exceed the limit; a smaller limit's hits are a prefix of a
//!   larger limit's hits (rank-then-truncate).
//! - Search reruns yield byte-identical hit payloads.
//! - Eval `k` growth never lowers `found`/`recall_at_k`, and `first_rank`
//!   never worsens (null ranks as +inf).
//! - Eval recall cutoffs are monotone: recall@1 <= recall@5 <= recall@20,
//!   per query and in the aggregate.
//! - A rank-1-hit query scores no lower than a total-miss query on
//!   RR/nDCG/recall at every cutoff.
//! - Aggregates sit within rounding distance of the per-query mean and
//!   within the per-query [min, max] band (+/- 0.001 rounding slack).
//! - Every reported float is a `round3` fixed point: `v * 1000` is
//!   integral (black-box idempotence; `round3` itself is private).
//! - Eval reruns yield byte-identical metric payloads (`queries` +
//!   `aggregate` serialization; the full envelope embeds a fresh temp
//!   index path per run, so full-stdout comparison is excluded by design).
//! - Human and JSON renderings agree numerically (within 0.001); the
//!   `found/relevant` pair agrees exactly.
//!
//! Deliberately NOT covered here (already pinned): exact metric arithmetic
//! and budget-cap messages (`numerical_pass1`), degenerate-input totality
//! (`numerical_pass2`).

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

fn run(bin: &PathBuf, args: &[&str]) -> Output {
    Command::new(bin)
        .args(args)
        .output()
        .expect("run asgrep")
}

fn parse_stdout(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout is JSON")
}

fn write_fixture(root: &std::path::Path, name: &str, body: &str) {
    std::fs::write(root.join(name), body).expect("write fixture file");
}

fn write_gold(dir: &std::path::Path, name: &str, body: &Value) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body.to_string()).expect("write gold");
    path
}

fn f64_at(value: &Value, pointer: &str) -> f64 {
    value
        .pointer(pointer)
        .unwrap_or_else(|| panic!("missing {pointer} in {value}"))
        .as_f64()
        .unwrap_or_else(|| panic!("{pointer} is not a number in {value}"))
}

/// Build a project with `n_files` files sharing one term, then index it.
fn indexed_project(term: &str, n_files: usize) -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for i in 0..n_files {
        write_fixture(&root, &format!("m{i:02}.rs"), &format!("fn {term}() {{}}\n"));
    }
    let bin = asgrep_bin();
    let output = run(&bin, &["--json", "--no-embed", "index", root.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "index must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (temp, root)
}

fn run_search_json(root: &std::path::Path, limit: usize, query: &str) -> Value {
    let bin = asgrep_bin();
    let limit_arg = limit.to_string();
    let output = run(
        &bin,
        &[
            "--json",
            "--no-embed",
            "--limit",
            &limit_arg,
            query,
            root.to_str().unwrap(),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "search must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_stdout(&output)
}

fn run_eval_json(gold: &std::path::Path, root: &std::path::Path) -> Value {
    let bin = asgrep_bin();
    let output = run(
        &bin,
        &[
            "--json",
            "--no-embed",
            "eval",
            "--gold",
            gold.to_str().unwrap(),
            root.to_str().unwrap(),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "eval must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_stdout(&output)
}

/// Rank with null (miss) treated as +inf, so "larger k never ranks worse"
/// is a plain `<=` comparison.
fn rank_key(value: &Value) -> usize {
    if value.is_null() { usize::MAX } else { value.as_u64().unwrap() as usize }
}

// --- search: limit growth never shrinks hits --------------------------------

#[test]
fn search_limit_growth_never_shrinks_hits() {
    // Twelve files share one term; sweeping the limit upward must never
    // drop a hit, and no run may report more hits than its limit.
    let (_temp, root) = indexed_project("wobblebuild_n3lim", 12);
    let mut prev = 0usize;
    for limit in [1usize, 2, 5, 10] {
        let value = run_search_json(&root, limit, "wobblebuild_n3lim");
        assert_eq!(value["ok"], true, "{value}");
        let n = value["hits"].as_array().expect("hits array").len();
        assert!(n <= limit, "hits ({n}) must not exceed limit ({limit}): {value}");
        assert!(
            n >= prev,
            "larger limit must never yield fewer hits: {prev} -> {n} at limit {limit}"
        );
        prev = n;
    }
    assert!(prev > 1, "sweep must discriminate: final hit count {prev}");
}

#[test]
fn search_smaller_limit_hits_are_a_prefix_of_larger() {
    // Rank-then-truncate: the limit-2 hit list must equal the first two
    // entries of the limit-10 hit list, as full JSON objects.
    let (_temp, root) = indexed_project("wobblebuild_n3pfx", 12);
    let small = run_search_json(&root, 2, "wobblebuild_n3pfx");
    let large = run_search_json(&root, 10, "wobblebuild_n3pfx");
    let small_hits = small["hits"].as_array().expect("hits array");
    let large_hits = large["hits"].as_array().expect("hits array");
    assert_eq!(small_hits.len(), 2, "limit 2 must yield exactly 2 hits: {small}");
    assert!(
        large_hits.len() >= small_hits.len(),
        "larger limit must cover smaller: {large}"
    );
    assert_eq!(
        &large_hits[..small_hits.len()],
        &small_hits[..],
        "small-limit hits must be a prefix of large-limit hits"
    );
}

#[test]
fn search_rerun_yields_identical_hit_bytes() {
    // Same query twice: the serialized hit payload must be byte-identical.
    let (_temp, root) = indexed_project("wobblebuild_n3det", 6);
    let a = run_search_json(&root, 10, "wobblebuild_n3det");
    let b = run_search_json(&root, 10, "wobblebuild_n3det");
    let bytes_a = serde_json::to_string(&a["hits"]).unwrap();
    let bytes_b = serde_json::to_string(&b["hits"]).unwrap();
    assert!(!a["hits"].as_array().unwrap().is_empty(), "{a}");
    assert_eq!(bytes_a, bytes_b, "rerun hit payload bytes must be identical");
}

// --- eval: k growth never lowers found/recall --------------------------------

#[test]
fn eval_k_growth_never_lowers_found_or_recall() {
    // Three relevant files share one term. Sweeping k upward: found and
    // recall_at_k are non-decreasing, and first_rank never worsens.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for name in ["k1.rs", "k2.rs", "k3.rs"] {
        write_fixture(&root, name, "fn wobblebuild_n3k() {}\n");
    }
    let relevant =
        serde_json::json!([{"file": "k1.rs"}, {"file": "k2.rs"}, {"file": "k3.rs"}]);
    let mut prev_found = 0u64;
    let mut prev_recall = 0.0f64;
    let mut prev_rank = usize::MAX;
    for k in [1u64, 2, 5] {
        let gold = write_gold(
            temp.path(),
            &format!("gold{k}.json"),
            &serde_json::json!({"corpus": "n3", "queries": [
                {"name": "grow", "query": "wobblebuild_n3k", "k": k, "relevant": relevant}
            ]}),
        );
        let value = run_eval_json(&gold, &root);
        let found = value["queries"][0]["found"].as_u64().expect("found");
        let recall = f64_at(&value, "/aggregate/recall_at_k");
        let rank = rank_key(&value["queries"][0]["first_rank"]);
        assert!(found >= prev_found, "found must not shrink with k: {prev_found} -> {found}");
        assert!(recall >= prev_recall, "recall_at_k must not shrink with k: {prev_recall} -> {recall}");
        assert!(rank <= prev_rank, "first_rank must not worsen with k: {prev_rank:?} -> {rank:?}");
        prev_found = found;
        prev_recall = recall;
        prev_rank = rank;
    }
    assert_eq!(prev_found, 3, "k=5 must find all three relevant files");
}

// --- eval: recall cutoffs monotone --------------------------------------------

#[test]
fn eval_recall_cutoffs_are_monotone() {
    // recall@1 <= recall@5 <= recall@20 per query and in the aggregate:
    // wider cutoffs scan a superset of the narrower scan.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for name in ["c1.rs", "c2.rs", "c3.rs", "c4.rs"] {
        write_fixture(&root, name, "fn wobblebuild_n3cut() {}\n");
    }
    write_fixture(&root, "decoy.rs", "fn wobblebuild_unrelated() {}\n");
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n3", "queries": [
            {"name": "cut", "query": "wobblebuild_n3cut", "k": 20,
             "relevant": [{"file": "c1.rs"}, {"file": "c2.rs"}, {"file": "c3.rs"}, {"file": "c4.rs"}]},
            {"name": "miss", "query": "qqqxq_missing_zzz", "k": 20,
             "relevant": [{"file": "decoy.rs"}]}
        ]}),
    );
    let value = run_eval_json(&gold, &root);
    for qi in [0, 1] {
        let r1 = f64_at(&value, &format!("/queries/{qi}/recall_at/1"));
        let r5 = f64_at(&value, &format!("/queries/{qi}/recall_at/5"));
        let r20 = f64_at(&value, &format!("/queries/{qi}/recall_at/20"));
        assert!(r1 <= r5 && r5 <= r20, "query {qi} cutoffs must be monotone: {r1} <= {r5} <= {r20}");
    }
    let a1 = f64_at(&value, "/aggregate/recall_at_1");
    let a5 = f64_at(&value, "/aggregate/recall_at_5");
    let a20 = f64_at(&value, "/aggregate/recall_at_20");
    assert!(a1 <= a5 && a5 <= a20, "aggregate cutoffs must be monotone: {a1} <= {a5} <= {a20}");
}

// --- eval: better ranking never scores lower -----------------------------------

#[test]
fn eval_hit_scores_no_lower_than_miss() {
    // A rank-1-hit query dominates a total-miss query on RR, nDCG, and
    // recall at every cutoff.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "hit.rs", "fn wobblebuild_n3dom() {}\n");
    write_fixture(&root, "other.rs", "fn wobblebuild_unrelated() {}\n");
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n3", "queries": [
            {"name": "q_hit", "query": "wobblebuild_n3dom", "k": 5,
             "relevant": [{"file": "hit.rs"}]},
            {"name": "q_miss", "query": "qqqxq_missing_zzz", "k": 5,
             "relevant": [{"file": "other.rs"}]}
        ]}),
    );
    let value = run_eval_json(&gold, &root);
    assert_eq!(value["queries"][0]["first_rank"], 1, "{value}");
    assert!(value["queries"][1]["first_rank"].is_null(), "{value}");
    for pointer in ["/rr", "/ndcg", "/recall_at/1", "/recall_at/5", "/recall_at/20"] {
        let hit = f64_at(&value, &format!("/queries/0{pointer}"));
        let miss = f64_at(&value, &format!("/queries/1{pointer}"));
        assert!(hit >= miss, "hit ({hit}) must score no lower than miss ({miss}) on {pointer}");
    }
    assert!(f64_at(&value, "/queries/0/rr") > f64_at(&value, "/queries/1/rr"));
}

// --- eval: aggregates stay inside the per-query band ---------------------------

#[test]
fn eval_aggregates_stay_within_per_query_band() {
    // Aggregates are means of unrounded per-query values, so each must
    // land within [min, max] of the rendered per-query values, plus at
    // most 0.001 of rounding slack on each side.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for name in ["b1.rs", "b2.rs", "b3.rs"] {
        write_fixture(&root, name, "fn wobblebuild_n3band() {}\n");
    }
    write_fixture(&root, "other.rs", "fn wobblebuild_unrelated() {}\n");
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n3", "queries": [
            {"name": "trio", "query": "wobblebuild_n3band", "k": 2,
             "relevant": [{"file": "b1.rs"}, {"file": "b2.rs"}, {"file": "b3.rs"}]},
            {"name": "miss", "query": "qqqxq_missing_zzz", "k": 2,
             "relevant": [{"file": "other.rs"}]}
        ]}),
    );
    let value = run_eval_json(&gold, &root);
    let per_query = |pointer: &str| -> Vec<f64> {
        (0..2).map(|qi| f64_at(&value, &format!("/queries/{qi}{pointer}"))).collect()
    };
    for (per_q, agg) in [
        ("/rr", "/aggregate/mrr"),
        ("/ndcg", "/aggregate/ndcg"),
        ("/recall_at/1", "/aggregate/recall_at_1"),
        ("/recall_at/5", "/aggregate/recall_at_5"),
        ("/recall_at/20", "/aggregate/recall_at_20"),
    ] {
        let vals = per_query(per_q);
        let lo = vals.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let got = f64_at(&value, agg);
        assert!(
            got >= lo - 0.001 - 1e-9 && got <= hi + 0.001 + 1e-9,
            "{agg}={got} must sit in per-query [{lo}, {hi}] +/- rounding: {value}"
        );
    }
    // recall_at_k is the mean of found/relevant: recompute exactly.
    let mut sum = 0.0f64;
    for qi in 0..2 {
        let found = value["queries"][qi]["found"].as_u64().unwrap() as f64;
        let relevant = value["queries"][qi]["relevant"].as_u64().unwrap() as f64;
        sum += found / relevant;
    }
    let expected = ((sum / 2.0) * 1000.0).round() / 1000.0;
    let got = f64_at(&value, "/aggregate/recall_at_k");
    assert!(
        (got - expected).abs() <= 1e-9,
        "recall_at_k must equal round3(mean(found/relevant)): {got} vs {expected}"
    );
}

// --- eval: reported floats are round3 fixed points -------------------------------

#[test]
fn eval_reported_floats_are_round3_fixed_points() {
    // Black-box round3 idempotence: every reported float v must satisfy
    // round3(v) == v, i.e. v * 1000 is integral. The 2-of-3 shape forces
    // repeating decimals (0.333/0.667) through the rounding path.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for name in ["f1.rs", "f2.rs", "f3.rs"] {
        write_fixture(&root, name, "fn wobblebuild_n3fix() {}\n");
    }
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n3", "queries": [
            {"name": "trio", "query": "wobblebuild_n3fix", "k": 2,
             "relevant": [{"file": "f1.rs"}, {"file": "f2.rs"}, {"file": "f3.rs"}]}
        ]}),
    );
    let value = run_eval_json(&gold, &root);
    let mut checked = 0usize;
    let mut check = |pointer: &str| {
        let v = f64_at(&value, pointer);
        let scaled = v * 1000.0;
        assert!(
            (scaled - scaled.round()).abs() <= 1e-6,
            "{pointer}={v} is not a round3 fixed point"
        );
        checked += 1;
    };
    for pointer in [
        "/queries/0/rr",
        "/queries/0/ndcg",
        "/queries/0/recall_at/1",
        "/queries/0/recall_at/5",
        "/queries/0/recall_at/20",
        "/aggregate/mrr",
        "/aggregate/ndcg",
        "/aggregate/recall_at_k",
        "/aggregate/recall_at_1",
        "/aggregate/recall_at_5",
        "/aggregate/recall_at_20",
    ] {
        check(pointer);
    }
    assert_eq!(checked, 11);
}

// --- eval: determinism across reruns ---------------------------------------------

#[test]
fn eval_rerun_yields_identical_metric_bytes() {
    // Same gold + root twice: the serialized metric payload (queries +
    // aggregate) must be byte-identical. The full envelope is excluded
    // because it embeds a fresh temp index path per run.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for name in ["d1.rs", "d2.rs", "d3.rs"] {
        write_fixture(&root, name, "fn wobblebuild_n3det() {}\n");
    }
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n3", "queries": [
            {"name": "trio", "query": "wobblebuild_n3det", "k": 2,
             "relevant": [{"file": "d1.rs"}, {"file": "d2.rs"}, {"file": "d3.rs"}]},
            {"name": "miss", "query": "qqqxq_missing_zzz", "k": 2,
             "relevant": [{"file": "d1.rs"}]}
        ]}),
    );
    let a = run_eval_json(&gold, &root);
    let b = run_eval_json(&gold, &root);
    for pointer in ["/queries", "/aggregate"] {
        let bytes_a = serde_json::to_string(a.pointer(pointer).unwrap()).unwrap();
        let bytes_b = serde_json::to_string(b.pointer(pointer).unwrap()).unwrap();
        assert_eq!(bytes_a, bytes_b, "rerun {pointer} bytes must be identical");
    }
}

// --- eval: human-vs-JSON agreement -------------------------------------------------

/// Parse the human `| name | rank | rr | found/relevant | ndcg |` row into
/// (rank_or_miss, rr, found, relevant, ndcg). Numeric fields only.
fn parse_human_row(stdout: &str, name: &str) -> (String, f64, u64, u64, f64) {
    let line = stdout
        .lines()
        .find(|l| l.starts_with(&format!("| {name} |")))
        .unwrap_or_else(|| panic!("missing human row for {name}: {stdout}"));
    let cells: Vec<&str> = line.split('|').map(str::trim).collect();
    assert_eq!(cells.len(), 7, "human row must have 5 cells: {line}");
    let counts: Vec<&str> = cells[4].split('/').collect();
    (
        cells[2].to_owned(),
        cells[3].parse().expect("rr parses"),
        counts[0].parse().expect("found parses"),
        counts[1].parse().expect("relevant parses"),
        cells[5].parse().expect("ndcg parses"),
    )
}

/// Parse the human `MRR=.. Recall@k=.. nDCG@k=.. Recall@1=.. Recall@5=..
/// Recall@20=.. n=..` summary line into its seven numeric fields.
fn parse_human_summary(stdout: &str) -> (f64, f64, f64, f64, f64, f64, u64) {
    let line = stdout
        .lines()
        .find(|l| l.starts_with("MRR="))
        .unwrap_or_else(|| panic!("missing human summary: {stdout}"));
    let mut vals = [0.0f64; 6];
    let mut n = 0u64;
    for token in line.split_whitespace() {
        let (key, raw) = token.split_once('=').expect("key=value token");
        match key {
            "MRR" => vals[0] = raw.parse().expect("MRR parses"),
            "Recall@k" => vals[1] = raw.parse().expect("Recall@k parses"),
            "nDCG@k" => vals[2] = raw.parse().expect("nDCG@k parses"),
            "Recall@1" => vals[3] = raw.parse().expect("Recall@1 parses"),
            "Recall@5" => vals[4] = raw.parse().expect("Recall@5 parses"),
            "Recall@20" => vals[5] = raw.parse().expect("Recall@20 parses"),
            "n" => n = raw.parse().expect("n parses"),
            other => panic!("unexpected summary token {other}"),
        }
    }
    (vals[0], vals[1], vals[2], vals[3], vals[4], vals[5], n)
}

#[test]
fn eval_human_and_json_metrics_agree() {
    // Same gold + root in both renderings: per-query rr/ndcg and every
    // aggregate float must agree within 0.001 (human {:.3} vs JSON
    // round3); found/relevant/first_rank agree exactly.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for name in ["g1.rs", "g2.rs", "g3.rs"] {
        write_fixture(&root, name, "fn wobblebuild_n3hum() {}\n");
    }
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n3", "queries": [
            {"name": "trio", "query": "wobblebuild_n3hum", "k": 2,
             "relevant": [{"file": "g1.rs"}, {"file": "g2.rs"}, {"file": "g3.rs"}]},
            {"name": "miss", "query": "qqqxq_missing_zzz", "k": 2,
             "relevant": [{"file": "g1.rs"}]}
        ]}),
    );
    let json_value = run_eval_json(&gold, &root);
    let bin = asgrep_bin();
    let output = run(
        &bin,
        &[
            "--no-embed",
            "eval",
            "--gold",
            gold.to_str().unwrap(),
            root.to_str().unwrap(),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "human eval must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for (qi, name) in ["trio", "miss"].iter().enumerate() {
        let (rank, rr, found, relevant, ndcg) = parse_human_row(&stdout, name);
        let q = &json_value["queries"][qi];
        let expected_rank = if q["first_rank"].is_null() {
            "miss".to_owned()
        } else {
            q["first_rank"].as_u64().unwrap().to_string()
        };
        assert_eq!(rank, expected_rank, "{name}: first_rank must agree exactly");
        assert_eq!(found, q["found"].as_u64().unwrap(), "{name}: found must agree exactly");
        assert_eq!(relevant, q["relevant"].as_u64().unwrap(), "{name}: relevant must agree exactly");
        for (label, human, pointer) in [("rr", rr, "/rr"), ("ndcg", ndcg, "/ndcg")] {
            let machine = f64_at(&json_value, &format!("/queries/{qi}{pointer}"));
            assert!(
                (human - machine).abs() <= 0.001 + 1e-9,
                "{name}: human {label}={human} vs json {machine}"
            );
        }
    }
    let (mrr, rk, ndcg, r1, r5, r20, n) = parse_human_summary(&stdout);
    assert_eq!(n, json_value["aggregate"]["n_queries"].as_u64().unwrap());
    for (label, human, pointer) in [
        ("MRR", mrr, "/aggregate/mrr"),
        ("Recall@k", rk, "/aggregate/recall_at_k"),
        ("nDCG@k", ndcg, "/aggregate/ndcg"),
        ("Recall@1", r1, "/aggregate/recall_at_1"),
        ("Recall@5", r5, "/aggregate/recall_at_5"),
        ("Recall@20", r20, "/aggregate/recall_at_20"),
    ] {
        let machine = f64_at(&json_value, pointer);
        assert!(
            (human - machine).abs() <= 0.001 + 1e-9,
            "human {label}={human} vs json {machine}"
        );
    }
}
