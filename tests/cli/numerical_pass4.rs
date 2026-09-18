//! N4 end-to-end metric drills, pass 4: FULL eval/search flows on hand-built
//! corpora with hand-computable outcomes.
//!
//! Relations under test (all probed against the real binary, exact numeric or
//! byte assertions only -- never message text):
//!
//! - Multi-query eval aggregates are hand-computable means: a hit + miss +
//!   2-of-4(k=2) corpus yields MRR=nDCG=0.667, recall@k=0.5, recall@1=0.417.
//! - A 6-relevant k=5 eval rounds recall to 1/6=0.167 and 5/6=0.833.
//! - An eval k-sweep (k=1..5 over 4 relevant) yields found 1,2,3,4,4 and
//!   recall@k 0.25,0.5,0.75,1.0,1.0 with nDCG=RR=1 throughout.
//! - A search --limit sweep (limits 1..7 over 5 matching files) yields hit
//!   counts 1,2,3,4,5,5,5 with the limit echoed back and an exact file set.
//! - Adversarial filename-ordered corpora: symmetric single-line matches rank
//!   in path order (the lexical channel emits (path,line)-sorted for
//!   cross-process stability and fusion preserves that order for symmetric
//!   hits), so decoys named to sort before the relevant file deterministically
//!   outrank it: one decoy forces first_rank=2 (RR=0.5, nDCG=0.631), two
//!   decoys force first_rank=3 (RR=0.333, nDCG=0.5 exactly).
//! - A rank-2 adversarial query mixed with a clean rank-1 query pins the
//!   unrounded-mean aggregate semantics: nDCG=0.815, NOT the 0.816 a mean of
//!   rounded per-query values would give.
//! - Search membership is exact: 3 matching files plus 1 fully-unrelated file
//!   yields exactly those 3 hits, with the query-term bytes in every excerpt.
//!
//! Deliberately NOT covered here (already pinned): single-query metric
//! arithmetic and budget caps (`numerical_pass1`), degenerate-input totality
//! (`numerical_pass2`), cross-run/flag/rendering relations (`numerical_pass3`).

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

fn run_eval_ok(gold: &std::path::Path, root: &std::path::Path) -> Value {
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

// --- eval: three-query hand-computed aggregates -------------------------------

#[test]
fn eval_three_query_fractional_aggregates() {
    // q_hit: term in one file, k=5 -> rank 1, RR=1, nDCG=1, recall=1.
    // q_miss: gibberish matches nothing -> rank null, RR=0, nDCG=0, recall=0.
    // q_part: term in 4 files, k=2 -> found=2 however ordered (both scanned
    //   hits relevant), RR=1, DCG=IDCG(min(4,2)=2) so nDCG=1,
    //   recall@1=1/4=0.25, recall@5/20 (cutoff min(n,2)=2)=2/4=0.5.
    // Aggregates over n=3 (means of unrounded values, then round3):
    //   MRR=(1+0+1)/3=0.6666..->0.667; nDCG likewise 0.667;
    //   recall@k=(1+0+0.5)/3=0.5; recall@1=(1+0+0.25)/3=0.4166..->0.417;
    //   recall@5=recall@20=(1+0+0.5)/3=0.5.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "hit.rs", "fn wobble_n4_q1hit() {}\n");
    write_fixture(&root, "other.rs", "fn wobble_n4_q1other() {}\n");
    for name in ["p1.rs", "p2.rs", "p3.rs", "p4.rs"] {
        write_fixture(&root, name, "fn wobble_n4_q1part() {}\n");
    }
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n4", "queries": [
            {"name": "q_hit", "query": "wobble_n4_q1hit", "k": 5,
             "relevant": [{"file": "hit.rs"}]},
            {"name": "q_miss", "query": "qqqxq_missing_zzz", "k": 5,
             "relevant": [{"file": "other.rs"}]},
            {"name": "q_part", "query": "wobble_n4_q1part", "k": 2,
             "relevant": [{"file": "p1.rs"}, {"file": "p2.rs"},
                          {"file": "p3.rs"}, {"file": "p4.rs"}]}
        ]}),
    );
    let value = run_eval_ok(&gold, &root);
    assert_eq!(value["queries"][0]["first_rank"], 1);
    assert!(value["queries"][1]["first_rank"].is_null());
    assert_eq!(value["queries"][2]["first_rank"], 1);
    assert_eq!(value["queries"][2]["found"], 2);
    assert_eq!(f64_at(&value, "/queries/2/recall_at/1"), 0.25);
    assert_eq!(f64_at(&value, "/queries/2/recall_at/5"), 0.5);
    assert_eq!(f64_at(&value, "/queries/2/recall_at/20"), 0.5);
    assert_eq!(f64_at(&value, "/aggregate/mrr"), 0.667);
    assert_eq!(f64_at(&value, "/aggregate/ndcg"), 0.667);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_k"), 0.5);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_1"), 0.417);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_5"), 0.5);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_20"), 0.5);
    assert_eq!(value["aggregate"]["n_queries"], 3);
}

