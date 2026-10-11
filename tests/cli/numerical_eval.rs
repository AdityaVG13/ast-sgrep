//! CLI numerical: eval surface contract suite.
//!
//! Canonical per-surface successor of the eval arms of
//! `numerical_pass{1,2,3,4}.rs` per `tests/catalog/numerical-cli.md`: eval
//! metric arithmetic, k-sweeps, degenerate totality, fail-closed gold,
//! adversarial ranks, unrounded-mean aggregates, and human/JSON rendering
//! agreement. Search-limit/determinism arms live in `numerical_search.rs`;
//! flag-cap/window arms live in `numerical_limits.rs`.
//!
//! Absorption map (catalog VERDICTs): 9 KEEP anchors stay standalone;
//! `eval_arithmetic_contract` absorbs 6, `eval_k_sweep_contract` absorbs 2,
//! `eval_degenerate_contract` absorbs 2, `eval_failclosed_contract`,
//! `adversarial_rank_contract`, and `unrounded_mean_contract` absorb 1 each.
//! `eval_hit_scores_no_lower_than_miss` is DELETEd per catalog (tautology
//! risk: exact endpoint tests already pin hit=1/miss=0). 15 tests.

#[path = "numerical_common.rs"]
mod common;

use ast_sgrep_testkit::{
    asgrep_bin, assert_finite_unit, assert_operational_envelope, eval_project, f64_at,
    parse_human_row, parse_human_summary, run, run_eval_ok, run_eval_raw, write_fixture,
    write_gold,
};
use common::rank_key;
use tempfile::TempDir;

#[test]
fn eval_boundary_relevance_uses_complete_path_components() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    write_fixture(&root, "src/target.rs", "fn intended_target() {}\n");
    write_fixture(&root, "src/not_target.rs", "fn filename_decoy() {}\n");
    write_fixture(&root, "prefixsrc/target.rs", "fn directory_decoy() {}\n");
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({
            "corpus":"path-identity", "queries":[
                {"name":"filename", "query":"defs:filename_decoy", "k":5, "relevant":[{"file":"target.rs"}]},
                {"name":"directory", "query":"defs:directory_decoy", "k":5, "relevant":[{"file":"src/target.rs"}]},
                {"name":"basename", "query":"defs:intended_target", "k":5, "relevant":[{"file":"target.rs"}]},
                {"name":"nested", "query":"defs:intended_target", "k":5, "relevant":[{"file":"src/target.rs"}]},
                {"name":"symbol", "query":"defs:intended_target", "k":5, "relevant":[{"file":"target.rs","symbol":"not_the_target"}]}
            ]
        }),
    );
    let report = run_eval_ok(&gold, &root);
    let found: Vec<_> = report["queries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|query| query["found"].as_u64().unwrap())
        .collect();
    assert_eq!(
        found,
        [0, 0, 1, 1, 0],
        "only whole path suffixes and matching symbols are relevant: {report}"
    );
}

#[test]
fn eval_boundary_invalid_ab_refuses_before_index_effects() {
    let (temp, root) = eval_project("admission_target");
    let index = temp.path().join("explicit-index.db");
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({
            "corpus":"admission", "queries":[{"name":"one", "query":"admission_target", "k":1, "relevant":[{"file":"a.rs"}]}]
        }),
    );
    let output = run(
        &asgrep_bin(),
        &[
            "--json",
            "--index-path",
            index.to_str().unwrap(),
            "eval",
            "--gold",
            gold.to_str().unwrap(),
            root.to_str().unwrap(),
            "--ab",
            "not-a-mode",
        ],
    );
    let report = assert_operational_envelope(&output);
    assert!(report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("--ab"));
    assert!(
        !index.exists(),
        "invalid evaluation options must not create or populate the explicit index"
    );
}

#[test]
fn eval_boundary_ambiguous_roots_refuse_before_index_effects() {
    let (temp, root) = eval_project("one_root_target");
    let (_other, positional) = eval_project("different_root_target");
    let index = temp.path().join("explicit-index.db");
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({
            "corpus":"one-root", "queries":[{"name":"one", "query":"one_root_target", "k":1, "relevant":[{"file":"a.rs"}]}]
        }),
    );
    let output = run(
        &asgrep_bin(),
        &[
            "--json",
            "--no-embed",
            "--root",
            root.to_str().unwrap(),
            "--index-path",
            index.to_str().unwrap(),
            "eval",
            "--gold",
            gold.to_str().unwrap(),
            positional.to_str().unwrap(),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "two roots must not silently select one"
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["error"]["kind"], "usage");
    assert!(!index.exists(), "ambiguous roots must not create an index");
}

