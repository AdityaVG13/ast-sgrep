//! Pass 3 (oracle-foundry, Mission 3): L3 metamorphic / differential /
//! adversarial oracles for the asgrep CLI search + outline surface.
//!
//! Non-overlap: pass 1/2 own supervisor arithmetic (cpu limits, duty cycle),
//! UTF-16 offsets, identifier extraction, line lookup, symbol kinds, file
//! URIs, and text edits — none are re-tested here. Multi-pattern ingress,
//! files-with-matches faces, outline shapes, and machine envelopes own their
//! files; this file covers RELATIONS across runs and formats instead:
//!
//! - Metamorphic: repetition determinism, JSON-vs-human hit-count agreement,
//!   limit-growth prefix stability, filter-narrowing monotonicity,
//!   outline JSON-vs-human agreement.
//! - Differential: CLI `keyword` binary vs library `Searcher::search_lexical`
//!   (a channel the sample-fixture parity suite does not compare).
//! - Adversarial fixed inputs: unicode paths, empty files, CRLF, huge lines,
//!   invalid UTF-8, deep directories.
//!
//! Expectations are hand-computed from the documented contracts (total-order
//! ranking + cross-process byte-stability in `search/finish.rs`, one hit
//! line per human row in `search_cmd.rs`, exit contract 0=ok 1=usage 2=fail).
//! Assertions are discriminants (exit codes, `ok` booleans, counts, key
//! sets) — never message text.

use ast_sgrep_core::{SearchOptions, Searcher};
use ast_sgrep_testkit::{json_hit_keys, response_hit_keys, SurfaceHitKey};
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

fn index_dir(root: &Path, index: &Path) {
    let (code, _value, _raw, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "index",
        root.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "index must succeed: {stderr}");
}

/// Search corpus: `sentinel_alpha` occurs on 6 lines across a.rs (4) and
/// b.rs (2); c.rs never matches. Hand-computed file set: {a.rs, b.rs}.
fn search_corpus() -> Corpus {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("corpus");
    write_bytes(
        &root,
        "a.rs",
        b"fn alpha_one() {\n    let sentinel_alpha = 1;\n    println!(\"{sentinel_alpha}\");\n}\n\nfn alpha_two() {\n    let sentinel_alpha = 2;\n    println!(\"{sentinel_alpha}\");\n}\n\nfn plain_alpha() {}\n",
    );
    write_bytes(
        &root,
        "b.rs",
        b"fn beta_one() {\n    let sentinel_alpha = 3;\n    println!(\"{sentinel_alpha}\");\n}\n\nfn plain_beta() {}\n",
    );
    write_bytes(&root, "c.rs", b"fn plain_gamma() {}\n");
    let index = temp.path().join("index.db");
    index_dir(&root, &index);
    Corpus {
        _temp: temp,
        root,
        index,
    }
}

fn keyword_json(corpus: &Corpus, query: &str, extra: &[&str]) -> (i32, Value, Vec<u8>, String) {
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
    owned.push("keyword".into());
    owned.push(query.into());
    owned.push(corpus.root.to_str().unwrap().into());
    let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
    run_json(&refs)
}

fn sorted_keys(mut keys: Vec<SurfaceHitKey>) -> Vec<SurfaceHitKey> {
    keys.sort();
    keys
}

// ---------------------------------------------------------------------------
// Metamorphic: repetition determinism
// ---------------------------------------------------------------------------

