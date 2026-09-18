//! Pass 4 (oracle-foundry, Mission 3): L4 end-to-end oracles driving the
//! REAL `asgrep` binary on tempdir fixture trees: index then search, the
//! `search`-channel faces pass 3 did not cover, and the exit-code contract.
//!
//! Non-overlap: pass 1/2 own supervisor arithmetic, UTF-16 offsets,
//! identifier extraction, line lookup, symbol kinds, file URIs, and text
//! edits. Pass 3 owns `keyword`-channel repetition, `keyword` JSON-vs-human
//! agreement, `keyword` files-with-matches faces, limit-prefix stability,
//! file-filter monotonicity, CLI-vs-library differential, outline
//! JSON-vs-human agreement, and adversarial inputs. `machine_contracts.rs`
//! owns machine-envelope shapes and message-text failure assertions;
//! `outline_cmd.rs` owns all-function outline faces; `files_with_matches.rs`
//! owns `keyword` files-with-matches faces; `cli_smoke.rs` owns unindexed
//! empty-checkout refusal. This file covers relations across runs and
//! commands instead:
//!
//! - `index` / `status` hand-computed counts on a tempdir tree.
//! - `search`-channel: hit shape, human-vs-JSON agreement, repetition
//!   byte-stability, files-with-matches consistency with hits.
//! - Exit-code contract: 0 for hits AND no-hits (both channels, both
//!   formats), 2 with `ok:false` for missing roots / empty indexes;
//!   discriminants only, never message text.
//! - `reindex` picks up an added file; `chain` edges follow the hand call
//!   graph; `outline` mixed-kind (type + function) spans.
//!
//! Expectations are hand-computed from the fixture sources below. Assertions
//! are discriminants (exit codes, `ok` booleans, counts, key sets, sorted
//! file lists) — never message text.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

struct Corpus {
    _temp: TempDir,
    root: PathBuf,
    index: PathBuf,
}

fn write_bytes(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("mkdirs");
    }
    fs::write(&path, bytes).expect("write corpus file");
}

fn run_raw(args: &[&str]) -> (i32, Vec<u8>, String) {
    let output = Command::new(asgrep_bin())
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .expect("run asgrep");
    let code = output.status.code().expect("exit code");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    (code, output.stdout, stderr)
}

fn run_json(args: &[&str]) -> (i32, Value, Vec<u8>, String) {
    let (code, stdout, stderr) = run_raw(args);
    let value: Value = serde_json::from_slice(&stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not JSON: {error}\nstdout: {}\nstderr: {stderr}",
            String::from_utf8_lossy(&stdout)
        )
    });
    (code, value, stdout, stderr)
}

/// Hand corpus: a.rs defines `pass4_alpha` (line 1) and `pass4_beta`
/// (line 2, calls alpha); b.rs defines isolated `pass4_gamma` (line 1).
/// Hand-computed: 2 files, 3 symbols, 1 caller edge (beta -> alpha).
fn search_corpus() -> Corpus {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("corpus");
    write_bytes(
        &root,
        "a.rs",
        b"fn pass4_alpha() {}\nfn pass4_beta() { pass4_alpha(); }\n",
    );
    write_bytes(&root, "b.rs", b"fn pass4_gamma() {}\n");
    let index = temp.path().join("index.db");
    let (code, value, _, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "index",
        root.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "index must succeed: {stderr}");
    assert_eq!(value["ok"], true);
    Corpus {
        _temp: temp,
        root,
        index,
    }
}

fn search_json(corpus: &Corpus, channel: &str, query: &str, extra: &[&str]) -> (i32, Value, Vec<u8>, String) {
    let mut owned: Vec<String> = vec![
        "--index-path".into(),
        corpus.index.to_str().unwrap().into(),
        "--no-embed".into(),
        "--no-auto-index".into(),
        "--json".into(),
        "--limit".into(),
        "50".into(),
    ];
    owned.extend(extra.iter().map(|s| s.to_string()));
    owned.push(channel.into());
    owned.push(query.into());
    owned.push(corpus.root.to_str().unwrap().into());
    let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
    run_json(&refs)
}

// ---------------------------------------------------------------------------
// index then search: hand-computed counts and hit shape
// ---------------------------------------------------------------------------