#[cfg(unix)]
#[test]
fn eval_nonregular_gold_refuses_without_blocking_or_index_effects() {
    let (temp, root) = eval_project("gold_ingress_target");
    let index = temp.path().join("explicit-index.db");
    let gold = temp.path().join("gold.json");
    assert!(std::process::Command::new("mkfifo")
        .arg(&gold)
        .status()
        .unwrap()
        .success());
    let output = ast_sgrep_testkit::run_drain_timeout(
        &asgrep_bin(),
        &[
            "--json",
            "--no-embed",
            "--index-path",
            index.to_str().unwrap(),
            "eval",
            "--gold",
            gold.to_str().unwrap(),
            root.to_str().unwrap(),
        ],
        &[],
        std::time::Duration::from_secs(5),
    );
    assert_operational_envelope(&output);
    assert!(
        !index.exists(),
        "refused gold input must not create an index"
    );
}

#[test]
fn eval_overlapping_gold_is_order_independent_and_one_to_one() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    write_fixture(
        &root,
        "left/a.rs",
        "fn overlap_alpha() {}\nfn overlap_beta() {}\n",
    );
    write_fixture(&root, "right/a.rs", "fn overlap_alpha() {}\n");
    let cases = [
        (
            "symbol-alpha",
            "overlap",
            serde_json::json!([
                {"file":"left/a.rs"}, {"file":"left/a.rs","symbol":"overlap_alpha"}
            ]),
            2,
        ),
        (
            "symbol-beta",
            "overlap",
            serde_json::json!([
                {"file":"left/a.rs"}, {"file":"left/a.rs","symbol":"overlap_beta"}
            ]),
            2,
        ),
        (
            "path",
            "defs:overlap_alpha",
            serde_json::json!([
                {"file":"a.rs"}, {"file":"left/a.rs"}
            ]),
            2,
        ),
        // These match sets cross: neither criterion contains the other.
        (
            "crossing",
            "defs:overlap_alpha",
            serde_json::json!([
                {"file":"left/a.rs"}, {"file":"a.rs","symbol":"overlap_alpha"}
            ]),
            2,
        ),
        (
            "single-hit",
            "defs:overlap_beta",
            serde_json::json!([
                {"file":"left/a.rs"}, {"file":"left/a.rs","symbol":"overlap_beta"}
            ]),
            1,
        ),
    ];
    let mut queries = Vec::new();
    let mut expected_found = Vec::new();
    for (name, query, relevant, complete_found) in cases {
        for k in [1, 5] {
            for reverse in [false, true] {
                let mut rows = relevant.as_array().unwrap().clone();
                if reverse {
                    rows.reverse();
                }
                queries.push(serde_json::json!({"name":name,"query":query,"k":k,"relevant":rows}));
            }
            expected_found.push(if k == 1 { 1 } else { complete_found });
        }
    }
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus":"overlapping-evidence","queries":queries}),
    );
    let report = run_eval_ok(&gold, &root);
    for (pair, found) in report["queries"]
        .as_array()
        .unwrap()
        .chunks_exact(2)
        .zip(expected_found)
    {
        assert_eq!(
            pair[0], pair[1],
            "gold order cannot change retrieval metrics: {report}"
        );
        assert_eq!(
            pair[0]["found"], found,
            "maximize distinct evidence without counting a hit twice"
        );
    }
}