// --- eval: 5-of-6 recall rounds to .833/.167 ----------------------------------

#[test]
fn eval_five_of_six_rounds_recall_to_833_and_167() {
    // Six relevant files share the term; k=5 scans ranks 1-5, all relevant
    // however ordered: found=5, RR=1,
    // DCG(ranks 1-5) = IDCG(min(6,5)=5) so nDCG=1.
    // recall@1 = 1/6 = 0.1666.. -> 0.167;
    // recall@5/20 (cutoff min(n,5)=5) = 5/6 = 0.8333.. -> 0.833.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for name in ["s1.rs", "s2.rs", "s3.rs", "s4.rs", "s5.rs", "s6.rs"] {
        write_fixture(&root, name, "fn wobble_n4_sextet() {}\n");
    }
    let relevant = serde_json::json!([
        {"file": "s1.rs"}, {"file": "s2.rs"}, {"file": "s3.rs"},
        {"file": "s4.rs"}, {"file": "s5.rs"}, {"file": "s6.rs"}
    ]);
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n4", "queries": [
            {"name": "sextet", "query": "wobble_n4_sextet", "k": 5,
             "relevant": relevant}
        ]}),
    );
    let value = run_eval_ok(&gold, &root);
    assert_eq!(value["queries"][0]["first_rank"], 1);
    assert_eq!(f64_at(&value, "/queries/0/rr"), 1.0);
    assert_eq!(value["queries"][0]["found"], 5);
    assert_eq!(f64_at(&value, "/queries/0/ndcg"), 1.0);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/1"), 0.167);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/5"), 0.833);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/20"), 0.833);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_k"), 0.833);
}

// --- eval: k-sweep found/recall match hand values -----------------------------

#[test]
fn eval_k_sweep_found_and_recall_match_hand_values() {
    // Four relevant files share the term. At each k the scan takes min(k,4)
    // hits, all relevant however ordered: found=k (capped at 4),
    // recall@k=found/4, RR=1, and DCG=IDCG(min(4,k)) so nDCG=1.
    // k=1..5 -> found 1,2,3,4,4; recall 0.25,0.5,0.75,1.0,1.0.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for name in ["w1.rs", "w2.rs", "w3.rs", "w4.rs"] {
        write_fixture(&root, name, "fn wobble_n4_sweep() {}\n");
    }
    let relevant =
        serde_json::json!([{"file": "w1.rs"}, {"file": "w2.rs"}, {"file": "w3.rs"}, {"file": "w4.rs"}]);
    for (k, expected_found, expected_recall) in
        [(1u64, 1u64, 0.25), (2, 2, 0.5), (3, 3, 0.75), (4, 4, 1.0), (5, 4, 1.0)]
    {
        let gold = write_gold(
            temp.path(),
            &format!("gold{k}.json"),
            &serde_json::json!({"corpus": "n4", "queries": [
                {"name": "sweep", "query": "wobble_n4_sweep", "k": k, "relevant": relevant}
            ]}),
        );
        let value = run_eval_ok(&gold, &root);
        assert_eq!(value["queries"][0]["first_rank"], 1, "k={k}");
        assert_eq!(value["queries"][0]["found"], expected_found, "k={k}");
        assert_eq!(f64_at(&value, "/queries/0/rr"), 1.0);
        assert_eq!(f64_at(&value, "/queries/0/ndcg"), 1.0);
        assert_eq!(
            f64_at(&value, "/aggregate/recall_at_k"),
            expected_recall,
            "k={k}"
        );
    }
}

// --- search: limit sweep hit counts match hand values -------------------------

