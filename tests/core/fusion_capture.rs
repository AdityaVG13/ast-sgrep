//! INTENT: the `ASGREP_FUSION_DUMP` capture contract — the diagnostic seam
//! the conformance ledger's retry predicates demand ("a capture of pre-fusion
//! per-candidate channel ranks ... for this frozen 1,100"). One JSONL line
//! per `apply_weighted_rrf` invocation, appended to the env-named path,
//! holding every fused result key with its per-channel ranks, final fused
//! score, and semantic magnitude. Unset env → no file, no allocation on the
//! query path beyond one `var_os` read.
//!
//! KILLS: no-dump-when-unset is untestable here without process isolation
//! (env is process-global and numerical_fusion exercises the same function
//! in parallel), so this test pins the POSITIVE contract only: dump line
//! exists, joins on result key, and carries channel ranks that distinguish
//! an embed-only result from a corroborated one. A mutant that drops the
//! ranks object, mis-names a channel, or skips the write fails here.

use ast_sgrep_core::fusion::apply_weighted_rrf;
use ast_sgrep_core::intent::ChannelWeights;
use ast_sgrep_core::search::HitKind;
use ast_sgrep_testkit::mk_hit;

#[test]
fn fusion_dump_records_prefusion_channel_ranks_per_result() {
    let path =
        std::env::temp_dir().join(format!("asgrep-fusion-dump-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&path);

    // Two independent result keys: src/a.py corroborated by lexical+def on
    // the same line, src/b.py found only by the embed channel. A third
    // lexical-only result makes the lexical rank column non-degenerate.
    let mut hits = vec![
        mk_hit(HitKind::Asgrep, "dumpcap/a.py", 10, 0.9),
        mk_hit(HitKind::Def, "dumpcap/a.py", 10, 0.8),
        mk_hit(HitKind::Embed, "dumpcap/b.py", 20, 0.7),
        mk_hit(HitKind::Asgrep, "dumpcap/c.py", 5, 0.5),
    ];

    // Parallel tests (numerical_fusion) call the same function; a concurrent
    // invocation may append its own line while the env is set. Filtering by
    // our unique file names makes the assertion robust to that interleaving.
    std::env::set_var("ASGREP_FUSION_DUMP", &path);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        apply_weighted_rrf(&mut hits, &ChannelWeights::default());
    }));
    std::env::remove_var("ASGREP_FUSION_DUMP");
    result.expect("fusion under capture must not fail");

    let text = std::fs::read_to_string(&path).expect("dump file written when env is set");
    let _ = std::fs::remove_file(&path);
    let line = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .find(|l| l.contains("dumpcap/b.py"))
        .expect("capture line containing the embed-only result");
    let value: serde_json::Value =
        serde_json::from_str(line).expect("capture line is one JSON object");

    assert_eq!(
        value["input_hits"].as_u64(),
        Some(4),
        "input hit count recorded (pre-merge lanes)"
    );
    let results = value["results"].as_array().expect("results array");
    let embed_only = results
        .iter()
        .find(|r| r["file"] == "dumpcap/b.py")
        .expect("embed-only result present in capture");
    assert_eq!(
        embed_only["ranks"]["semantic"].as_u64(),
        Some(0),
        "embed channel ranks the sole semantic member 0"
    );
    assert_eq!(
        embed_only["ranks"]["lexical"],
        serde_json::Value::Null,
        "absent channels serialize as null, not 0"
    );
    let corroborated = results
        .iter()
        .find(|r| r["file"] == "dumpcap/a.py")
        .expect("corroborated result present in capture");
    assert_eq!(
        corroborated["ranks"]["lexical"].as_u64(),
        Some(0),
        "lexical rank 0 for the strongest lexical result"
    );
    assert_eq!(
        corroborated["ranks"]["definition"].as_u64(),
        Some(0),
        "definition rank recorded beside lexical"
    );
    assert!(
        corroborated["kinds"]
            .as_array()
            .is_some_and(|k| k.len() == 2),
        "contributor kinds recorded for merged same-line results"
    );
    assert!(
        embed_only["score"]
            .as_f64()
            .is_some_and(|s| s.is_finite() && s > 0.0),
        "final fused score recorded"
    );
}