/// INTENT: Hit+miss+2-of-4 corpus pins hand-computed fractional aggregates
/// (MRR=nDCG=0.667, recall@k=0.5, recall@1=0.417).
/// KILLS: aggregation-mean-error / wrong-denominator.
/// ABSORBS: none (KEEP anchor).
#[test]
fn eval_three_query_fractional_aggregates() {
    // q_hit: term in one file, k=5 -> rank 1, RR=1, nDCG=1, recall=1.
    // q_miss: gibberish matches nothing -> rank null, RR=0, nDCG=0, recall=0.
    // q_part: term in 4 files, k=2 -> found=2 however ordered (both scanned
    //   hits relevant), RR=1, DCG=IDCG(min(4,2)=2) so nDCG=1,
    //   recall@1=1/4=0.25, fixed recall@5/20=4/4=1 despite k=2.
    // Aggregates over n=3 (means of unrounded values, then round3):
    //   MRR=(1+0+1)/3=0.6666..->0.667; nDCG likewise 0.667;
    //   recall@k=(1+0+0.5)/3=0.5; recall@1=(1+0+0.25)/3=0.4166..->0.417;
    //   recall@5=recall@20=(1+0+1)/3=0.667.
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
    assert_eq!(f64_at(&value, "/queries/2/recall_at/5"), 1.0);
    assert_eq!(f64_at(&value, "/queries/2/recall_at/20"), 1.0);
    assert_eq!(f64_at(&value, "/aggregate/mrr"), 0.667);
    assert_eq!(f64_at(&value, "/aggregate/ndcg"), 0.667);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_k"), 0.5);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_1"), 0.417);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_5"), 0.667);
    assert_eq!(f64_at(&value, "/aggregate/recall_at_20"), 0.667);
    assert_eq!(value["aggregate"]["n_queries"], 3);
}

/// INTENT: k=1..5 sweep over 4 relevant pins found 1,2,3,4,4 and recall
/// 0.25-1.0 with nDCG=RR=1 throughout.
/// KILLS: found/recall-off-by-one across k.
/// ABSORBS: none (KEEP anchor).
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
    let relevant = serde_json::json!([{"file": "w1.rs"}, {"file": "w2.rs"}, {"file": "w3.rs"}, {"file": "w4.rs"}]);
    for (k, expected_found, expected_recall) in [
        (1u64, 1u64, 0.25),
        (2, 2, 0.5),
        (3, 3, 0.75),
        (4, 4, 1.0),
        (5, 4, 1.0),
    ] {
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

/// Fixed cutoffs must retrieve enough results even with --limit=1 and k=1.
/// A/B and single-run rendering must use the same cutoff semantics.
#[test]
fn eval_fixed_cutoffs_ignore_primary_k_and_cli_limit() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    let relevant: Vec<_> = (0..25)
        .map(|n| {
            let file = format!("item{n:02}.rs");
            write_fixture(&root, &file, "fn cutoff_probe() {}\n");
            serde_json::json!({"file": file})
        })
        .collect();
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "cutoffs", "queries": [
            {"name": "fixed", "query": "defs:cutoff_probe", "k": 1, "relevant": relevant}
        ]}),
    );
    let bin = asgrep_bin();
    for ab in [false, true] {
        let mut args = vec![
            "--json",
            "--limit",
            "1",
            "eval",
            "--gold",
            gold.to_str().unwrap(),
            root.to_str().unwrap(),
        ];
        if ab {
            args.extend(["--ab", "no-embed"]);
        }
        let output = run(&bin, &args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let reports = if ab {
            vec![&value["a"], &value["b"]]
        } else {
            vec![&value]
        };
        for report in reports {
            assert_eq!(report["queries"][0]["found"], 1);
            assert_eq!(f64_at(report, "/aggregate/recall_at_k"), 0.04);
            assert_eq!(f64_at(report, "/aggregate/recall_at_1"), 0.04);
            assert_eq!(f64_at(report, "/aggregate/recall_at_5"), 0.2);
            assert_eq!(f64_at(report, "/aggregate/recall_at_20"), 0.8);
        }
    }
}

