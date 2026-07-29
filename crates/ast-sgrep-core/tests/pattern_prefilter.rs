use ast_sgrep_core::pattern::profile_pattern_search;
use std::fs;

#[test]
fn literal_prefilter_skips_noncandidate_files() {
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
