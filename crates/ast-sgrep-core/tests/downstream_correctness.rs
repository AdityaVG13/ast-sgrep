//! Downstream correctness beads (PR #22 wave): 2hhq, 50hx, ql1u, firi, 6dx9, vwga, …
use ast_sgrep_core::chain::{expand_chain, ChainConfig};
use ast_sgrep_core::query::{ParsedQuery, QueryMode};
use ast_sgrep_core::search::{SearchOptions, Searcher};
use ast_sgrep_core::semantic_ann::SemanticAnnIndex;
use ast_sgrep_core::store::{CallerRow, SymbolRow, UpsertFileInput};
use ast_sgrep_core::tantivy_index::{should_use_tantivy, TANTIVY_AUTO_THRESHOLD};
use ast_sgrep_core::{IndexOptions, IndexStore, Indexer};
use ast_sgrep_embed::{top_k_flat_similarity, MIN_SIMILARITY};
use ast_sgrep_testkit::{index_sample, response_hit_keys, sample_root, searcher_from};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn base<'a>(
    path: &'a str,
    language: Option<&'a str>,
    lines: &'a [(u32, String)],
    hash: &'a str,
) -> UpsertFileInput<'a> {
    UpsertFileInput {
        rel_path: path,
        language,
        mtime_secs: 1,
        mtime_nanos: 0,
        content_hash: hash,
        lines,
        eol: "\n",
        symbols: &[],
        callers: &[],
        imports: &[],
        pattern_nodes: &[],
        semantic_chunks: &[],
        embed_semantic: false,
        embed_backend: ast_sgrep_embed::EmbedPreference::Auto,
    }
}

fn write_src(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

/// 2hhq — edges to truncated-out nodes must be dropped (count matches edges.len()).
#[test]
fn bead_2hhq_chain_drops_edges_to_truncated_nodes() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let symbols_a = [SymbolRow {
        name: "seed_fn".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        byte_start: 0,
        byte_end: 7,
    }];
    let callers_a = [CallerRow {
        line_no: 2,
        caller: "seed_fn".into(),
        callee: "hop_target".into(),
        byte_start: 0,
        byte_end: 0,
    }];
    let lines_a = [
        (1u32, "fn seed_fn() { hop_target(); }".into()),
        (2u32, "    hop_target();".into()),
    ];
    let mut input_a = base("seed.rs", Some("rust"), &lines_a, "hseed");
    input_a.symbols = &symbols_a;
    input_a.callers = &callers_a;
    store.upsert_file(input_a).unwrap();

    let symbols_b = [SymbolRow {
        name: "hop_target".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        byte_start: 0,
        byte_end: 10,
    }];
    let lines_b = [(1u32, "fn hop_target() {}".into())];
    let mut input_b = base("hop.rs", Some("rust"), &lines_b, "hhop");
    input_b.symbols = &symbols_b;
    store.upsert_file(input_b).unwrap();

    for i in 0..8 {
        let name = format!("filler{i}");
        let symbols = [SymbolRow {
            name: name.clone(),
            kind: "function".into(),
            line_start: 1,
            line_end: 1,
            byte_start: 0,
            byte_end: 1,
        }];
        let lines = [(1u32, format!("fn {name}() {{}}"))];
        let path = format!("f{i}.rs");
        let hash = format!("hf{i}");
        let mut input = base(&path, Some("rust"), &lines, &hash);
        input.symbols = &symbols;
        store.upsert_file(input).unwrap();
    }

    let resp = expand_chain(
        &store,
        "hop_target",
        &ChainConfig {
            max_depth: 2,
            decay_factor: 0.5,
            limit: 2,
            top_n: 8,
        },
    )
    .unwrap();
    assert_eq!(resp.edge_count, resp.edges.len());
    let node_files: HashSet<_> = resp.nodes.iter().map(|n| n.file.as_str()).collect();
    for e in &resp.edges {
        assert!(
            node_files.contains(e.from_file.as_str()) && node_files.contains(e.to_file.as_str()),
            "2hhq: orphan edge {:?}->{:?} vs nodes {:?}",
            e.from_file,
            e.to_file,
            node_files
        );
    }
}