/// Run the evaluation pack's payload-measurement block against real Unicode
/// output. This catches character-count and whitespace-stripping regressions
/// in every format, including the budgeted arm, without building a fake CLI.
#[test]
fn eval_pack_counts_complete_stdout_bytes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(
        &root,
        "café.rs",
        "fn payload_probe() { let _ = \"你好 🦀\"; }\n",
    );
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({
            "queries": [{"name": "unicode", "query": "defs:payload_probe"}]
        }),
    );
    let bin = asgrep_bin();
    let index = temp.path().join("index.db");
    let indexed = run(
        &bin,
        &[
            "--index-path",
            index.to_str().unwrap(),
            "index",
            root.to_str().unwrap(),
        ],
    );
    assert!(
        indexed.status.success(),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    let pack = include_str!("../../benchmarks/run_eval.sh");
    let marker = "python3 - \"$bin\" \"$gold\" \"$out\" \"$token_index\" <<'PY'\n";
    let block = pack
        .split_once(marker)
        .unwrap()
        .1
        .split_once("\nPY\n")
        .unwrap()
        .0;
    let output = std::process::Command::new("python3")
        .args(["-c", block])
        .arg(&bin)
        .arg(&gold)
        .arg(temp.path())
        .arg(&index)
        .current_dir(&root)
        .env("NO_COLOR", "1")
        .env("HF_HUB_OFFLINE", "1")
        .output()
        .expect("run measurement block");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(temp.path().join("self-token-efficiency.json")).unwrap(),
    )
    .unwrap();
    for fmt in [
        "native",
        "agent",
        "agent-capsule",
        "compact",
        "compact-budget300",
    ] {
        let mut args = vec![
            "--index-path",
            index.to_str().unwrap(),
            "--json",
            "--format",
        ];
        if fmt == "compact-budget300" {
            args.extend(["compact", "--budget-tokens", "300"]);
        } else {
            args.push(fmt);
        }
        args.extend(["--limit", "10", "defs:payload_probe", "."]);
        let actual = std::process::Command::new(&bin)
            .args(args)
            .current_dir(&root)
            .env("NO_COLOR", "1")
            .env("HF_HUB_OFFLINE", "1")
            .output()
            .unwrap();
        assert!(actual.status.success());
        assert_eq!(
            report["per_query"][0][fmt].as_u64().unwrap(),
            actual.stdout.len() as u64,
            "{fmt}"
        );
    }
}

/// Repetition in one noise file must not consume a distinct-file discovery
/// budget and hide another file's implementation. Exercise both sides of the
/// literal scan's small/large-corpus threshold without changing the gold.
#[test]
fn eval_repeated_lines_do_not_hide_implementation_files() {
    let temp = TempDir::new().unwrap();
    let bin = asgrep_bin();
    for repetitions in [1, 150, 1100] {
        let root = temp.path().join(repetitions.to_string());
        std::fs::create_dir_all(&root).unwrap();
        write_fixture(&root, "a_noise.md", &"quartz notes\n".repeat(repetitions));
        write_fixture(&root, "z_impl.rs", "pub fn quartz_resolve() {}\n");
        let gold = write_gold(
            temp.path(),
            &format!("gold-{repetitions}.json"),
            &serde_json::json!({
                "corpus": "file-admission", "queries": [{"name": "implementation",
                    "query": "quartz resolution", "k": 20, "relevant": [{"file": "z_impl.rs"}]}]
            }),
        );
        let output = run(
            &bin,
            &[
                "--json",
                "eval",
                "--gold",
                gold.to_str().unwrap(),
                root.to_str().unwrap(),
                "--ab",
                "no-embed",
            ],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        for arm in ["a", "b"] {
            assert_eq!(
                f64_at(&report[arm], "/aggregate/recall_at_20"),
                1.0,
                "{repetitions} repeated lines, arm {arm}"
            );
        }
    }
}

/// INTENT: Path-ordered decoy forces rank 2 with RR=0.5/nDCG=0.631.
/// KILLS: rank-order-bug + DCG-log-error.
/// ABSORBS: none (KEEP anchor).
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

/// INTENT: Rank-2+rank-1 mix pins unrounded-mean nDCG=0.815, not the 0.816 a
/// mean of rounded per-query values would give.
/// KILLS: round-then-mean-order (mean of rounded values).
/// ABSORBS: none (KEEP anchor).
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

/// INTENT: Human eval table renders {:.3} in per-query row and MRR summary
/// line (2/3 prints 0.667, 1/3 prints 0.333).
/// KILLS: format-precision-mutant ({:.2} / unrounded).
/// ABSORBS: none (KEEP anchor).
#[test]
fn eval_human_table_rounds_to_three_decimals() {
    // Same 2-of-3 shape as the arithmetic contract's trio arm, human
    // (non-JSON) rendering: the per-query row and the MRR summary line use
    // {:.3}, so 2/3 prints 0.667 and 1/3 prints 0.333.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for name in ["h1.rs", "h2.rs", "h3.rs"] {
        write_fixture(&root, name, "fn wobblebuild_human() {}\n");
    }
    let gold = write_gold(
        temp.path(),
        "gold.json",
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
             Recall@1=0.333  Recall@5=1.000  Recall@20=1.000  n=1"
        ),
        "summary line must show {{:.3}} aggregates: {stdout}"
    );
}