#[test]
fn search_limit_sweep_hit_counts_match_hand_values() {
    // Five files each hold one matching line (dedup merges the lexical and
    // symbol evidence per file locus into one hit), so the hit count is
    // exactly min(limit, 5): limits 1..7 -> 1,2,3,4,5,5,5. The envelope
    // echoes the requested limit, and at full coverage the sorted file set
    // is exactly the five fixtures.
    let (_temp, root) = indexed_project("wobble_n4_lim", 5);
    for (limit, expected) in [(1usize, 1usize), (2, 2), (3, 3), (4, 4), (5, 5), (6, 5), (7, 5)] {
        let value = run_search_json(&root, limit, "wobble_n4_lim");
        assert_eq!(value["ok"], true, "limit={limit}: {value}");
        assert_eq!(value["limit"], limit, "limit echo: {value}");
        let hits = value["hits"].as_array().expect("hits array");
        assert_eq!(hits.len(), expected, "limit={limit}: {value}");
    }
    let value = run_search_json(&root, 10, "wobble_n4_lim");
    let mut files: Vec<&str> = value["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["file"].as_str().expect("file string"))
        .collect();
    files.sort();
    let mut basenames: Vec<&str> = files
        .iter()
        .map(|f| f.rsplit('/').next().expect("basename"))
        .collect();
    basenames.sort();
    assert_eq!(basenames, ["m00.rs", "m01.rs", "m02.rs", "m03.rs", "m04.rs"]);
}

// --- eval: filename-ordered decoy forces rank 2 -------------------------------

#[test]
fn eval_filename_ordered_decoy_forces_rank_two() {
    // a_decoy.rs and z_rel.rs hold byte-identical matching lines, so the
    // symmetric hits rank in path order: the decoy is rank 1, the relevant
    // file rank 2, deterministically (creation order is irrelevant).
    // RR=1/2=0.5; DCG=1/log2(2+1)=1/log2(3)=0.63092975..->0.631;
    // IDCG(min(1,5)=1)=1/log2(2)=1; nDCG=0.631. recall@1 scans only the
    // decoy -> 0/1=0; recall@5/20 scan both -> 1/1=1.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    // Created in reverse alphabetical order to prove order follows the path
    // sort, not insertion order.
    write_fixture(&root, "z_rel.rs", "fn wobble_n4_rk2() {}\n");
    write_fixture(&root, "a_decoy.rs", "fn wobble_n4_rk2() {}\n");
    write_fixture(&root, "extra.rs", "fn plain_unrelated_symbol() {}\n");
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n4", "queries": [
            {"name": "adv", "query": "wobble_n4_rk2", "k": 5,
             "relevant": [{"file": "z_rel.rs"}]}
        ]}),
    );
    let value = run_eval_ok(&gold, &root);
    let q = &value["queries"][0];
    assert_eq!(q["first_rank"], 2, "{value}");
    assert_eq!(f64_at(&value, "/queries/0/rr"), 0.5);
    assert_eq!(q["found"], 1);
    assert_eq!(q["relevant"], 1);
    assert_eq!(f64_at(&value, "/queries/0/ndcg"), 0.631);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/1"), 0.0);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/5"), 1.0);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/20"), 1.0);
}

// --- eval: two filename-ordered decoys force rank 3 ---------------------------

#[test]
fn eval_two_filename_ordered_decoys_force_rank_three() {
    // Three byte-identical matching lines rank in path order: a_d1 rank 1,
    // b_d2 rank 2, z_rel rank 3. RR=1/3=0.3333..->0.333;
    // DCG=1/log2(3+1)=1/2=0.5 exactly; IDCG(1)=1; nDCG=0.5.
    // recall@1=0 (rank 1 is a decoy); recall@5/20=1.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "a_d1.rs", "fn wobble_n4_rk3() {}\n");
    write_fixture(&root, "b_d2.rs", "fn wobble_n4_rk3() {}\n");
    write_fixture(&root, "z_rel.rs", "fn wobble_n4_rk3() {}\n");
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n4", "queries": [
            {"name": "adv", "query": "wobble_n4_rk3", "k": 5,
             "relevant": [{"file": "z_rel.rs"}]}
        ]}),
    );
    let value = run_eval_ok(&gold, &root);
    let q = &value["queries"][0];
    assert_eq!(q["first_rank"], 3, "{value}");
    assert_eq!(f64_at(&value, "/queries/0/rr"), 0.333);
    assert_eq!(q["found"], 1);
    assert_eq!(f64_at(&value, "/queries/0/ndcg"), 0.5);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/1"), 0.0);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/5"), 1.0);
    assert_eq!(f64_at(&value, "/queries/0/recall_at/20"), 1.0);
}