#[test]
fn index_envelope_counts_match_hand_fixture() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("corpus");
    write_bytes(
        &root,
        "a.rs",
        b"fn pass4_alpha() {}\nfn pass4_beta() { pass4_alpha(); }\n",
    );
    write_bytes(&root, "b.rs", b"fn pass4_gamma() {}\n");
    let index = temp.path().join("index.db");
    let (code, value, _, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "index",
        root.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["command"], "index");
    assert_eq!(value["ok"], true);
    assert_eq!(value["exit_code"], 0);
    // Hand-computed from the two fixture files above.
    assert_eq!(value["files_indexed"], 2);
    assert_eq!(value["files_failed"], 0);
    assert_eq!(value["symbols_extracted"], 3);
}

#[test]
fn search_finds_hand_placed_symbol_first() {
    let corpus = search_corpus();
    let (code, value, _, stderr) = search_json(&corpus, "search", "pass4_alpha", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["command"], "search");
    assert_eq!(value["ok"], true);
    assert_eq!(value["exit_code"], 0);
    assert_eq!(value["query"], "pass4_alpha");
    let hits = value["hits"].as_array().expect("hits array");
    assert!(!hits.is_empty(), "hand-placed symbol must hit");
    // Exact definition hit first: symbol, file, and one-line span.
    assert_eq!(hits[0]["symbol"], "pass4_alpha");
    assert_eq!(hits[0]["file"], "a.rs");
    assert_eq!(hits[0]["line_start"], 1);
    assert_eq!(hits[0]["line_end"], 1);
    assert!(hits.iter().all(|h| h["symbol"].is_string() && h["file"].is_string()));
}

#[test]
fn status_counts_match_hand_fixture() {
    let corpus = search_corpus();
    let (code, value, _, stderr) = run_json(&[
        "--index-path",
        corpus.index.to_str().unwrap(),
        "--json",
        "status",
        corpus.root.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["command"], "status");
    assert_eq!(value["ok"], true);
    assert_eq!(value["exit_code"], 0);
    // Hand-computed: 2 files, 3 symbols, 1 caller edge.
    assert_eq!(value["file_count"], 2);
    assert_eq!(value["symbol_count"], 3);
    assert_eq!(value["caller_count"], 1);
}

// ---------------------------------------------------------------------------
// Exit codes: hits vs no-hits vs errors
// ---------------------------------------------------------------------------

#[test]
fn no_hits_is_ok_empty_exit_zero_both_channels() {
    let corpus = search_corpus();
    for channel in ["search", "keyword"] {
        let (code, value, _, stderr) = search_json(&corpus, channel, "zzz_no_such_symbol_zzz", &[]);
        assert_eq!(code, 0, "channel={channel} stderr={stderr}");
        assert_eq!(value["ok"], true, "channel={channel}");
        assert_eq!(value["exit_code"], 0, "channel={channel}");
        assert_eq!(
            value["hits"].as_array().expect("hits").len(),
            0,
            "channel={channel}"
        );

        // Human face: no-hits is also exit 0 with empty stdout.
        let (hcode, hstdout, hstderr) = run_raw(&[
            "--index-path",
            corpus.index.to_str().unwrap(),
            "--no-embed",
            "--no-auto-index",
            "--limit",
            "50",
            channel,
            "zzz_no_such_symbol_zzz",
            corpus.root.to_str().unwrap(),
        ]);
        assert_eq!(hcode, 0, "channel={channel} stderr={hstderr}");
        assert!(hstdout.is_empty(), "channel={channel}");
        assert!(hstderr.is_empty(), "channel={channel}");
    }
}

#[test]
fn missing_root_fails_closed_across_commands() {
    let corpus = search_corpus();
    let missing = corpus._temp.path().join("does-not-exist");
    let missing = missing.to_str().unwrap();

    // `index` on a missing root: operational failure, exit 2.
    let (icode, ivalue, _, _) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        corpus._temp.path().join("other.db").to_str().unwrap(),
        "index",
        missing,
    ]);
    assert_eq!(icode, 2);
    assert_eq!(ivalue["ok"], false);
    assert_eq!(ivalue["exit_code"], 2);
    assert_eq!(ivalue["error"]["kind"], "operational");

    // `search` on a missing root: same fail-closed discriminant.
    let (scode, svalue, _, _) = run_json(&[
        "--index-path",
        corpus.index.to_str().unwrap(),
        "--no-embed",
        "--no-auto-index",
        "--json",
        "search",
        "pass4_alpha",
        missing,
    ]);
    assert_eq!(scode, 2);
    assert_eq!(svalue["ok"], false);
    assert_eq!(svalue["exit_code"], 2);
    assert_eq!(svalue["error"]["kind"], "operational");
}