/// INTENT: Human and JSON renderings agree within 0.001; counts/rank exact.
/// KILLS: rendering-divergence (separate formulas drift).
/// ABSORBS: none (KEEP anchor).
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
    let json_value = run_eval_ok(&gold, &root);
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
        assert_eq!(
            found,
            q["found"].as_u64().unwrap(),
            "{name}: found must agree exactly"
        );
        assert_eq!(
            relevant,
            q["relevant"].as_u64().unwrap(),
            "{name}: relevant must agree exactly"
        );
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

/// INTENT: recall@1<=recall@5<=recall@20 per query and in aggregate.
/// KILLS: cutoff-clamp-bug (narrow scan exceeds wide).
/// ABSORBS: none (KEEP anchor).
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
    let value = run_eval_ok(&gold, &root);
    for qi in [0, 1] {
        let r1 = f64_at(&value, &format!("/queries/{qi}/recall_at/1"));
        let r5 = f64_at(&value, &format!("/queries/{qi}/recall_at/5"));
        let r20 = f64_at(&value, &format!("/queries/{qi}/recall_at/20"));
        assert!(
            r1 <= r5 && r5 <= r20,
            "query {qi} cutoffs must be monotone: {r1} <= {r5} <= {r20}"
        );
    }
    let a1 = f64_at(&value, "/aggregate/recall_at_1");
    let a5 = f64_at(&value, "/aggregate/recall_at_5");
    let a20 = f64_at(&value, "/aggregate/recall_at_20");
    assert!(
        a1 <= a5 && a5 <= a20,
        "aggregate cutoffs must be monotone: {a1} <= {a5} <= {a20}"
    );
}

/// INTENT: k=0 zeroes primary metrics, not independent fixed cutoffs;
/// all floats remain JSON numbers (a NaN leak would render as null).
/// KILLS: NaN-leak (0/0 renders null).
/// ABSORBS: none (KEEP anchor).
#[test]
fn eval_k_zero_yields_exact_zeros() {
    // k=0: scan takes no hits (found=0, first_rank=null) and idcg(min(1,0))
    // is 0, forcing nDCG 0. Every float must be a JSON number: a NaN leak
    // would render as null and fail as_f64.
    let (temp, root) = eval_project("wobblebuild_k0");
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n2", "queries": [
            {"name": "k0", "query": "wobblebuild_k0", "k": 0,
             "relevant": [{"file": "a.rs"}]}
        ]}),
    );
    let value = run_eval_ok(&gold, &root);
    let q = &value["queries"][0];
    assert!(q["first_rank"].is_null(), "{value}");
    assert_eq!(q["found"], 0, "{value}");
    assert_eq!(q["relevant"], 1, "{value}");
    assert_eq!(assert_finite_unit(&value, "/queries/0/rr"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/queries/0/ndcg"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/queries/0/recall_at/1"), 1.0);
    assert_eq!(assert_finite_unit(&value, "/queries/0/recall_at/5"), 1.0);
    assert_eq!(assert_finite_unit(&value, "/queries/0/recall_at/20"), 1.0);
    assert_eq!(assert_finite_unit(&value, "/aggregate/mrr"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/aggregate/ndcg"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/aggregate/recall_at_k"), 0.0);
    assert_eq!(value["aggregate"]["n_queries"], 1, "{value}");
}

/// INTENT: Empty-queries / non-usize-k gold fails closed exit-2 operational,
/// never ok:true with fabricated zeros.
/// KILLS: fail-open (fabricated zeros with ok:true).
/// ABSORBS: none (KEEP anchor).
#[test]
fn eval_degenerate_gold_fails_closed() {
    // Zero queries, or a non-usize k (1.5), is an operational failure (exit
    // 2), never ok:true with fabricated zeros.
    let (temp, root) = eval_project("wobblebuild_badgold");
    let empty = write_gold(
        temp.path(),
        "empty.json",
        &serde_json::json!({"corpus": "n2", "queries": []}),
    );
    let output = run_eval_raw(&empty, &root);
    let value = assert_operational_envelope(&output);
    assert_eq!(value["command"], "eval", "{value}");
    let float_k = write_gold(
        temp.path(),
        "floatk.json",
        &serde_json::json!({"corpus": "n2", "queries": [
            {"name": "f", "query": "x", "k": 1.5, "relevant": []}
        ]}),
    );
    let output = run_eval_raw(&float_k, &root);
    let value = assert_operational_envelope(&output);
    assert_eq!(value["command"], "eval", "{value}");
}

