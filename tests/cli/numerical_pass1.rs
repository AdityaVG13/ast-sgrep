//! N1 numerical oracles, pass 1: eval metric math + token-budget caps.
//!
//! Contract under test (all expectations hand-computed, rank-order
//! independent unless noted):
//!
//! - `eval` DCG/nDCG/RR/MRR/recall arithmetic (`eval.rs`): DCG sums
//!   `1/log2(rank+1)` over matched ranks; nDCG divides by the ideal DCG
//!   over `min(relevant, k)`; RR is `1/first_rank` (0 on miss); every
//!   reported float passes through `round3` (human table: `{:.3}`).
//! - `parse_bounded_usize` caps (`cli_args.rs`): `--budget-tokens` /
//!   `--response-snippet-tokens` max 65536, `--snippet-tokens` max 4096.
//!   Existing `bounded_arguments_are_json_usage_errors` only pins the
//!   envelope shape with the message redacted; these tests pin the exact
//!   numeric clause in the message plus max-value acceptance.
//!
//! Deliberately NOT covered here (already oracle-covered): cpu-limit /
//! duty-cycle math (`oracle_foundry_pass1/2`), codemode-batch byte cap,
//! `--limit`/`--excerpt-lines` rejection envelopes (`machine_contracts`).

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

/// Run `asgrep --json --no-embed eval --gold <gold> <root>`; assert success.
fn run_eval_ok(gold: &std::path::Path, root: &std::path::Path) -> Value {
    let bin = asgrep_bin();
    let output = run(
        &bin,
        &[
            "--json",
            "--no-embed",
            "eval",
            "--gold",
            gold.to_str().expect("gold utf8"),
            root.to_str().expect("root utf8"),
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

fn write_gold(dir: &std::path::Path, body: &Value) -> PathBuf {
    let path = dir.join("gold.json");
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

// --- eval: total miss -> exact zeros -------------------------------------

#[test]
fn eval_total_miss_yields_exact_zeros() {
    // Query term appears in NO file, so first_rank=null, RR=0, DCG=0,
    // nDCG=0/IDCG(1)=0, recall=0/1=0 at every cutoff.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "present.rs", "fn wobblebuild_here() {}\n");
    let gold = write_gold(
        temp.path(),
        &serde_json::json!({"corpus": "n1", "queries": [
            {"name": "miss", "query": "qqqxq_missing_zzz", "k": 5,
             "relevant": [{"file": "present.rs"}]}
        ]}),
    );
    let value = run_eval_ok(&gold, &root);
    let q = &value["queries"][0];
    assert!(q["first_rank"].is_null(), "miss has no rank: {q}");
    assert_eq!(f64_at(&value, "/queries/0/rr"), 0.0);
    assert_eq!(q["found"], 0);
    assert_eq!(q["relevant"], 1);
    assert_eq!(f64_at(&value, "/queries/0/ndcg"), 0.0);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/1"), 0.0);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/5"), 0.0);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/20"), 0.0);
    assert_eq!(f64_at(&value, "/aggregate/mrr"), 0.0);
    assert_eq!(f64_at(&value, "/aggregate/ndcg"), 0.0);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_k"), 0.0);
    assert_eq!(value["aggregate"]["n_queries"], 1);
}

// --- eval: single rank-1 hit -> exact ones -------------------------------

#[test]
fn eval_single_rank1_hit_yields_exact_ones() {
    // One file holds the term (once), so the first hit is rank 1:
    // RR=1/1=1; DCG=1/log2(2)=1; IDCG(min(1,5)=1)=1; nDCG=1; recall=1.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "solo.rs", "fn wobblebuild_solo() {}\n");
    let gold = write_gold(
        temp.path(),
        &serde_json::json!({"corpus": "n1", "queries": [
            {"name": "solo", "query": "wobblebuild_solo", "k": 5,
             "relevant": [{"file": "solo.rs"}]}
        ]}),
    );
    let value = run_eval_ok(&gold, &root);
    let q = &value["queries"][0];
    assert_eq!(q["first_rank"], 1);
    assert_eq!(f64_at(&value, "/queries/0/rr"), 1.0);
    assert_eq!(q["found"], 1);
    assert_eq!(f64_at(&value, "/queries/0/ndcg"), 1.0);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/1"), 1.0);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/5"), 1.0);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/20"), 1.0);
    assert_eq!(f64_at(&value, "/aggregate/mrr"), 1.0);
    assert_eq!(f64_at(&value, "/aggregate/ndcg"), 1.0);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_k"), 1.0);
}