/// 50hx — quoted hybrid Literal intent must hit the same line as literal:…
#[test]
fn bead_50hx_hybrid_quoted_runs_literal_pass() {
    let temp = TempDir::new().unwrap();
    write_src(
        temp.path(),
        "lib.rs",
        "fn main() {\n    let msg = \"unique_literal_needle_xyzz\";\n}\n",
    );
    let mut indexer = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(temp.path().join("index.db")),
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    let searcher = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(temp.path().join("index.db")),
        limit: 20,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    let needle = "unique_literal_needle_xyzz";
    let lit = searcher.search(&format!("literal:{needle}")).unwrap();
    let quoted = searcher.search(&format!("\"{needle}\"")).unwrap();
    assert!(
        !lit.hits.is_empty(),
        "literal: must find needle; got {:?}",
        lit.hits
    );
    let lit_lines: HashSet<_> = lit
        .hits
        .iter()
        .map(|h| (h.file.as_str(), h.line_start))
        .collect();
    assert!(
        quoted
            .hits
            .iter()
            .any(|h| lit_lines.contains(&(h.file.as_str(), h.line_start))),
        "50hx: quoted hybrid must share a hit line with literal:; quoted={:?} literal={:?}",
        quoted.hits,
        lit.hits
    );
    let parsed = ParsedQuery::parse(&format!("\"{needle}\""));
    assert_eq!(parsed.mode, QueryMode::Hybrid);
    assert_eq!(
        ast_sgrep_core::intent::classify(&parsed),
        ast_sgrep_core::intent::QueryIntent::Literal
    );
}

/// ql1u — hit_symbol must not invent seeds via first_symbol_in_file.
#[test]
fn bead_ql1u_chain_seed_skips_first_symbol_invention() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    // File with an unrelated top symbol and a later matching line without symbol/callee.
    let symbols = [
        SymbolRow {
            name: "unrelated_top".into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 1,
            byte_start: 0,
            byte_end: 13,
        },
        SymbolRow {
            name: "real_match".into(),
            kind: "function".into(),
            line_start: 5,
            line_end: 5,
            byte_start: 0,
            byte_end: 10,
        },
    ];
    let lines = [
        (1u32, "fn unrelated_top() {}".into()),
        (2u32, "// padding".into()),
        (3u32, "// padding".into()),
        (4u32, "// padding".into()),
        (5u32, "fn real_match() { /* real_match marker */ }".into()),
    ];
    let mut input = base("mixed.rs", Some("rust"), &lines, "hmix");
    input.symbols = &symbols;
    store.upsert_file(input).unwrap();

    let resp = expand_chain(
        &store,
        "real_match",
        &ChainConfig {
            max_depth: 1,
            decay_factor: 0.5,
            limit: 20,
            top_n: 8,
        },
    )
    .unwrap();
    for seed in &resp.seeds {
        assert_ne!(
            seed.symbol.as_deref(),
            Some("unrelated_top"),
            "ql1u: must not invent first_symbol_in_file as seed; seeds={:?}",
            resp.seeds
        );
    }
}

/// firi — IVF (all probes) and flat share MIN_SIMILARITY via exceeds_threshold.
#[test]
fn bead_firi_ivf_and_flat_min_similarity_agree() {
    let dim = 16usize;
    let n = 256usize;
    let mut flat = Vec::with_capacity(n * dim);
    let mut state = 0x00F1_0091_u64;
    for _ in 0..n {
        let start = flat.len();
        for _ in 0..dim {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            flat.push((((state >> 32) as u32) as f32 / u32::MAX as f32) * 2.0 - 1.0);
        }
        let norm: f32 = flat[start..start + dim]
            .iter()
            .map(|x| x * x)
            .sum::<f32>()
            .sqrt();
        if norm > 0.0 {
            for x in &mut flat[start..start + dim] {
                *x /= norm;
            }
        }
    }
    let index = SemanticAnnIndex::build_from_flat(&flat, dim);
    let limit = 16usize;
    for &qi in &[0usize, 41, 128, 200, 255] {
        let query = flat[qi * dim..(qi + 1) * dim].to_vec();
        let mut qn = query.clone();
        let qnorm: f32 = qn.iter().map(|x| x * x).sum::<f32>().sqrt();
        if qnorm > 0.0 {
            for x in &mut qn {
                *x /= qnorm;
            }
        }
        let flat_hits: HashSet<usize> =
            top_k_flat_similarity(&qn, &flat, dim, limit, Some(MIN_SIMILARITY))
                .into_iter()
                .map(|(i, _)| i)
                .collect();
        let ivf_hits: HashSet<usize> = index
            .search_flat_with_probes(&flat, dim, &query, limit, Some(usize::MAX))
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            ivf_hits, flat_hits,
            "firi: IVF vs flat hit sets diverge at query {qi}"
        );
    }
}