#[test]
fn empty_tree_index_ok_then_search_fails_closed() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("empty");
    fs::create_dir(&root).expect("empty dir");
    let index = temp.path().join("index.db");

    // Indexing an empty tree succeeds with zero counts.
    let (icode, ivalue, _, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "index",
        root.to_str().unwrap(),
    ]);
    assert_eq!(icode, 0, "stderr={stderr}");
    assert_eq!(ivalue["ok"], true);
    assert_eq!(ivalue["files_indexed"], 0);
    assert_eq!(ivalue["symbols_extracted"], 0);
    assert_eq!(ivalue["files_failed"], 0);

    // Searching that empty index fails closed: exit 2, ok false.
    let (scode, svalue, _, _) = run_json(&[
        "--index-path",
        index.to_str().unwrap(),
        "--no-embed",
        "--no-auto-index",
        "--json",
        "search",
        "anything",
        root.to_str().unwrap(),
    ]);
    assert_eq!(scode, 2);
    assert_eq!(svalue["ok"], false);
    assert_eq!(svalue["exit_code"], 2);
    assert_eq!(svalue["error"]["kind"], "operational");
}

// ---------------------------------------------------------------------------
// search-channel relations: human agreement, determinism, files-with-matches
// ---------------------------------------------------------------------------

#[test]
fn search_human_rows_equal_json_hits() {
    let corpus = search_corpus();
    let (code, value, _, stderr) = search_json(&corpus, "search", "pass4_alpha", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    let json_hits = value["hits"].as_array().expect("hits array").len();
    assert!(json_hits >= 1, "corpus must yield hits");

    let (hcode, hstdout, hstderr) = run_raw(&[
        "--index-path",
        corpus.index.to_str().unwrap(),
        "--no-embed",
        "--no-auto-index",
        "--limit",
        "50",
        "search",
        "pass4_alpha",
        corpus.root.to_str().unwrap(),
    ]);
    assert_eq!(hcode, 0, "stderr={hstderr}");
    assert!(hstderr.is_empty(), "clean run stays silent: {hstderr}");
    let human = String::from_utf8_lossy(&hstdout);
    assert_eq!(
        human.lines().count(),
        json_hits,
        "human rows must equal JSON hits"
    );
}

#[test]
fn search_repetition_is_byte_identical() {
    let corpus = search_corpus();
    let (_, _, first, _) = search_json(&corpus, "search", "pass4_alpha", &[]);
    let (code, _, second, stderr) = search_json(&corpus, "search", "pass4_alpha", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(first, second, "repeated search must be byte-identical");
    assert!(!first.is_empty());
}

#[test]
fn search_files_with_matches_consistent_with_hits() {
    let corpus = search_corpus();
    let (code, value, _, stderr) =
        search_json(&corpus, "search", "pass4_alpha", &["--files-with-matches"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["ok"], true);
    let files = value["files"].as_array().expect("files array").clone();
    // Hand-computed: hits span exactly a.rs and b.rs, sorted and deduped.
    let expected = vec![Value::from("a.rs"), Value::from("b.rs")];
    assert_eq!(files, expected);
    // Consistency: files array equals the sorted unique hit files.
    let mut hit_files: Vec<String> = value["hits"]
        .as_array()
        .expect("hits")
        .iter()
        .map(|h| h["file"].as_str().expect("file").to_string())
        .collect();
    hit_files.sort();
    hit_files.dedup();
    let hit_files: Vec<Value> = hit_files.into_iter().map(Value::from).collect();
    assert_eq!(files, hit_files);

    // No-hits files-with-matches: empty files array, still exit 0.
    let (zcode, zvalue, _, _) = search_json(
        &corpus,
        "search",
        "zzz_no_such_symbol_zzz",
        &["--files-with-matches"],
    );
    assert_eq!(zcode, 0);
    assert_eq!(zvalue["ok"], true);
    assert_eq!(zvalue["files"].as_array().expect("files").len(), 0);
}

// ---------------------------------------------------------------------------
// reindex, chain, outline faces not owned elsewhere
// ---------------------------------------------------------------------------

#[test]
fn reindex_picks_up_added_file() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("corpus");
    write_bytes(&root, "a.rs", b"fn pass4_alpha() {}\n");
    let index = temp.path().join("index.db");
    let (icode, ivalue, _, _) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "index",
        root.to_str().unwrap(),
    ]);
    assert_eq!(icode, 0);
    assert_eq!(ivalue["files_indexed"], 1);

    write_bytes(&root, "b.rs", b"fn pass4_newcomer() {}\n");
    let (rcode, rvalue, _, rstderr) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "reindex",
        root.to_str().unwrap(),
    ]);
    assert_eq!(rcode, 0, "stderr={rstderr}");
    assert_eq!(rvalue["command"], "reindex");
    assert_eq!(rvalue["ok"], true);
    assert_eq!(rvalue["files_indexed"], 2);

    let corpus = Corpus {
        _temp: temp,
        root,
        index,
    };
    let (scode, svalue, _, sstderr) = search_json(&corpus, "search", "pass4_newcomer", &[]);
    assert_eq!(scode, 0, "stderr={sstderr}");
    let hits = svalue["hits"].as_array().expect("hits");
    assert!(
        hits.iter().any(|h| h["symbol"] == "pass4_newcomer" && h["file"] == "b.rs"),
        "reindex must surface the added file: {hits:?}"
    );
}

