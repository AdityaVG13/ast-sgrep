use ast_sgrep_core::pattern::profile_pattern_search;
use std::fs;

#[test]
fn literal_prefilter_skips_noncandidate_parses_and_reports_work_span() {
    let corpus = tempfile::tempdir().unwrap();
    for index in 0..64 {
        fs::write(
            corpus.path().join(format!("irrelevant_{index}.rs")),
            format!("fn irrelevant_{index}() {{}}\n"),
        )
        .unwrap();
    }
    fs::write(
        corpus.path().join("needle.rs"),
        "fn Needle(value: usize) -> usize { value }\nfn caller() { let _ = Needle(1); }\n",
    )
    .unwrap();

    let profile = profile_pattern_search("Needle($$$ARGS)", corpus.path(), Some("rust")).unwrap();
    assert_eq!(profile.files_considered, 65);
    assert_eq!(profile.files_prefiltered, 64);
    assert_eq!(profile.files_parsed, 1);
    assert_eq!(profile.hits, 1);
    assert!(profile.bytes_scanned > 0);
    assert!(profile.workers > 0);
    assert!(profile.t1_ns >= profile.t_inf_ns);
    assert!(profile.t1_ns >= profile.parse_match_work_ns);
    assert!(profile.brent_upper_bound_ns >= profile.t_inf_ns);
    assert!((0.0..=1.0).contains(&profile.serial_fraction));
}

#[test]
fn metavariable_only_pattern_disables_prefilter_without_losing_matches() {
    let corpus = tempfile::tempdir().unwrap();
    fs::write(
        corpus.path().join("calls.rs"),
        "fn first() { second(); }\nfn second() {}\n",
    )
    .unwrap();

    let profile = profile_pattern_search("$FUNC($$$ARGS)", corpus.path(), Some("rust")).unwrap();
    assert_eq!(profile.files_considered, 1);
    assert_eq!(profile.files_prefiltered, 0);
    assert_eq!(profile.files_parsed, 1);
    assert!(profile.hits > 0);
}

#[test]
fn declaration_keyword_is_not_a_cross_language_required_literal() {
    let corpus = tempfile::tempdir().unwrap();
    fs::write(
        corpus.path().join("foreign.js"),
        "export function foreignName() {}\n",
    )
    .unwrap();

    let profile =
        profile_pattern_search("fn $NAME($$$ARGS)", corpus.path(), Some("javascript")).unwrap();
    assert_eq!(profile.files_considered, 1);
    assert_eq!(profile.files_prefiltered, 0);
    assert_eq!(profile.files_parsed, 1);
    assert_eq!(profile.hits, 1);
}

#[test]
#[ignore = "requires ASGREP_PATTERN_PROFILE_FIXTURE repository"]
fn real_repository_work_span_profile() {
    let root = std::env::var_os("ASGREP_PATTERN_PROFILE_FIXTURE")
        .map(std::path::PathBuf::from)
        .expect("ASGREP_PATTERN_PROFILE_FIXTURE must name a repository");
    let pattern = std::env::var("ASGREP_PATTERN_PROFILE_PATTERN")
        .unwrap_or_else(|_| "Searcher::new($$$ARGS)".to_string());
    let profiles = (0..5)
        .map(|_| profile_pattern_search(&pattern, &root, Some("rust")).unwrap())
        .collect::<Vec<_>>();
    eprintln!("{}", serde_json::to_string_pretty(&profiles).unwrap());
    for profile in profiles {
        assert!(profile.files_considered >= 30);
        assert!(profile.files_prefiltered > 0);
        assert!(profile.files_parsed > 0);
        assert!(profile.hits > 0);
        assert!(profile.t1_ns >= profile.t_inf_ns);
        assert!(profile.prefilter_disabled_t1_ns > 0);
        assert!(profile.parallel_span_ns > 0);
        assert!(profile.observed_speedup.is_finite());
        assert!(profile.prefilter_speedup.is_finite());
    }
}