#[test]
fn keyword_repetition_is_byte_identical() {
    let corpus = search_corpus();
    let (_, _, first, _) = keyword_json(&corpus, "sentinel_alpha", &[]);
    let (code, _, second, stderr) = keyword_json(&corpus, "sentinel_alpha", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    // Documented cross-process byte-stability contract: identical bytes.
    assert_eq!(first, second, "repeated search must be byte-identical");
    assert!(!first.is_empty());
}

#[test]
fn outline_repetition_is_byte_identical() {
    let corpus = outline_corpus();
    let run = || {
        run_json(&[
            "--index-path",
            corpus.index.to_str().unwrap(),
            "--no-auto-index",
            "--root",
            corpus.root.to_str().unwrap(),
            "outline",
            "src/main.rs",
            "--json",
        ])
    };
    let (code_a, _, first, _) = run();
    let (code_b, _, second, stderr) = run();
    assert_eq!((code_a, code_b), (0, 0), "stderr={stderr}");
    assert_eq!(first, second, "repeated outline must be byte-identical");
}

// ---------------------------------------------------------------------------
// Metamorphic: JSON-vs-human agreement
// ---------------------------------------------------------------------------

#[test]
fn json_vs_human_hit_count_agrees() {
    let corpus = search_corpus();
    let (code, value, _, stderr) = keyword_json(&corpus, "sentinel_alpha", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["ok"], true);
    let json_hits = value["hits"].as_array().expect("hits array").len();
    assert!(json_hits >= 3, "corpus must yield >=3 hits, got {json_hits}");

    let (hcode, hstdout, hstderr) = run_raw(&[
        "--index-path",
        corpus.index.to_str().unwrap(),
        "--no-embed",
        "--no-auto-index",
        "--limit",
        "50",
        "keyword",
        "sentinel_alpha",
        corpus.root.to_str().unwrap(),
    ]);
    assert_eq!(hcode, 0, "stderr={hstderr}");
    assert!(hstderr.is_empty(), "clean run stays silent: {hstderr}");
    let human = String::from_utf8_lossy(&hstdout);
    // One human row per hit, no headers or footers.
    assert_eq!(
        human.lines().count(),
        json_hits,
        "human rows must equal JSON hits"
    );
}

#[test]
fn files_with_matches_human_equals_json_files_array() {
    let corpus = search_corpus();
    let (code, value, _, stderr) =
        keyword_json(&corpus, "sentinel_alpha", &["--files-with-matches"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let files = value["files"].as_array().expect("files array").clone();
    // Hand-computed: exactly a.rs and b.rs match, sorted and deduped.
    let expected = vec![Value::from("a.rs"), Value::from("b.rs")];
    assert_eq!(files, expected);

    let (hcode, hstdout, hstderr) = run_raw(&[
        "--index-path",
        corpus.index.to_str().unwrap(),
        "--no-embed",
        "--no-auto-index",
        "--limit",
        "50",
        "keyword",
        "sentinel_alpha",
        corpus.root.to_str().unwrap(),
        "--files-with-matches",
    ]);
    assert_eq!(hcode, 0, "stderr={hstderr}");
    let human = String::from_utf8_lossy(&hstdout);
    let lines: Vec<&str> = human.lines().collect();
    assert_eq!(lines, vec!["a.rs", "b.rs"]);
}

// ---------------------------------------------------------------------------
// Metamorphic: limit growth + filter narrowing
// ---------------------------------------------------------------------------

#[test]
fn limit_growth_is_prefix_stable() {
    let corpus = search_corpus();
    let (big_code, big_value, _, _) = keyword_json(&corpus, "sentinel_alpha", &[]);
    assert_eq!(big_code, 0);
    let big = json_hit_keys(&big_value);
    assert!(big.len() >= 3, "need >=3 hits, got {}", big.len());

    // Same query at limit 2: total-order ranking truncates to an exact prefix.
    let root = corpus.root.to_str().unwrap().to_string();
    let index = corpus.index.to_str().unwrap().to_string();
    let (small_code, small_value, _, stderr) = run_json(&[
        "--index-path",
        &index,
        "--no-embed",
        "--no-auto-index",
        "--json",
        "--limit",
        "2",
        "keyword",
        "sentinel_alpha",
        &root,
    ]);
    assert_eq!(small_code, 0, "stderr={stderr}");
    let small = json_hit_keys(&small_value);
    assert_eq!(small.len(), 2);
    assert_eq!(
        small,
        big[..2].to_vec(),
        "limit-2 hits must prefix the limit-50 order"
    );

    // Saturation fixed point: growth past the hit count changes nothing.
    let (sat_code, sat_value, _, _) = keyword_json(&corpus, "sentinel_alpha", &[]);
    assert_eq!(sat_code, 0);
    assert_eq!(json_hit_keys(&sat_value), big);
}

#[test]
fn file_filter_narrowing_is_monotone() {
    let corpus = search_corpus();
    let (code, value, _, _) = keyword_json(&corpus, "sentinel_alpha", &[]);
    assert_eq!(code, 0);
    let all = json_hit_keys(&value);
    let files: std::collections::BTreeSet<&str> =
        all.iter().map(|k| k.file.as_str()).collect();
    assert!(
        files.len() >= 2,
        "corpus must span >=2 files, got {files:?}"
    );

    let (fcode, fvalue, _, fstderr) = keyword_json(
        &corpus,
        "sentinel_alpha",
        &["--file-filter", "a.rs"],
    );
    assert_eq!(fcode, 0, "stderr={fstderr}");
    assert_eq!(fvalue["ok"], true);
    let filtered = json_hit_keys(&fvalue);
    // Strong form: same total order, non-matching rows removed.
    let expected: Vec<SurfaceHitKey> = all
        .iter()
        .filter(|k| k.file == "a.rs")
        .cloned()
        .collect();
    assert!(!expected.is_empty(), "a.rs must match");
    assert_eq!(filtered, expected);
    assert!(filtered.len() < all.len(), "narrowing must shrink the set");

    // A filter matching nothing is ok:true with zero hits, exit 0.
    let (zcode, zvalue, _, _) = keyword_json(
        &corpus,
        "sentinel_alpha",
        &["--file-filter", "zzz-no-such-file-zzz"],
    );
    assert_eq!(zcode, 0);
    assert_eq!(zvalue["ok"], true);
    assert_eq!(zvalue["hits"].as_array().expect("hits").len(), 0);
}

// ---------------------------------------------------------------------------
// Differential: CLI binary vs library (keyword channel)
// ---------------------------------------------------------------------------

#[test]
fn keyword_cli_matches_library_search_lexical() {
    let corpus = search_corpus();
    let (code, value, _, stderr) = keyword_json(&corpus, "sentinel_alpha", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["command"], "keyword");
    let cli = sorted_keys(json_hit_keys(&value));
    assert!(!cli.is_empty(), "corpus must yield CLI hits");

    let searcher = Searcher::new(SearchOptions {
        root: corpus.root.clone(),
        index_path: Some(corpus.index.clone()),
        limit: 50,
        use_embed: false,
        ..SearchOptions::default()
    })
    .expect("library searcher");
    let response = searcher
        .search_lexical("sentinel_alpha")
        .expect("library search");
    let lib = sorted_keys(response_hit_keys(&response));
    assert_eq!(cli, lib, "CLI keyword must equal library search_lexical");
}

// ---------------------------------------------------------------------------
// Outline agreement
// ---------------------------------------------------------------------------

fn outline_corpus() -> Corpus {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("corpus");
    write_bytes(
        &root,
        "src/main.rs",
        b"fn alpha() {}\nfn beta() {}\nfn gamma() {}\n",
    );
    let index = temp.path().join("index.db");
    index_dir(&root, &index);
    Corpus {
        _temp: temp,
        root,
        index,
    }
}

#[test]
fn outline_json_human_and_count_agree() {
    let corpus = outline_corpus();
    let (code, value, _, stderr) = run_json(&[
        "--index-path",
        corpus.index.to_str().unwrap(),
        "--no-auto-index",
        "--root",
        corpus.root.to_str().unwrap(),
        "outline",
        "src/main.rs",
        "--json",
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["command"], "outline");
    assert_eq!(value["ok"], true);
    // Hand-computed: three one-line fns in line order.
    assert_eq!(value["count"], 3);
    let symbols = value["symbols"].as_array().expect("symbols");
    assert_eq!(symbols.len(), 3);
    let names: Vec<&str> = symbols.iter().map(|s| s["name"].as_str().expect("name")).collect();
    assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    let starts: Vec<u64> = symbols
        .iter()
        .map(|s| s["line_start"].as_u64().expect("line_start"))
        .collect();
    assert_eq!(starts, vec![1, 2, 3]);
    assert!(symbols.iter().all(|s| s["kind"] == "function"));

    let (hcode, hstdout, hstderr) = run_raw(&[
        "--index-path",
        corpus.index.to_str().unwrap(),
        "--no-auto-index",
        "--root",
        corpus.root.to_str().unwrap(),
        "outline",
        "src/main.rs",
    ]);
    assert_eq!(hcode, 0, "stderr={hstderr}");
    let human = String::from_utf8_lossy(&hstdout);
    assert_eq!(human.lines().count(), 3, "one human row per symbol");
    for name in ["alpha", "beta", "gamma"] {
        assert!(human.contains(name), "human view names {name}");
    }
}

// ---------------------------------------------------------------------------
// Adversarial fixed inputs
// ---------------------------------------------------------------------------

fn adversarial_corpus(files: &[(&str, &[u8])]) -> Corpus {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("corpus");
    for (rel, bytes) in files {
        write_bytes(&root, rel, bytes);
    }
    let index = temp.path().join("index.db");
    index_dir(&root, &index);
    Corpus {
        _temp: temp,
        root,
        index,
    }
}

fn keyword_codes(corpus: &Corpus, query: &str) -> (i32, Value, String) {
    let (code, value, _, stderr) = keyword_json(corpus, query, &[]);
    (code, value, stderr)
}

#[test]
fn unicode_paths_index_search_and_outline() {
    let name = "héllo_世界.rs";
    let corpus = adversarial_corpus(&[(name, b"fn unicode_sentinel_fn() {}\n")]);
    let (code, value, stderr) = keyword_codes(&corpus, "unicode_sentinel_fn");
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().expect("hits");
    assert!(
        hits.iter().any(|h| h["file"] == name),
        "unicode path must round-trip through search: {hits:?}"
    );

    let (ocode, ovalue, _, ostderr) = run_json(&[
        "--index-path",
        corpus.index.to_str().unwrap(),
        "--no-auto-index",
        "--root",
        corpus.root.to_str().unwrap(),
        "outline",
        name,
        "--json",
    ]);
    assert_eq!(ocode, 0, "stderr={ostderr}");
    assert_eq!(ovalue["file"], name);
    assert_eq!(ovalue["count"], 1);
}

#[test]
fn empty_file_indexes_searches_and_outline_refuses() {
    let corpus = adversarial_corpus(&[("empty.rs", b""), ("full.rs", b"fn full_fn() {}\n")]);
    let (code, value, stderr) = keyword_codes(&corpus, "full_fn");
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["ok"], true);

    // No indexed symbols: fail-closed operational error (exit 2, ok false).
    let (ocode, ovalue, _, _) = run_json(&[
        "--index-path",
        corpus.index.to_str().unwrap(),
        "--no-auto-index",
        "--root",
        corpus.root.to_str().unwrap(),
        "outline",
        "empty.rs",
        "--json",
    ]);
    assert_eq!(ocode, 2);
    assert_eq!(ovalue["ok"], false);
    assert_eq!(ovalue["exit_code"], 2);
}

#[test]
fn crlf_line_spans_are_hand_computed() {
    let corpus = adversarial_corpus(&[("crlf.rs", b"fn crlf_one() {}\r\nfn crlf_two() {}\r\n")]);
    let (code, value, _, stderr) = run_json(&[
        "--index-path",
        corpus.index.to_str().unwrap(),
        "--no-auto-index",
        "--root",
        corpus.root.to_str().unwrap(),
        "outline",
        "crlf.rs",
        "--json",
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["count"], 2);
    let symbols = value["symbols"].as_array().expect("symbols");
    let starts: Vec<u64> = symbols
        .iter()
        .map(|s| s["line_start"].as_u64().expect("line_start"))
        .collect();
    assert_eq!(starts, vec![1, 2]);
}

#[test]
fn huge_single_line_is_stable_and_located() {
    let mut line = vec![b'x'; 150_000];
    line.extend_from_slice(b" hugesentinel ");
    line.extend_from_slice(&vec![b'y'; 150_000]);
    line.push(b'\n');
    let corpus = adversarial_corpus(&[("huge.rs", &line)]);
    let (code, value, stderr) = keyword_codes(&corpus, "hugesentinel");
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().expect("hits");
    assert!(
        hits.iter()
            .any(|h| h["file"] == "huge.rs" && h["line_start"] == 1),
        "huge-line hit must locate line 1: {hits:?}"
    );
}

#[test]
fn invalid_utf8_is_stable_with_valid_json_output() {
    // A lone hostile file would leave the index EMPTY (skipped as binary),
    // tripping the documented empty-index fail-closed (exit 2) — so the
    // corpus also carries a clean file; the oracle is that the hostile file
    // neither crashes indexing nor poisons search over the rest.
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("corpus");
    write_bytes(
        &root,
        "broken.rs",
        b"fn broken_one() {}\n\xff\xfe\x00bad\x80bytes\nfn broken_two() {}\n",
    );
    write_bytes(&root, "clean.rs", b"fn clean_sentinel() {}\n");
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
    assert_eq!(ivalue["ok"], true);
    // Hand-computed: hostile file counted as failed, clean file indexed.
    assert_eq!(ivalue["files_failed"], 1);
    assert_eq!(ivalue["files_indexed"], 1);
    let corpus = Corpus {
        _temp: temp,
        root,
        index,
    };
    // Key discriminant: stdout stays valid machine JSON despite hostile input.
    let (code, value, stderr) = keyword_codes(&corpus, "broken_one");
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["ok"], true);
    assert!(value["hits"].is_array());
    // The clean file stays searchable: no result poisoning.
    let (ccode, cvalue, cstderr) = keyword_codes(&corpus, "clean_sentinel");
    assert_eq!(ccode, 0, "stderr={cstderr}");
    let chits = cvalue["hits"].as_array().expect("hits");
    assert!(
        chits.iter().any(|h| h["file"] == "clean.rs"),
        "clean file must stay searchable: {chits:?}"
    );
}

#[test]
fn deep_directories_index_and_search() {
    let mut rel = String::new();
    for depth in 0..25 {
        rel.push_str(&format!("d{depth}/"));
    }
    rel.push_str("leaf.rs");
    let corpus = adversarial_corpus(&[(rel.as_str(), b"fn deep_leaf_fn() {}\n")]);
    let (code, value, stderr) = keyword_codes(&corpus, "deep_leaf_fn");
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().expect("hits");
    assert!(
        hits.iter().any(|h| h["file"]
            .as_str()
            .is_some_and(|f| f.ends_with("leaf.rs"))),
        "deep leaf must be found: {hits:?}"
    );
}