// --- eval: k=1 truncation halves recall, keeps nDCG at 1 ----------------

#[test]
fn eval_k1_truncation_halves_recall_keeps_ndcg_one() {
    // Two relevant files share the term; k=1 scans only the rank-1 hit.
    // found=1 regardless of which file ranks first: recall=1/2=0.5 at
    // every cutoff (all cutoffs clamp to min(n,1)=1), while
    // DCG=1/log2(2)=1 and IDCG(min(2,1)=1)=1 give nDCG=1.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "ka.rs", "fn wobblebuild_kk() {}\n");
    write_fixture(&root, "kb.rs", "fn wobblebuild_kk() {}\n");
    let gold = write_gold(
        temp.path(),
        &serde_json::json!({"corpus": "n1", "queries": [
            {"name": "k1", "query": "wobblebuild_kk", "k": 1,
             "relevant": [{"file": "ka.rs"}, {"file": "kb.rs"}]}
        ]}),
    );
    let value = run_eval_ok(&gold, &root);
    let q = &value["queries"][0];
    assert_eq!(q["first_rank"], 1);
    assert_eq!(f64_at(&value, "/queries/0/rr"), 1.0);
    assert_eq!(q["found"], 1);
    assert_eq!(f64_at(&value, "/queries/0/ndcg"), 1.0);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/1"), 0.5);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/5"), 0.5);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/20"), 0.5);
    assert_eq!(f64_at(&value, "/aggregate/mrr"), 1.0);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_k"), 0.5);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_1"), 0.5);
}

// --- eval: 2-of-3 recall exercises round3 --------------------------------

#[test]
fn eval_two_of_three_rounds_recall_to_667_and_333() {
    // Three relevant files share the term; k=2 scans ranks 1-2, both
    // relevant however ordered: found=2, RR=1.
    // DCG = 1/log2(2)+1/log2(3) = 1.63092975... = IDCG(min(3,2)=2),
    // so nDCG is exactly 1. recall@1 = 1/3 -> round3 -> 0.333;
    // recall@5/20 (cutoff min(n,2)=2) = 2/3 -> round3 -> 0.667.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for name in ["t1.rs", "t2.rs", "t3.rs"] {
        write_fixture(&root, name, "fn wobblebuild_trio() {}\n");
    }
    let gold = write_gold(
        temp.path(),
        &serde_json::json!({"corpus": "n1", "queries": [
            {"name": "trio", "query": "wobblebuild_trio", "k": 2,
             "relevant": [{"file": "t1.rs"}, {"file": "t2.rs"}, {"file": "t3.rs"}]}
        ]}),
    );
    let value = run_eval_ok(&gold, &root);
    let q = &value["queries"][0];
    assert_eq!(q["first_rank"], 1);
    assert_eq!(f64_at(&value, "/queries/0/rr"), 1.0);
    assert_eq!(q["found"], 2);
    assert_eq!(q["relevant"], 3);
    assert_eq!(f64_at(&value, "/queries/0/ndcg"), 1.0);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/1"), 0.333);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/5"), 0.667);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/20"), 0.667);
    assert_eq!(f64_at(&value, "/aggregate/mrr"), 1.0);
    assert_eq!(f64_at(&value, "/aggregate/ndcg"), 1.0);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_k"), 0.667);
}

// --- eval: hit + miss averages to one half -------------------------------

#[test]
fn eval_mixed_hit_miss_halves_mrr_and_ndcg() {
    // q_hit: RR=1, nDCG=1. q_miss: gibberish matches nothing: RR=0,
    // nDCG=0. Aggregate over n=2: MRR=(1+0)/2=0.5, nDCG=0.5,
    // recall@k=(1+0)/2=0.5.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "hit.rs", "fn wobblebuild_mixed() {}\n");
    write_fixture(&root, "other.rs", "fn wobblebuild_unrelated() {}\n");
    let gold = write_gold(
        temp.path(),
        &serde_json::json!({"corpus": "n1", "queries": [
            {"name": "q_hit", "query": "wobblebuild_mixed", "k": 5,
             "relevant": [{"file": "hit.rs"}]},
            {"name": "q_miss", "query": "qqqxq_missing_zzz", "k": 5,
             "relevant": [{"file": "other.rs"}]}
        ]}),
    );
    let value = run_eval_ok(&gold, &root);
    assert_eq!(value["queries"][0]["first_rank"], 1);
    assert_eq!(f64_at(&value, "/queries/0/rr"), 1.0);
    assert!(value["queries"][1]["first_rank"].is_null());
    assert_eq!(f64_at(&value, "/queries/1/rr"), 0.0);
    assert_eq!(f64_at(&value, "/aggregate/mrr"), 0.5);
    assert_eq!(f64_at(&value, "/aggregate/ndcg"), 0.5);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_k"), 0.5);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_1"), 0.5);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_5"), 0.5);
    assert_eq!(value["aggregate"]["n_queries"], 2);
}