#[test]
fn chain_edges_follow_hand_call_graph() {
    let corpus = search_corpus();
    // Isolated symbol: single seed node, zero edges.
    let (gcode, gvalue, _, gstderr) = search_json(&corpus, "chain", "pass4_gamma", &[]);
    assert_eq!(gcode, 0, "stderr={gstderr}");
    assert_eq!(gvalue["command"], "chain");
    assert_eq!(gvalue["ok"], true);
    assert_eq!(gvalue["node_count"], 1);
    assert_eq!(gvalue["edge_count"], 0);
    assert_eq!(gvalue["edges"].as_array().expect("edges").len(), 0);

    // Connected symbol: the beta -> alpha edge is present.
    let (bcode, bvalue, _, bstderr) = search_json(&corpus, "chain", "pass4_beta", &[]);
    assert_eq!(bcode, 0, "stderr={bstderr}");
    assert_eq!(bvalue["ok"], true);
    assert_eq!(bvalue["edge_count"], 1);
    assert!(!bvalue["nodes"].as_array().expect("nodes").is_empty());
}

#[test]
fn outline_mixed_kinds_match_hand_spans() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("corpus");
    write_bytes(
        &root,
        "s.rs",
        b"struct Pass4Thing { x: i32 }\nfn pass4_method() {}\n",
    );
    let index = temp.path().join("index.db");
    let (icode, _, _, _) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "index",
        root.to_str().unwrap(),
    ]);
    assert_eq!(icode, 0);

    let (code, value, _, stderr) = run_json(&[
        "--index-path",
        index.to_str().unwrap(),
        "--no-auto-index",
        "--root",
        root.to_str().unwrap(),
        "outline",
        "s.rs",
        "--json",
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["command"], "outline");
    assert_eq!(value["ok"], true);
    assert_eq!(value["exit_code"], 0);
    assert_eq!(value["file"], "s.rs");
    // Hand-computed: struct on line 1, fn on line 2, line order.
    assert_eq!(value["count"], 2);
    let symbols = value["symbols"].as_array().expect("symbols");
    assert_eq!(symbols.len(), 2);
    assert_eq!(symbols[0]["name"], "Pass4Thing");
    assert_eq!(symbols[0]["kind"], "type");
    assert_eq!(symbols[0]["line_start"], 1);
    assert_eq!(symbols[1]["name"], "pass4_method");
    assert_eq!(symbols[1]["kind"], "function");
    assert_eq!(symbols[1]["line_start"], 2);
}
