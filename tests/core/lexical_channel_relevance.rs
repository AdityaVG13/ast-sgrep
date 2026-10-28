//! INTENT: the hybrid lexical channel must rank its line hits by content
//! relevance over the query's terms, not by the cascade's discovery order.
//!
//! Failure this pins (measured 2026-10-25 on flask/ripgrep foreign corpora,
//! semgrep-lane harness): `literal_prefilter_pass` keeps ONE discovery hit
//! per file — the first line where the rarest term that found the file
//! appears, emitted in `ORDER BY f.path` order with uniform scores. Fusion's
//! deterministic tie-break is path-ascending, so a file whose ONLY match is
//! one common query term (an `examples/*/__init__.py` saying the project
//! name) occupies lexical rank 0 and outranks files matching three and four
//! query terms deeper in the alphabet. The lexical channel's head is an
//! alphabetical lottery on real-world repo shapes.
//!
//! This test runs with `use_embed: false`. With the embed channel on, a tiny
//! fixture's junk keys are all absorbed into embed-primary rows by the
//! evidence merge, masking the channel order (that merge rescue cannot scale
//! to real corpora — on flask the junk keys survive embed-free). Embed-off
//! exposes the lexical channel's own ordering, which is the contract under
//! test.
//!
//! Contract: within one hybrid query, the file whose retained line matches
//! MORE of the query's distinctive terms owns the earlier lexical channel
//! rank than a file matching only the shared common term.
//!
//! KILLS:
//! - discovery-order-as-rank mutant (no bm25 re-rank of the channel): the
//!   aa_examples junk files keep lexical ranks 0..2;
//! - tie-break-only mutant (reordering without score differentiation):
//!   junk ranks can still interleave with the target's rank;
//! - channel-swap mutant (rerank over the wrong term set — borrowed/
//!   discovery extras instead of the user's terms): junk-only terms leak
//!   into the OR-query and the junk files come back.
use ast_sgrep_core::{IndexOptions, SearchOptions, Searcher};
use std::fs;
use tempfile::TempDir;

fn write_src(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

fn corpus() -> (TempDir, Searcher) {
    let temp = TempDir::new().unwrap();
    // Alphabetically-early junk: matches ONLY the shared term `turbo`. Six
    // files so the embed channel cannot absorb every junk (file,line) key
    // into an embed-primary row — at least one junk line survives the
    // evidence merge as an asgrep-primary and owns a lexical rank (the
    // flask-corpus condition: `examples/*/__init__.py` matching only the
    // project-name term).
    for name in ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"] {
        write_src(
            temp.path(),
            &format!("aa_examples/{name}.py"),
            &format!("import turbo\n\ndef {name}_notes():\n    turbo.write(\"{name} notes\")\n"),
        );
    }
    // Alphabetically-late target: matches turbo + ignite + fusion + coil.
    write_src(
        temp.path(),
        "zz_app/coil.rs",
        r#"
/// Ignite the fusion coil with turbo assist.
pub fn ignite_fusion_coil(power: u32) -> u32 {
    let turbo = power.saturating_mul(2);
    turbo + coil_bias()
}

fn coil_bias() -> u32 {
    3
}
"#,
    );
    ast_sgrep_core::Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        force_reindex: true,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");
    let searcher = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        limit: 12,
        use_embed: false,
        ..SearchOptions::default()
    })
    .expect("searcher");
    (temp, searcher)
}

#[test]
fn lexical_channel_ranks_multiterm_match_above_common_term_junk() {
    let (_temp, searcher) = corpus();
    let dump = std::env::temp_dir().join(format!(
        "asgrep-lexrelev-{}-{}.jsonl",
        std::process::id(),
        line!()
    ));
    let _ = fs::remove_file(&dump);
    std::env::set_var("ASGREP_FUSION_DUMP", &dump);
    let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        searcher
            .search("where does the turbo ignite the fusion coil")
            .expect("hybrid search")
    }))
    .expect("search under capture must not fail");
    std::env::remove_var("ASGREP_FUSION_DUMP");

    // Channel-level contract: read the capture lines' lexical ranks for our
    // unique fixture paths (filtering by path makes this robust to
    // concurrent tests appending their own lines while the env is set).
    let text = fs::read_to_string(&dump).expect("dump written under capture");
    let _ = fs::remove_file(&dump);
    let mut junk_best: Option<u64> = None;
    let mut target_rank: Option<u64> = None;
    for line in text.lines().filter(|l| l.contains("zz_app/coil.rs")) {
        let value: serde_json::Value = serde_json::from_str(line).expect("dump line is JSON");
        for result in value["results"].as_array().expect("results array") {
            let file = result["file"].as_str().unwrap_or_default();
            let lexical = result["ranks"]["lexical"].as_u64();
            if file.starts_with("aa_examples/") {
                if let Some(rank) = lexical {
                    junk_best = Some(junk_best.map_or(rank, |best: u64| best.min(rank)));
                }
            } else if file == "zz_app/coil.rs" {
                if let Some(rank) = lexical {
                    target_rank = Some(target_rank.map_or(rank, |best: u64| best.min(rank)));
                }
            }
        }
    }
    // Precondition: with the embed channel off, single-term files must own
    // lexical ranks. A None here means the fixture stopped reproducing the
    // lottery (vacuous pass) — fail loudly, do not fall back to MAX.
    let Some(junk_best) = junk_best else {
        panic!("fixture regression: junk files own no lexical ranks; the test no longer reproduces the alphabetical lottery");
    };
    let target_rank = target_rank.expect("target has a lexical rank in the capture");
    assert!(
        target_rank < junk_best,
        "multi-term target must own the earlier lexical rank: target={target_rank} junk={junk_best}"
    );

    // User-level contract: the first distinct file of the response is the
    // multi-term target, not a common-term junk file.
    let first_file = response
        .hits
        .first()
        .map(|hit| hit.file.as_str())
        .expect("response has hits");
    assert_eq!(
        first_file, "zz_app/coil.rs",
        "first distinct file must be the multi-term target, got {first_file}"
    );
}