/// 6dx9 — hybrid search returns hits on both small and large corpora; both
/// sides of the tantivy-1000 threshold are exercised. The parallel-pass gate
/// concept (128 files) is a historical constant kept as a corpus size here.
#[test]
fn bead_6dx9_threshold_sides_differentially_exercised() {
    const PARALLEL_PASS_FILE_THRESHOLD: usize = 128;
    assert_eq!(TANTIVY_AUTO_THRESHOLD, 1000);
    assert!(!should_use_tantivy(TANTIVY_AUTO_THRESHOLD - 1, false));
    assert!(should_use_tantivy(TANTIVY_AUTO_THRESHOLD, false));
    assert!(should_use_tantivy(1, true));

    // Serial side (<128 files): HitKey set for a fixture query.
    let temp_small = TempDir::new().unwrap();
    for i in 0..10 {
        write_src(
            temp_small.path(),
            &format!("f{i}.rs"),
            &format!("fn process_request_{i}() {{ let _ = {i}; }}\n"),
        );
    }
    write_src(
        temp_small.path(),
        "target.rs",
        "fn process_request() { /* marker */ }\n",
    );
    let mut idx_small = Indexer::new(IndexOptions {
        root: temp_small.path().to_path_buf(),
        index_path: Some(temp_small.path().join("i.db")),
        ..IndexOptions::default()
    })
    .unwrap();
    idx_small.index_all().unwrap();
    let status_small = idx_small.store().status().unwrap();
    assert!(
        status_small.file_count < PARALLEL_PASS_FILE_THRESHOLD,
        "serial side needs file_count < {PARALLEL_PASS_FILE_THRESHOLD}"
    );
    let serial_keys = response_hit_keys(
        &Searcher::new(SearchOptions {
            root: temp_small.path().to_path_buf(),
            index_path: Some(temp_small.path().join("i.db")),
            limit: 10,
            use_embed: false,
            ..SearchOptions::default()
        })
        .unwrap()
        .search("process_request")
        .unwrap(),
    );
    assert!(!serial_keys.is_empty(), "serial hybrid must return hits");

    // Parallel side (>=128 files): same query shape; HitKeys non-empty and include target.
    let temp_big = TempDir::new().unwrap();
    for i in 0..PARALLEL_PASS_FILE_THRESHOLD {
        write_src(
            temp_big.path(),
            &format!("p{i}.rs"),
            &format!("fn filler_{i}() {{}}\n"),
        );
    }
    write_src(
        temp_big.path(),
        "target.rs",
        "fn process_request() { /* marker */ }\n",
    );
    let mut idx_big = Indexer::new(IndexOptions {
        root: temp_big.path().to_path_buf(),
        index_path: Some(temp_big.path().join("i.db")),
        ..IndexOptions::default()
    })
    .unwrap();
    idx_big.index_all().unwrap();
    let status_big = idx_big.store().status().unwrap();
    assert!(
        status_big.file_count >= PARALLEL_PASS_FILE_THRESHOLD,
        "parallel side needs file_count >= {PARALLEL_PASS_FILE_THRESHOLD}, got {}",
        status_big.file_count
    );
    let parallel_keys = response_hit_keys(
        &Searcher::new(SearchOptions {
            root: temp_big.path().to_path_buf(),
            index_path: Some(temp_big.path().join("i.db")),
            limit: 10,
            use_embed: false,
            ..SearchOptions::default()
        })
        .unwrap()
        .search("process_request")
        .unwrap(),
    );
    assert!(
        parallel_keys.iter().any(|k| k.file.ends_with("target.rs")),
        "parallel hybrid must find process_request; keys={parallel_keys:?}"
    );

    // Tantivy force-on vs force-off at small corpus: HitKey equivalence (or documented empty→FTS).
    let tantivy_off = Searcher::new(SearchOptions {
        root: temp_small.path().to_path_buf(),
        index_path: Some(temp_small.path().join("i.db")),
        limit: 10,
        use_embed: false,
        use_tantivy: false,
        ..SearchOptions::default()
    })
    .unwrap()
    .search("process_request")
    .unwrap();
    let tantivy_on = Searcher::new(SearchOptions {
        root: temp_small.path().to_path_buf(),
        index_path: Some(temp_small.path().join("i.db")),
        limit: 10,
        use_embed: false,
        use_tantivy: true, // no ready sidecar → falls through to SQL FTS
        ..SearchOptions::default()
    })
    .unwrap()
    .search("process_request")
    .unwrap();
    // Tantivy force-on vs force-off at small corpus: same HitKey *set*
    // (order may differ when sidecar path no-ops to FTS — documented delta).
    let off_keys: HashSet<_> = response_hit_keys(&tantivy_off).into_iter().collect();
    let on_keys: HashSet<_> = response_hit_keys(&tantivy_on).into_iter().collect();
    assert_eq!(
        off_keys, on_keys,
        "6dx9: forced tantivy without ready sidecar must match FTS HitKey set"
    );
}

