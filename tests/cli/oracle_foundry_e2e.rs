//! Consolidated CLI oracle e2e: L3 metamorphic/differential/adversarial
//! oracles (pass3) plus L4 end-to-end flows (pass4), grouped by channel.
//! Every test drives the REAL `asgrep` binary on tempdir fixture trees.
//!
//! Channel-mirror discipline: search≠keyword mirrors are load-bearing
//! (distinct code paths), so each channel pins its own repetition /
//! human-agreement / files-with-matches facets inside its face test; where
//! both channels pin the SAME contract (no-hits), one test loops both
//! channels (the no_hits pattern) instead of two tests.
//!
//! Expectations are hand-computed. Assertions are discriminants (exit
//! codes, `ok` booleans, counts, key sets) — never message text.

#[path = "oracle_foundry_common.rs"]
mod common;

use ast_sgrep_core::{SearchOptions, Searcher};
use ast_sgrep_testkit::{
    json_hit_keys, oracle_keyword_corpus, oracle_outline_corpus, oracle_run_json,
    oracle_search_corpus, response_hit_keys, sorted_surface_keys, write_root_bytes, OracleCorpus,
    SurfaceHitKey,
};
use common::asgrep;

/// INTENT: the keyword channel face is coherent — repetition is
/// byte-identical, human rows agree with JSON hits, files-with-matches is
/// exactly [a.rs,b.rs] on both faces, limit growth is prefix-stable with a
/// saturation fixed point, file-filter narrowing is monotone (and empty
/// filters stay ok:true), and CLI keyword equals library search_lexical.
/// KILLS: nondeterminism (hash-order/timestamp leak), human-face
/// header/footer/row-skew, sort/dedupe/face-divergence,
/// limit-dependent-ranking, filter-ignored/order-scramble, CLI-envelope/
/// marshal divergence mutants.
/// ABSORBS: keyword_repetition_is_byte_identical +
/// json_vs_human_hit_count_agrees +
/// files_with_matches_human_equals_json_files_array +
/// limit_growth_is_prefix_stable + file_filter_narrowing_is_monotone +
/// keyword_cli_matches_library_search_lexical.
#[test]
fn keyword_face_is_coherent() {
    let corpus = oracle_keyword_corpus(&asgrep());

    // Facet 1: repeated keyword JSON run is byte-identical, non-empty.
    let (_, _, first, _) = corpus.query_json("keyword", "sentinel_alpha", &[]);
    let (code, _, second, stderr) = corpus.query_json("keyword", "sentinel_alpha", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(first, second, "repeated search must be byte-identical");
    assert!(!first.is_empty());

    // Facet 2: human keyword rows == JSON hits count, clean stderr.
    let (code, value, _, stderr) = corpus.query_json("keyword", "sentinel_alpha", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["ok"], true);
    let json_hits = value["hits"].as_array().expect("hits array").len();
    assert!(
        json_hits >= 3,
        "corpus must yield >=3 hits, got {json_hits}"
    );
    let (hcode, hstdout, hstderr) = corpus.query_human("keyword", "sentinel_alpha", &[]);
    assert_eq!(hcode, 0, "stderr={hstderr}");
    assert!(hstderr.is_empty(), "clean run stays silent: {hstderr}");
    let human = String::from_utf8_lossy(&hstdout);
    assert_eq!(
        human.lines().count(),
        json_hits,
        "human rows must equal JSON hits"
    );

    // Facet 3: files face is exactly [a.rs,b.rs] sorted/deduped, both faces.
    let (code, value, _, stderr) =
        corpus.query_json("keyword", "sentinel_alpha", &["--files-with-matches"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let files = value["files"].as_array().expect("files array").clone();
    assert_eq!(
        files,
        vec![
            serde_json::Value::from("a.rs"),
            serde_json::Value::from("b.rs")
        ]
    );
    let (hcode, hstdout, hstderr) =
        corpus.query_human("keyword", "sentinel_alpha", &["--files-with-matches"]);
    assert_eq!(hcode, 0, "stderr={hstderr}");
    let human = String::from_utf8_lossy(&hstdout);
    assert_eq!(human.lines().collect::<Vec<_>>(), vec!["a.rs", "b.rs"]);

    // Facet 4: limit-2 hits == limit-50 prefix; saturation is fixed point.
    let (big_code, big_value, _, _) = corpus.query_json("keyword", "sentinel_alpha", &[]);
    assert_eq!(big_code, 0);
    let big = json_hit_keys(&big_value);
    assert!(big.len() >= 3, "need >=3 hits, got {}", big.len());
    let (small_code, small_value, _, stderr) =
        corpus.query_json_limit("keyword", "sentinel_alpha", "2", &[]);
    assert_eq!(small_code, 0, "stderr={stderr}");
    let small = json_hit_keys(&small_value);
    assert_eq!(small.len(), 2);
    assert_eq!(
        small,
        big[..2].to_vec(),
        "limit-2 hits must prefix the limit-50 order"
    );
    let (sat_code, sat_value, _, _) = corpus.query_json("keyword", "sentinel_alpha", &[]);
    assert_eq!(sat_code, 0);
    assert_eq!(json_hit_keys(&sat_value), big);

    // Facet 5: filter keeps total order minus removed rows, shrinks the
    // set; no-match filter is ok:true with zero hits.
    let (code, value, _, _) = corpus.query_json("keyword", "sentinel_alpha", &[]);
    assert_eq!(code, 0);
    let all = json_hit_keys(&value);
    let files: std::collections::BTreeSet<&str> = all.iter().map(|k| k.file.as_str()).collect();
    assert!(
        files.len() >= 2,
        "corpus must span >=2 files, got {files:?}"
    );
    let (fcode, fvalue, _, fstderr) =
        corpus.query_json("keyword", "sentinel_alpha", &["--file-filter", "a.rs"]);
    assert_eq!(fcode, 0, "stderr={fstderr}");
    assert_eq!(fvalue["ok"], true);
    let filtered = json_hit_keys(&fvalue);
    let expected: Vec<SurfaceHitKey> = all.iter().filter(|k| k.file == "a.rs").cloned().collect();
    assert!(!expected.is_empty(), "a.rs must match");
    assert_eq!(filtered, expected);
    assert!(filtered.len() < all.len(), "narrowing must shrink the set");
    let (zcode, zvalue, _, _) = corpus.query_json(
        "keyword",
        "sentinel_alpha",
        &["--file-filter", "zzz-no-such-file-zzz"],
    );
    assert_eq!(zcode, 0);
    assert_eq!(zvalue["ok"], true);
    assert_eq!(zvalue["hits"].as_array().expect("hits").len(), 0);

    // Facet 6: CLI keyword hit keys == library search_lexical keys (sorted).
    let (code, value, _, stderr) = corpus.query_json("keyword", "sentinel_alpha", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["command"], "keyword");
    let cli = sorted_surface_keys(json_hit_keys(&value));
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
    let lib = sorted_surface_keys(response_hit_keys(&response));
    assert_eq!(cli, lib, "CLI keyword must equal library search_lexical");
}

/// INTENT: the search channel face is coherent — the hand-placed symbol
/// hits first with symbol/file/1-1 span, human rows agree with JSON hits,
/// repetition is byte-identical, files-with-matches is exactly [a.rs,b.rs]
/// and consistent with hit files, and no-hits is ok:true exit 0 with empty
/// hits/human-stdout on BOTH channels (looped in this one test).
/// KILLS: ranking/span-shape, human-face skew (search path),
/// nondeterminism (search path), files/hits inconsistency, fail-on-empty mutants.
/// ABSORBS: search_finds_hand_placed_symbol_first +
/// search_human_rows_equal_json_hits + search_repetition_is_byte_identical +
/// search_files_with_matches_consistent_with_hits +
/// no_hits_is_ok_empty_exit_zero_both_channels (already channel-looped; the
/// merge pattern — one test loops both channels rather than two tests).
#[test]
fn search_face_is_coherent() {
    let corpus = oracle_search_corpus(&asgrep());

    // Facet 1: exact definition hit first — symbol, file, one-line span.
    let (code, value, _, stderr) = corpus.query_json("search", "pass4_alpha", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["command"], "search");
    assert_eq!(value["ok"], true);
    assert_eq!(value["exit_code"], 0);
    assert_eq!(value["query"], "pass4_alpha");
    let hits = value["hits"].as_array().expect("hits array");
    assert!(!hits.is_empty(), "hand-placed symbol must hit");
    assert_eq!(hits[0]["symbol"], "pass4_alpha");
    assert_eq!(hits[0]["file"], "a.rs");
    assert_eq!(hits[0]["line_start"], 1);
    assert_eq!(hits[0]["line_end"], 1);
    assert!(hits
        .iter()
        .all(|h| h["symbol"].is_string() && h["file"].is_string()));

    // Facet 2: human search rows == JSON hits count, clean stderr.
    let (code, value, _, stderr) = corpus.query_json("search", "pass4_alpha", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    let json_hits = value["hits"].as_array().expect("hits array").len();
    assert!(json_hits >= 1, "corpus must yield hits");
    let (hcode, hstdout, hstderr) = corpus.query_human("search", "pass4_alpha", &[]);
    assert_eq!(hcode, 0, "stderr={hstderr}");
    assert!(hstderr.is_empty(), "clean run stays silent: {hstderr}");
    let human = String::from_utf8_lossy(&hstdout);
    assert_eq!(
        human.lines().count(),
        json_hits,
        "human rows must equal JSON hits"
    );

    // Facet 3: repeated search JSON run byte-identical, non-empty.
    let (_, _, first, _) = corpus.query_json("search", "pass4_alpha", &[]);
    let (code, _, second, stderr) = corpus.query_json("search", "pass4_alpha", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(first, second, "repeated search must be byte-identical");
    assert!(!first.is_empty());

    // Facet 4: files==[a.rs,b.rs], equal to the sorted unique hit files;
    // no-hits files face is empty with exit 0.
    let (code, value, _, stderr) =
        corpus.query_json("search", "pass4_alpha", &["--files-with-matches"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["ok"], true);
    let files = value["files"].as_array().expect("files array").clone();
    assert_eq!(
        files,
        vec![
            serde_json::Value::from("a.rs"),
            serde_json::Value::from("b.rs")
        ]
    );
    let mut hit_files: Vec<String> = value["hits"]
        .as_array()
        .expect("hits")
        .iter()
        .map(|h| h["file"].as_str().expect("file").to_string())
        .collect();
    hit_files.sort();
    hit_files.dedup();
    let hit_files: Vec<serde_json::Value> =
        hit_files.into_iter().map(serde_json::Value::from).collect();
    assert_eq!(files, hit_files);
    let (zcode, zvalue, _, _) = corpus.query_json(
        "search",
        "zzz_no_such_symbol_zzz",
        &["--files-with-matches"],
    );
    assert_eq!(zcode, 0);
    assert_eq!(zvalue["ok"], true);
    assert_eq!(zvalue["files"].as_array().expect("files").len(), 0);

    // Facet 5: no-hits is ok:true exit 0 with empty hits/human-stdout —
    // looped over BOTH channels in this one test.
    for channel in ["search", "keyword"] {
        let (code, value, _, stderr) = corpus.query_json(channel, "zzz_no_such_symbol_zzz", &[]);
        assert_eq!(code, 0, "channel={channel} stderr={stderr}");
        assert_eq!(value["ok"], true, "channel={channel}");
        assert_eq!(value["exit_code"], 0, "channel={channel}");
        assert_eq!(
            value["hits"].as_array().expect("hits").len(),
            0,
            "channel={channel}"
        );
        let (hcode, hstdout, hstderr) = corpus.query_human(channel, "zzz_no_such_symbol_zzz", &[]);
        assert_eq!(hcode, 0, "channel={channel} stderr={hstderr}");
        assert!(hstdout.is_empty(), "channel={channel}");
        assert!(hstderr.is_empty(), "channel={channel}");
    }
}

/// INTENT: the outline face is coherent — 3 fns in line order with
/// hand-pinned names/starts/kinds on both faces, repetition is
/// byte-identical, and mixed kinds (struct→type + fn→function) keep
/// hand spans in line order.
/// KILLS: outline ordering/kind/span, nondeterminism (outline path),
/// kind-conflation mutants.
/// ABSORBS: outline_json_human_and_count_agree +
/// outline_repetition_is_byte_identical (distinct binary path from keyword;
/// kept as a facet, not folded into the keyword face) +
/// outline_mixed_kinds_match_hand_spans.
#[test]
fn outline_face_is_coherent() {
    let corpus = oracle_outline_corpus(&asgrep());

    // Facet 1: 3 fns in line order, names/starts/kinds hand-pinned, human
    // face has 3 rows naming each fn.
    let (code, value, _, stderr) = corpus.outline_json("src/main.rs");
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["command"], "outline");
    assert_eq!(value["ok"], true);
    assert_eq!(value["count"], 3);
    let symbols = value["symbols"].as_array().expect("symbols");
    assert_eq!(symbols.len(), 3);
    let names: Vec<&str> = symbols
        .iter()
        .map(|s| s["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    let starts: Vec<u64> = symbols
        .iter()
        .map(|s| s["line_start"].as_u64().expect("line_start"))
        .collect();
    assert_eq!(starts, vec![1, 2, 3]);
    assert!(symbols.iter().all(|s| s["kind"] == "function"));
    let (hcode, hstdout, hstderr) = corpus.outline_human("src/main.rs");
    assert_eq!(hcode, 0, "stderr={hstderr}");
    let human = String::from_utf8_lossy(&hstdout);
    assert_eq!(human.lines().count(), 3, "one human row per symbol");
    for name in ["alpha", "beta", "gamma"] {
        assert!(human.contains(name), "human view names {name}");
    }

    // Facet 2: repeated outline JSON run is byte-identical.
    let (code_a, _, first, _) = corpus.outline_json("src/main.rs");
    let (code_b, _, second, stderr) = corpus.outline_json("src/main.rs");
    assert_eq!((code_a, code_b), (0, 0), "stderr={stderr}");
    assert_eq!(first, second, "repeated outline must be byte-identical");

    // Facet 3: struct line 1 kind type + fn line 2 kind function, line order.
    let (mixed, _) = OracleCorpus::index_files(
        &asgrep(),
        &[(
            "s.rs",
            b"struct Pass4Thing { x: i32 }\nfn pass4_method() {}\n".as_slice(),
        )],
    );
    let (code, value, _, stderr) = mixed.outline_json("s.rs");
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["command"], "outline");
    assert_eq!(value["ok"], true);
    assert_eq!(value["exit_code"], 0);
    assert_eq!(value["file"], "s.rs");
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

/// INTENT: index/status/chain flows hold end to end — index and status
/// envelopes carry hand counts, chain edges follow the hand call graph,
/// reindex picks up an added file, and missing roots / empty indexes fail
/// closed (exit 2, ok:false, operational).
/// KILLS: count/envelope-shape, status-count, call-graph edge-drop,
/// stale-index/reindex-noop, fail-open-on-missing-root,
/// empty-index fail-open mutants.
/// ABSORBS: index_envelope_counts_match_hand_fixture +
/// status_counts_match_hand_fixture + chain_edges_follow_hand_call_graph +
/// reindex_picks_up_added_file + missing_root_fails_closed_across_commands +
/// empty_tree_index_ok_then_search_fails_closed.
#[test]
fn index_status_chain_flows() {
    // Facets 1-3 share one hand corpus: 2 files, 3 symbols, 1 caller edge.
    let (corpus, index_value) = OracleCorpus::index_files(
        &asgrep(),
        &[
            (
                "a.rs",
                b"fn pass4_alpha() {}\nfn pass4_beta() { pass4_alpha(); }\n".as_slice(),
            ),
            ("b.rs", b"fn pass4_gamma() {}\n".as_slice()),
        ],
    );

    // Facet 1: index envelope hand counts 2 files / 0 failed / 3 symbols.
    assert_eq!(index_value["command"], "index");
    assert_eq!(index_value["ok"], true);
    assert_eq!(index_value["exit_code"], 0);
    assert_eq!(index_value["files_indexed"], 2);
    assert_eq!(index_value["files_failed"], 0);
    assert_eq!(index_value["symbols_extracted"], 3);

    // Facet 2: status hand counts 2 files / 3 symbols / 1 caller edge.
    let (code, value, _, stderr) = corpus.status_json();
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["command"], "status");
    assert_eq!(value["ok"], true);
    assert_eq!(value["exit_code"], 0);
    assert_eq!(value["file_count"], 2);
    assert_eq!(value["symbol_count"], 3);
    assert_eq!(value["caller_count"], 1);

    // Facet 3: isolated symbol 1 node/0 edges; beta chain carries the 1
    // hand edge.
    let (gcode, gvalue, _, gstderr) = corpus.query_json("chain", "pass4_gamma", &[]);
    assert_eq!(gcode, 0, "stderr={gstderr}");
    assert_eq!(gvalue["command"], "chain");
    assert_eq!(gvalue["ok"], true);
    assert_eq!(gvalue["node_count"], 1);
    assert_eq!(gvalue["edge_count"], 0);
    assert_eq!(gvalue["edges"].as_array().expect("edges").len(), 0);
    let (bcode, bvalue, _, bstderr) = corpus.query_json("chain", "pass4_beta", &[]);
    assert_eq!(bcode, 0, "stderr={bstderr}");
    assert_eq!(bvalue["ok"], true);
    assert_eq!(bvalue["edge_count"], 1);
    assert!(!bvalue["nodes"].as_array().expect("nodes").is_empty());

    // Facet 4: reindex after adding b.rs reports 2 files and surfaces the newcomer.
    let (re_corpus, re_value) =
        OracleCorpus::index_files(&asgrep(), &[("a.rs", b"fn pass4_alpha() {}\n".as_slice())]);
    assert_eq!(re_value["files_indexed"], 1);
    write_root_bytes(&re_corpus.root, "b.rs", b"fn pass4_newcomer() {}\n");
    let (rcode, rvalue, _, rstderr) = re_corpus.reindex();
    assert_eq!(rcode, 0, "stderr={rstderr}");
    assert_eq!(rvalue["command"], "reindex");
    assert_eq!(rvalue["ok"], true);
    assert_eq!(rvalue["files_indexed"], 2);
    let (scode, svalue, _, sstderr) = re_corpus.query_json("search", "pass4_newcomer", &[]);
    assert_eq!(scode, 0, "stderr={sstderr}");
    let hits = svalue["hits"].as_array().expect("hits");
    assert!(
        hits.iter()
            .any(|h| h["symbol"] == "pass4_newcomer" && h["file"] == "b.rs"),
        "reindex must surface the added file: {hits:?}"
    );

    // Facet 5: index and search on a missing root are exit 2 ok:false operational.
    let missing = corpus.temp.path().join("does-not-exist");
    let missing = missing.to_str().unwrap();
    let (icode, ivalue, _, _) = oracle_run_json(
        &asgrep(),
        &[
            "--json",
            "--no-embed",
            "--index-path",
            corpus.temp.path().join("other.db").to_str().unwrap(),
            "index",
            missing,
        ],
    );
    assert_eq!(icode, 2);
    assert_eq!(ivalue["ok"], false);
    assert_eq!(ivalue["exit_code"], 2);
    assert_eq!(ivalue["error"]["kind"], "operational");
    let (scode, svalue, _, _) = corpus.run_json(&[
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

    // Facet 6: empty tree indexes ok with zero counts, search on it fails closed.
    let (empty, empty_value) = OracleCorpus::index_files(&asgrep(), &[]);
    assert_eq!(empty_value["ok"], true);
    assert_eq!(empty_value["files_indexed"], 0);
    assert_eq!(empty_value["symbols_extracted"], 0);
    assert_eq!(empty_value["files_failed"], 0);
    let (scode, svalue, _, _) = empty.run_json(&[
        "--index-path",
        empty.index.to_str().unwrap(),
        "--no-embed",
        "--no-auto-index",
        "--json",
        "search",
        "anything",
        empty.root.to_str().unwrap(),
    ]);
    assert_eq!(scode, 2);
    assert_eq!(svalue["ok"], false);
    assert_eq!(svalue["exit_code"], 2);
    assert_eq!(svalue["error"]["kind"], "operational");
}

/// INTENT: adversarial files stay stable — non-ASCII paths round-trip
/// through index/search/outline, empty files coexist with search while
/// outline refuses them closed, CRLF spans are hand-computed, a 300KB
/// single line locates at line 1, invalid UTF-8 neither crashes indexing
/// nor poisons search, and 25-deep trees index and search.
/// KILLS: path-mangling/non-UTF8-loss, fail-open-on-empty,
/// CRLF line-split, line-length truncation/OOM-path, crash-on-invalid-UTF8/
/// result-poisoning, depth-limit/walk-prune mutants.
/// ABSORBS: unicode_paths_index_search_and_outline,
/// empty_file_indexes_searches_and_outline_refuses,
/// crlf_line_spans_are_hand_computed, huge_single_line_is_stable_and_located,
/// invalid_utf8_is_stable_with_valid_json_output,
/// deep_directories_index_and_search.
#[test]
fn adversarial_files_stay_stable() {
    // Facet 1: non-ASCII filename round-trips through index, search, outline.
    let name = "héllo_世界.rs";
    let (corpus, _) = OracleCorpus::index_files(
        &asgrep(),
        &[(name, b"fn unicode_sentinel_fn() {}\n".as_slice())],
    );
    let (code, value, _, stderr) = corpus.query_json("keyword", "unicode_sentinel_fn", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().expect("hits");
    assert!(
        hits.iter().any(|h| h["file"] == name),
        "unicode path must round-trip through search: {hits:?}"
    );
    let (ocode, ovalue, _, ostderr) = corpus.outline_json(name);
    assert_eq!(ocode, 0, "stderr={ostderr}");
    assert_eq!(ovalue["file"], name);
    assert_eq!(ovalue["count"], 1);

    // Facet 2: empty file coexists with search; outline on it is exit 2 ok:false.
    let (corpus, _) = OracleCorpus::index_files(
        &asgrep(),
        &[
            ("empty.rs", b"".as_slice()),
            ("full.rs", b"fn full_fn() {}\n".as_slice()),
        ],
    );
    let (code, value, _, stderr) = corpus.query_json("keyword", "full_fn", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["ok"], true);
    let (ocode, ovalue, _, _) = corpus.outline_json("empty.rs");
    assert_eq!(ocode, 2);
    assert_eq!(ovalue["ok"], false);
    assert_eq!(ovalue["exit_code"], 2);

    // Facet 3: CRLF file outlines count 2 with starts [1,2].
    let (corpus, _) = OracleCorpus::index_files(
        &asgrep(),
        &[(
            "crlf.rs",
            b"fn crlf_one() {}\r\nfn crlf_two() {}\r\n".as_slice(),
        )],
    );
    let (code, value, _, stderr) = corpus.outline_json("crlf.rs");
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["count"], 2);
    let symbols = value["symbols"].as_array().expect("symbols");
    let starts: Vec<u64> = symbols
        .iter()
        .map(|s| s["line_start"].as_u64().expect("line_start"))
        .collect();
    assert_eq!(starts, vec![1, 2]);

    // Facet 4: 300KB single-line file indexes and locates hit at line 1.
    let mut line = vec![b'x'; 150_000];
    line.extend_from_slice(b" hugesentinel ");
    line.extend_from_slice(&vec![b'y'; 150_000]);
    line.push(b'\n');
    let (corpus, _) = OracleCorpus::index_files(&asgrep(), &[("huge.rs", line.as_slice())]);
    let (code, value, _, stderr) = corpus.query_json("keyword", "hugesentinel", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().expect("hits");
    assert!(
        hits.iter()
            .any(|h| h["file"] == "huge.rs" && h["line_start"] == 1),
        "huge-line hit must locate line 1: {hits:?}"
    );

    // Facet 5: hostile file counted files_failed=1, stdout stays valid
    // JSON, clean file searchable. The corpus carries a clean file because
    // a lone hostile file would leave the index EMPTY (skipped as binary),
    // tripping the documented empty-index fail-closed (exit 2).
    let (corpus, index_value) = OracleCorpus::index_files(
        &asgrep(),
        &[
            (
                "broken.rs",
                b"fn broken_one() {}\n\xff\xfe\x00bad\x80bytes\nfn broken_two() {}\n".as_slice(),
            ),
            ("clean.rs", b"fn clean_sentinel() {}\n".as_slice()),
        ],
    );
    assert_eq!(index_value["ok"], true);
    assert_eq!(index_value["files_failed"], 1);
    assert_eq!(index_value["files_indexed"], 1);
    let (code, value, stderr) = {
        let (code, value, _, stderr) = corpus.query_json("keyword", "broken_one", &[]);
        (code, value, stderr)
    };
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["ok"], true);
    assert!(value["hits"].is_array());
    let (ccode, cvalue, _, cstderr) = corpus.query_json("keyword", "clean_sentinel", &[]);
    assert_eq!(ccode, 0, "stderr={cstderr}");
    let chits = cvalue["hits"].as_array().expect("hits");
    assert!(
        chits.iter().any(|h| h["file"] == "clean.rs"),
        "clean file must stay searchable: {chits:?}"
    );

    // Facet 6: 25-deep nested leaf.rs indexes and is found by search.
    let mut rel = String::new();
    for depth in 0..25 {
        rel.push_str(&format!("d{depth}/"));
    }
    rel.push_str("leaf.rs");
    let (corpus, _) = OracleCorpus::index_files(
        &asgrep(),
        &[(rel.as_str(), b"fn deep_leaf_fn() {}\n".as_slice())],
    );
    let (code, value, _, stderr) = corpus.query_json("keyword", "deep_leaf_fn", &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().expect("hits");
    assert!(
        hits.iter()
            .any(|h| h["file"].as_str().is_some_and(|f| f.ends_with("leaf.rs"))),
        "deep leaf must be found: {hits:?}"
    );
}