/// INTENT: One contract for eval metric arithmetic: exact-zero miss and
/// exact-one rank-1 endpoints, 2-of-3 and 5-of-6 round3 recalls, hit+miss
/// mean halving, and black-box round3 fixed-point idempotence.
/// KILLS: zero-guard-omission, reciprocal/log-base-error, rounding-omission,
/// aggregation-mean-error (wrong divisor).
/// ABSORBS: eval_total_miss_yields_exact_zeros, eval_single_rank1_hit_yields_exact_ones,
/// eval_two_of_three_rounds_recall_to_667_and_333, eval_mixed_hit_miss_halves_mrr_and_ndcg,
/// eval_reported_floats_are_round3_fixed_points, eval_five_of_six_rounds_recall_to_833_and_167.
#[test]
fn eval_arithmetic_contract() {
    // Arm 1 (absorbed: eval_total_miss_yields_exact_zeros). Query term
    // appears in NO file, so first_rank=null, RR=0, DCG=0, nDCG=0/IDCG(1)=0,
    // recall=0/1=0 at every cutoff.
    {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        write_fixture(&root, "present.rs", "fn wobblebuild_here() {}\n");
        let gold = write_gold(
            temp.path(),
            "gold.json",
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
    // Arm 2 (absorbed: eval_single_rank1_hit_yields_exact_ones). One file
    // holds the term (once), so the first hit is rank 1: RR=1/1=1;
    // DCG=1/log2(2)=1; IDCG(min(1,5)=1)=1; nDCG=1; recall=1.
    {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        write_fixture(&root, "solo.rs", "fn wobblebuild_solo() {}\n");
        let gold = write_gold(
            temp.path(),
            "gold.json",
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
    // Arm 3 (absorbed: eval_two_of_three_rounds_recall_to_667_and_333).
    // Three relevant files share the term; k=2 scans ranks 1-2, both
    // relevant however ordered: found=2, RR=1.
    // DCG = 1/log2(2)+1/log2(3) = 1.63092975... = IDCG(min(3,2)=2),
    // so nDCG is exactly 1. recall@1 = 1/3 -> round3 -> 0.333;
    // recall@k = 2/3 -> 0.667, while fixed recall@5/20 = 3/3 = 1.
    {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        for name in ["t1.rs", "t2.rs", "t3.rs"] {
            write_fixture(&root, name, "fn wobblebuild_trio() {}\n");
        }
        let gold = write_gold(
            temp.path(),
            "gold.json",
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
        assert_eq!(f64_at(&value, "/queries/0/recall_at/5"), 1.0);
        assert_eq!(f64_at(&value, "/queries/0/recall_at/20"), 1.0);
        assert_eq!(f64_at(&value, "/aggregate/mrr"), 1.0);
        assert_eq!(f64_at(&value, "/aggregate/ndcg"), 1.0);
        assert_eq!(f64_at(&value, "/aggregate/recall_at_k"), 0.667);
    }
    // Arm 4 (absorbed: eval_mixed_hit_miss_halves_mrr_and_ndcg). q_hit:
    // RR=1, nDCG=1. q_miss: gibberish matches nothing: RR=0, nDCG=0.
    // Aggregate over n=2: MRR=(1+0)/2=0.5, nDCG=0.5, recall@k=(1+0)/2=0.5.
    {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        write_fixture(&root, "hit.rs", "fn wobblebuild_mixed() {}\n");
        write_fixture(&root, "other.rs", "fn wobblebuild_unrelated() {}\n");
        let gold = write_gold(
            temp.path(),
            "gold.json",
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
    // Arm 5 (absorbed: eval_reported_floats_are_round3_fixed_points).
    // Black-box round3 idempotence: every reported float v must satisfy
    // round3(v) == v, i.e. v * 1000 is integral. The 2-of-3 shape forces
    // repeating decimals (0.333/0.667) through the rounding path.
    {
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
        let value = run_eval_ok(&gold, &root);
        let mut checked = 0usize;
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
            let v = f64_at(&value, pointer);
            let scaled = v * 1000.0;
            assert!(
                (scaled - scaled.round()).abs() <= 1e-6,
                "{pointer}={v} is not a round3 fixed point"
            );
            checked += 1;
        }
        assert_eq!(checked, 11);
    }
    // Arm 6 (absorbed: eval_five_of_six_rounds_recall_to_833_and_167). Six
    // relevant files share the term; k=5 scans ranks 1-5, all relevant
    // however ordered: found=5, RR=1, DCG(ranks 1-5) = IDCG(min(6,5)=5) so
    // nDCG=1. recall@1 = 1/6 -> 0.167; recall@5 = 5/6 -> 0.833;
    // fixed recall@20 includes the sixth result and equals 1.
    {
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
        assert_eq!(f64_at(&value, "/queries/0/recall_at/20"), 1.0);
        assert_eq!(f64_at(&value, "/aggregate/recall_at_k"), 0.833);
    }
}

/// INTENT: One contract for eval k-truncation: k=1 halves recall while
/// IDCG(min(relevant,k)) keeps nDCG=1, and growing k never lowers
/// found/recall nor worsens rank.
/// KILLS: IDCG-clamp-omission (IDCG over all relevant), cutoff-inversion.
/// ABSORBS: eval_k1_truncation_halves_recall_keeps_ndcg_one,
/// eval_k_growth_never_lowers_found_or_recall.
#[test]
fn eval_k_sweep_contract() {
    // Arm 1 (absorbed: eval_k1_truncation_halves_recall_keeps_ndcg_one). Two
    // relevant files share the term; k=1 scans only the rank-1 hit. found=1
    // regardless of which file ranks first: recall@k=recall@1=1/2=0.5,
    // fixed recall@5/20=1, while DCG=1/log2(2)=1 and
    // IDCG(min(2,1)=1)=1 give nDCG=1.
    {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        write_fixture(&root, "ka.rs", "fn wobblebuild_kk() {}\n");
        write_fixture(&root, "kb.rs", "fn wobblebuild_kk() {}\n");
        let gold = write_gold(
            temp.path(),
            "gold.json",
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
        assert_eq!(f64_at(&value, "/queries/0/recall_at/5"), 1.0);
        assert_eq!(f64_at(&value, "/queries/0/recall_at/20"), 1.0);
        assert_eq!(f64_at(&value, "/aggregate/mrr"), 1.0);
        assert_eq!(f64_at(&value, "/aggregate/recall_at_k"), 0.5);
        assert_eq!(f64_at(&value, "/aggregate/recall_at_1"), 0.5);
    }
    // Arm 2 (absorbed: eval_k_growth_never_lowers_found_or_recall). Three
    // relevant files share one term. Sweeping k upward: found and
    // recall_at_k are non-decreasing, and first_rank never worsens.
    {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        for name in ["k1.rs", "k2.rs", "k3.rs"] {
            write_fixture(&root, name, "fn wobblebuild_n3k() {}\n");
        }
        let relevant = serde_json::json!([{"file": "k1.rs"}, {"file": "k2.rs"}, {"file": "k3.rs"}]);
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
            let value = run_eval_ok(&gold, &root);
            let found = value["queries"][0]["found"].as_u64().expect("found");
            let recall = f64_at(&value, "/aggregate/recall_at_k");
            let rank = rank_key(&value["queries"][0]["first_rank"]);
            assert!(
                found >= prev_found,
                "found must not shrink with k: {prev_found} -> {found}"
            );
            assert!(
                recall >= prev_recall,
                "recall_at_k must not shrink with k: {prev_recall} -> {recall}"
            );
            assert!(
                rank <= prev_rank,
                "first_rank must not worsen with k: {prev_rank:?} -> {rank:?}"
            );
            prev_found = found;
            prev_recall = recall;
            prev_rank = rank;
        }
        assert_eq!(prev_found, 3, "k=5 must find all three relevant files");
    }
}

/// INTENT: One contract for eval degenerate totality: relevant=[] yields
/// exact zeros via 0/0 guards, and k=usize::MAX stays finite with the rank-1
/// hit found.
/// KILLS: div-by-zero (unguarded recall_of), limit-derivation-overflow
/// (unclamped huge k wraps/panics).
/// ABSORBS: eval_empty_relevant_yields_exact_zeros, eval_usize_max_k_stays_finite.
#[test]
fn eval_degenerate_contract() {
    // Arm 1 (absorbed: eval_empty_relevant_yields_exact_zeros).
    // relevant=[]: recall_of guards 0/0 -> 0, idcg(0)==0 forces nDCG 0.
    {
        let (temp, root) = eval_project("wobblebuild_norel");
        let gold = write_gold(
            temp.path(),
            "gold.json",
            &serde_json::json!({"corpus": "n2", "queries": [
                {"name": "norel", "query": "wobblebuild_norel", "k": 5, "relevant": []}
            ]}),
        );
        let value = run_eval_ok(&gold, &root);
        let q = &value["queries"][0];
        assert!(q["first_rank"].is_null(), "{value}");
        assert_eq!(q["found"], 0, "{value}");
        assert_eq!(q["relevant"], 0, "{value}");
        assert_eq!(assert_finite_unit(&value, "/queries/0/rr"), 0.0);
        assert_eq!(assert_finite_unit(&value, "/queries/0/ndcg"), 0.0);
        assert_eq!(assert_finite_unit(&value, "/queries/0/recall_at/5"), 0.0);
        assert_eq!(assert_finite_unit(&value, "/aggregate/mrr"), 0.0);
        assert_eq!(assert_finite_unit(&value, "/aggregate/ndcg"), 0.0);
        assert_eq!(assert_finite_unit(&value, "/aggregate/recall_at_k"), 0.0);
    }
    // Arm 2 (absorbed: eval_usize_max_k_stays_finite). k == usize::MAX: the
    // derived searcher limit is clamped downstream and the scan is bounded
    // by take(cutoff); metrics stay finite in [0,1] and the rank-1 hit is
    // still found.
    {
        let (temp, root) = eval_project("wobblebuild_bigk");
        let gold = write_gold(
            temp.path(),
            "gold.json",
            &serde_json::json!({"corpus": "n2", "queries": [
                {"name": "big", "query": "wobblebuild_bigk", "k": 18446744073709551615u64,
                 "relevant": [{"file": "a.rs"}]}
            ]}),
        );
        let value = run_eval_ok(&gold, &root);
        assert_eq!(value["queries"][0]["first_rank"], 1, "{value}");
        assert_eq!(assert_finite_unit(&value, "/queries/0/rr"), 1.0);
        assert_eq!(assert_finite_unit(&value, "/queries/0/ndcg"), 1.0);
        assert_eq!(assert_finite_unit(&value, "/queries/0/recall_at/20"), 1.0);
        assert_eq!(assert_finite_unit(&value, "/aggregate/mrr"), 1.0);
        assert_eq!(assert_finite_unit(&value, "/aggregate/ndcg"), 1.0);
        assert_eq!(assert_finite_unit(&value, "/aggregate/recall_at_k"), 1.0);
    }
}

/// INTENT: eval on an empty corpus fails closed exit-2 operational, never
/// all-zero MRR.
/// KILLS: fail-open (fabricated zeros with ok:true).
/// ABSORBS: eval_empty_corpus_fails_closed.
#[test]
fn eval_failclosed_contract() {
    // Indexing an empty directory then reporting all-zero MRR/recall would
    // fabricate a quality measurement: fail closed (exit 2).
    let temp = TempDir::new().unwrap();
    let empty = temp.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n2", "queries": [
            {"name": "solo", "query": "wobblebuild_probe", "k": 5,
             "relevant": [{"file": "a.rs"}]}
        ]}),
    );
    let output = run_eval_raw(&gold, &empty);
    let value = assert_operational_envelope(&output);
    assert_eq!(value["command"], "eval", "{value}");
}

/// INTENT: Two path-ordered decoys force rank 3 with RR=0.333/nDCG=0.5
/// exactly.
/// KILLS: RR-reciprocal-error, rank-order-bug.
/// ABSORBS: eval_two_filename_ordered_decoys_force_rank_three.
#[test]
fn adversarial_rank_contract() {
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

/// INTENT: Aggregates sit in the per-query [min,max] band (+/-0.001 rounding
/// slack) with recall@k exactly round3(mean(found/relevant)).
/// KILLS: aggregation-mean-error.
/// ABSORBS: eval_aggregates_stay_within_per_query_band.
#[test]
fn unrounded_mean_contract() {
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
    let value = run_eval_ok(&gold, &root);
    let per_query = |pointer: &str| -> Vec<f64> {
        (0..2)
            .map(|qi| f64_at(&value, &format!("/queries/{qi}{pointer}")))
            .collect()
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