#[derive(Debug, Deserialize)]
struct RankingCases {
    cases: Vec<RankingCase>,
}
#[derive(Debug, Deserialize)]
struct RankingCase {
    name: String,
    query: String,
    top_k: usize,
    must_include: Vec<MustInclude>,
}
#[derive(Debug, Deserialize)]
struct MustInclude {
    kind: String,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    callee: Option<String>,
    #[serde(default)]
    file: Option<String>,
    max_rank: usize,
}

/// vwga — wire ranking/cases.json as CI self-oracle on the sample fixture.
#[test]
fn bead_vwga_ranking_cases_json_self_oracle() {
    let cases_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/ranking/cases.json");
    let raw = fs::read_to_string(&cases_path).expect("cases.json");
    let fixture: RankingCases = serde_json::from_str(&raw).expect("parse cases.json");
    let indexed = index_sample(IndexOptions {
        root: sample_root(),
        ..IndexOptions::default()
    });
    let searcher = searcher_from(
        &indexed,
        SearchOptions {
            limit: 32,
            // Ranking oracle cases that need embed are soft-skipped when absent;
            // lexical/graph cases must pass with embed off for CI stability.
            use_embed: false,
            ..SearchOptions::default()
        },
    );
    for case in &fixture.cases {
        let resp = searcher
            .search(&case.query)
            .unwrap_or_else(|e| panic!("vwga search failed for {}: {e}", case.name));
        for req in &case.must_include {
            // Embed-only synonym may be empty without a live embed backend — soft-skip.
            if req.kind == "embed" && resp.hits.iter().all(|h| h.kind.as_str() != "embed") {
                eprintln!(
                    "vwga: soft-skip {} (no embed hits; backend unavailable)",
                    case.name
                );
                continue;
            }
            // Prefixed modes: rank in the global top_k window.
            // Hybrid/NL: rank among same-kind hits so multi-lang graph/anchor
            // channels cannot falsely fail a def/embed oracle (vwga harden).
            let prefixed = case.query.contains(':');
            let ranked: Vec<_> = if prefixed {
                resp.hits.iter().take(case.top_k).collect()
            } else {
                resp.hits
                    .iter()
                    .filter(|h| h.kind.as_str() == req.kind)
                    .take(case.top_k)
                    .collect()
            };
            let found = ranked.iter().enumerate().find(|(_, h)| {
                if h.kind.as_str() != req.kind {
                    return false;
                }
                if let Some(sym) = req.symbol.as_deref() {
                    if h.symbol.as_deref() != Some(sym) {
                        return false;
                    }
                }
                if let Some(cal) = req.callee.as_deref() {
                    if h.callee.as_deref() != Some(cal) {
                        return false;
                    }
                }
                if let Some(file) = req.file.as_deref() {
                    if !h.file.ends_with(file) {
                        return false;
                    }
                }
                true
            });
            let Some((rank0, _)) = found else {
                panic!(
                    "vwga: case {} missing {:?} within top_k={}; ranked={:?}",
                    case.name,
                    req,
                    case.top_k,
                    ranked
                        .iter()
                        .map(|h| (
                            h.kind.as_str(),
                            h.symbol.as_deref(),
                            h.callee.as_deref(),
                            &h.file
                        ))
                        .collect::<Vec<_>>()
                );
            };
            assert!(
                rank0 < req.max_rank,
                "vwga: case {} hit at rank {} exceeds max_rank {}",
                case.name,
                rank0 + 1,
                req.max_rank
            );
        }
    }
}