// --- eval: adversarial + clean aggregate uses unrounded means ------------------

#[test]
fn eval_adversarial_plus_clean_aggregate_uses_unrounded_means() {
    // q_adv replays the rank-2 decoy drill (per-query nDCG renders 0.631
    // but the unrounded value is 1/log2(3)=0.63092975..); q_solo is a clean
    // rank-1 hit (RR=1, nDCG=1) on a token-disjoint symbol so neither query
    // pollutes the other. Aggregates average UNROUNDED per-query values:
    //   MRR=(0.5+1)/2=0.75; recall@k=(1+1)/2=1.0;
    //   recall@1=(0+1)/2=0.5; recall@5=recall@20=1.0;
    //   nDCG=round3((0.63092975..+1)/2)=round3(0.81546487..)=0.815 --
    //   a mean of ROUNDED per-query values would give (0.631+1)/2=0.8155,
    //   which rounds to 0.816, so 0.815 pins the unrounded semantics.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "a_decoy.rs", "fn wobble_n4_q7mu() {}\n");
    write_fixture(&root, "z_rel.rs", "fn wobble_n4_q7mu() {}\n");
    write_fixture(&root, "m_solo.rs", "fn zqxk_q7_solo() {}\n");
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n4", "queries": [
            {"name": "q_adv", "query": "wobble_n4_q7mu", "k": 5,
             "relevant": [{"file": "z_rel.rs"}]},
            {"name": "q_solo", "query": "zqxk_q7_solo", "k": 5,
             "relevant": [{"file": "m_solo.rs"}]}
        ]}),
    );
    let value = run_eval_ok(&gold, &root);
    assert_eq!(value["queries"][0]["first_rank"], 2, "{value}");
    assert_eq!(f64_at(&value, "/queries/0/rr"), 0.5);
    assert_eq!(f64_at(&value, "/queries/0/ndcg"), 0.631);
    assert_eq!(value["queries"][1]["first_rank"], 1, "{value}");
    assert_eq!(f64_at(&value, "/queries/1/rr"), 1.0);
    assert_eq!(f64_at(&value, "/queries/1/ndcg"), 1.0);
    assert_eq!(f64_at(&value, "/aggregate/mrr"), 0.75);
    assert_eq!(f64_at(&value, "/aggregate/ndcg"), 0.815);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_k"), 1.0);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_1"), 0.5);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_5"), 1.0);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_20"), 1.0);
    assert_eq!(value["aggregate"]["n_queries"], 2);
}

// --- search: exact membership and term bytes -----------------------------------

#[test]
fn search_exact_file_membership_and_term_bytes() {
    // Three files hold the term; a fourth holds a token-disjoint symbol, so
    // no weak channel match can pollute the result: at limit 10 the hit set
    // is exactly the three matching files, and every hit excerpt carries the
    // query-term bytes.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for name in ["m0.rs", "m1.rs", "m2.rs"] {
        write_fixture(&root, name, "fn wobble_n4_mem() {}\n");
    }
    write_fixture(&root, "nomatch.rs", "fn plain_unrelated_symbol() {}\n");
    let bin = asgrep_bin();
    let output = run(&bin, &["--json", "--no-embed", "index", root.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "index must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = run_search_json(&root, 10, "wobble_n4_mem");
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["limit"], 10, "{value}");
    let hits = value["hits"].as_array().expect("hits array").clone();
    assert_eq!(hits.len(), 3, "{value}");
    let mut basenames: Vec<&str> = hits
        .iter()
        .map(|h| {
            h["file"]
                .as_str()
                .expect("file string")
                .rsplit('/')
                .next()
                .expect("basename")
        })
        .collect();
    basenames.sort();
    assert_eq!(basenames, ["m0.rs", "m1.rs", "m2.rs"]);
    for hit in &hits {
        let excerpt = hit["excerpt"].as_str().expect("excerpt string");
        assert!(
            excerpt.contains("wobble_n4_mem"),
            "excerpt must carry the term bytes: {excerpt}"
        );
    }
}