// --- eval: human table rounds to three decimals --------------------------

#[test]
fn eval_human_table_rounds_to_three_decimals() {
    // Same 2-of-3 shape as above, human (non-JSON) rendering: the
    // per-query row and the MRR summary line use {:.3}, so 2/3 prints
    // 0.667 and 1/3 prints 0.333.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for name in ["h1.rs", "h2.rs", "h3.rs"] {
        write_fixture(&root, name, "fn wobblebuild_human() {}\n");
    }
    let gold = write_gold(
        temp.path(),
        &serde_json::json!({"corpus": "n1", "queries": [
            {"name": "trio", "query": "wobblebuild_human", "k": 2,
             "relevant": [{"file": "h1.rs"}, {"file": "h2.rs"}, {"file": "h3.rs"}]}
        ]}),
    );
    let bin = asgrep_bin();
    let output = run(
        &bin,
        &[
            "--no-embed",
            "eval",
            "--gold",
            gold.to_str().expect("gold utf8"),
            root.to_str().expect("root utf8"),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "human eval must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("| trio | 1 | 1.000 | 2/3 | 1.000 |"),
        "per-query row must show {{:.3}} rr/ndcg: {stdout}"
    );
    assert!(
        stdout.contains(
            "MRR=1.000  Recall@k=0.667  nDCG@k=1.000  \
             Recall@1=0.333  Recall@5=0.667  Recall@20=0.667  n=1"
        ),
        "summary line must show {{:.3}} aggregates: {stdout}"
    );
}

// --- budget caps: rejection messages name the exact maxima ---------------

fn usage_message(args: &[&str]) -> (Output, Value) {
    let bin = asgrep_bin();
    let output = run(&bin, args);
    assert_eq!(
        output.status.code(),
        Some(1),
        "over-cap flag must be a usage error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "--json usage errors stay on stdout: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(&output);
    assert_eq!(value["error"]["kind"], "usage");
    (output, value)
}

#[test]
fn budget_tokens_cap_names_65536() {
    let (_o, value) = usage_message(&["--json", "--budget-tokens", "65537", "query", "."]);
    let msg = value["error"]["message"].as_str().expect("message");
    assert!(
        msg.contains("--budget-tokens must not exceed 65536"),
        "message must name the exact cap: {msg}"
    );
}

#[test]
fn snippet_tokens_cap_names_4096() {
    let (_o, value) = usage_message(&["--json", "--snippet-tokens", "4097", "query", "."]);
    let msg = value["error"]["message"].as_str().expect("message");
    assert!(
        msg.contains("--snippet-tokens must not exceed 4096"),
        "message must name the exact cap: {msg}"
    );
}

#[test]
fn response_snippet_tokens_cap_names_65536() {
    let (_o, value) = usage_message(&[
        "--json",
        "--response-snippet-tokens",
        "65537",
        "query",
        ".",
    ]);
    let msg = value["error"]["message"].as_str().expect("message");
    assert!(
        msg.contains("--response-snippet-tokens must not exceed 65536"),
        "message must name the exact cap: {msg}"
    );
}

#[test]
fn budget_tokens_max_is_accepted() {
    // 65536 is the cap itself (parse rejects only value > maximum), so a
    // search carrying it must proceed past argument parsing: index the
    // root, then zero hits on a gibberish query stays exit 0 / ok:true
    // per the exit contract.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "a.rs", "fn wobblebuild_cap() {}\n");
    let bin = asgrep_bin();
    let indexed = run(&bin, &["--json", "--no-embed", "index", root.to_str().unwrap()]);
    assert_eq!(
        indexed.status.code(),
        Some(0),
        "index must succeed: {}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    let output = run(
        &bin,
        &[
            "--json",
            "--no-embed",
            "--budget-tokens",
            "65536",
            "qqqxq_missing_zzz",
            root.to_str().expect("root utf8"),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "max budget must parse and search must stay exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(&output);
    assert_eq!(value["ok"], true, "{value}");
}
