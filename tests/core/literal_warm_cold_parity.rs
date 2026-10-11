//! One-shot `literal:` must not pay the RAM-corpus load; cold and warmed
//! searchers must agree (hit-for-hit on the literal lane, file-sets on
//! hybrid — see C2/C3 for the exact contracts).
//!
//! Flamegraph (2026-09-20, 3/3 captures): ~95% of one-shot `literal:SearchHit`
//! wall sat under `literal_pass` -> `IndexStore::line_corpus` — a full `lines`
//! table materialization plus per-line token indexing. The corpus pays off
//! across queries in a warmed session (codemode-serve); a single query must
//! take the trigram/SQL path instead.
//!
//! Contracts:
//! C1 no-implicit-load — a cold `Searcher::search("literal:...")` leaves no
//!    resident corpus behind; only `warm_search_path` loads one.
//! C2 warm/cold parity — for under-cap queries, cold (trigram) and warmed
//!    (corpus) literal hits agree on file, line, score bits, and excerpt.
//!    (Over-cap candidate sets may legitimately differ: posting-order cap vs
//!    scan-order cap. All fixtures here stay under cap.)
//! C3 hybrid parity — the hybrid prefilter fallback (cold) and the corpus
//!    distinct-files scan (warm) must agree hit-for-hit on under-cap
//!    unprefixed queries: same files, same representative lines, same
//!    scores. The warm token lookup locates the actual first matching line
//!    per file via bounded memchr (`hits_from_file_ids`), not a first-line
//!    stub: a stub merges differently downstream and surfaces non-matching
//!    excerpts as evidence.
//! C4 mixed-kind symbol parity — over-budget symbol queries with minority
//!    kinds must agree too: both paths sort by (kind, path, line, name) and
//!    apply the same quota buckets (functions plus reserved type/other
//!    slots), so the cold SQL window and the warmed table select the same
//!    rows. (A rowid-ordered head-cut with no quotas would crowd minority
//!    kinds out of the cold page while the warmed page reserves them slots.)
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use std::fs;
use tempfile::TempDir;

// 20 files x (28 fill defs + 3 qzz fns) x 2 lines + 5 classes x 2 lines +
// 4 marker lines = 1254 indexed lines: above BMH_LINE_THRESHOLD (1000) so
// the cold path is the trigram scan, not literal_sql (except the
// short-needle query, which pins literal_sql).
const FILLER_FILES: usize = 20;
const FILLER_DEFS: usize = 28;

fn write_src(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

fn setup() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    for f in 0..FILLER_FILES {
        let mut body = String::new();
        for i in 0..FILLER_DEFS {
            body.push_str(&format!(
                "def fill_{f}_{i}(value):\n    return value * {i} + {f}\n"
            ));
        }
        // C4 vehicles: 60 single-kind functions plus 5 minority-kind classes
        // sharing one rare term, so quota bucketing (not a head-cut) decides
        // the over-budget symbol page.
        for k in 0..3 {
            body.push_str(&format!("def qzz_f{f}_{k}(value):\n    return value\n"));
        }
        if f < 5 {
            body.push_str(&format!("class qzz_C{f}:\n    pass\n"));
        }
        if f == 0 {
            body.push_str("ALPHA_ZZQUUX_MARKER_PAYLOAD sentinel\n");
        }
        if f < 3 {
            body.push_str("beta_shared_rare_token payload\n");
        }
        write_src(root, &format!("src/mod_{f}.py"), &body);
    }
    let index_path = root.join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(index_path),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    temp
}

fn searcher_for(root: &std::path::Path) -> Searcher {
    Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(root.join("index.db")),
        limit: 50,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap()
}

fn hit_key(hit: &ast_sgrep_core::SearchHit) -> (String, u32, u64, String) {
    (
        hit.file.clone(),
        hit.line_start,
        hit.score.to_bits(),
        hit.excerpt.clone(),
    )
}

#[test]
fn c1_cold_literal_search_leaves_no_resident_corpus() {
    let temp = setup();
    let searcher = searcher_for(temp.path());
    assert!(
        !searcher.store().has_resident_line_corpus(),
        "fresh searcher must start without a resident corpus"
    );
    let resp = searcher.search("literal:ZZQUUX").unwrap();
    assert!(
        !resp.hits.is_empty(),
        "fixture must contain the needle; got no hits"
    );
    assert!(
        !searcher.store().has_resident_line_corpus(),
        "one-shot literal must take the trigram/SQL path, not load the RAM corpus"
    );
    searcher.warm_search_path().unwrap();
    assert!(
        searcher.store().has_resident_line_corpus(),
        "explicit warm must still residently load the corpus for the session"
    );
}

#[test]
fn c2_cold_trigram_and_warmed_corpus_literal_agree() {
    let temp = setup();
    let root = temp.path();
    let cold = searcher_for(root);
    let warmed = searcher_for(root);
    warmed.warm_search_path().unwrap();
    assert!(
        warmed.store().has_resident_line_corpus(),
        "warmed searcher must actually hold the corpus for this comparison"
    );
    for query in [
        "literal:ZZQUUX",
        "literal:beta_shared_rare_token",
        "literal:fill_7_13",
        "literal:ZZ", // short needle: cold takes literal_sql, warm takes corpus
        "word:sentinel",
        "literal:valeur_absente",
    ] {
        let cold_hits: Vec<_> = cold
            .search(query)
            .unwrap()
            .hits
            .iter()
            .map(hit_key)
            .collect();
        let warm_hits: Vec<_> = warmed
            .search(query)
            .unwrap()
            .hits
            .iter()
            .map(hit_key)
            .collect();
        if query != "literal:valeur_absente" {
            assert!(
                !cold_hits.is_empty(),
                "fixture must contain {query}; both-empty agreement proves nothing"
            );
        }
        assert_eq!(cold_hits, warm_hits, "cold/warm hit divergence for {query}");
    }
}

#[test]
fn c3_cold_and_warmed_hybrid_identifier_agree() {
    let temp = setup();
    let root = temp.path();
    let cold = searcher_for(root);
    let warmed = searcher_for(root);
    warmed.warm_search_path().unwrap();
    for query in ["ZZQUUX", "beta_shared_rare_token"] {
        let cold_hits: Vec<_> = cold
            .search(query)
            .unwrap()
            .hits
            .iter()
            .map(hit_key)
            .collect();
        assert!(
            !cold_hits.is_empty(),
            "fixture must answer hybrid {query}; both-empty agreement proves nothing"
        );
        let warm_hits: Vec<_> = warmed
            .search(query)
            .unwrap()
            .hits
            .iter()
            .map(hit_key)
            .collect();
        assert_eq!(
            cold_hits, warm_hits,
            "cold/warm hybrid divergence for {query}"
        );
    }
}

fn discovery_response(
    session: &ast_sgrep_testkit::IsolatedIndexSession,
    query: &str,
    warm: bool,
) -> ast_sgrep_core::SearchResponse {
    let searcher = session
        .searcher(SearchOptions {
            limit: 200,
            use_embed: false,
            use_tantivy: false,
            use_repository_vocabulary: false,
            ..session.search_options()
        })
        .with_response_cache(false)
        .with_response_stamp(false);
    if warm {
        searcher.warm_search_path().unwrap();
    }
    let response = searcher.search(query).unwrap();
    assert_eq!(
        searcher.store().has_resident_line_corpus(),
        warm,
        "cold discovery must not load the corpus"
    );
    assert!(
        response
            .hits
            .iter()
            .all(|hit| hit.kind == ast_sgrep_core::HitKind::Asgrep
                && hit
                    .contributors
                    .iter()
                    .all(|kind| *kind == ast_sgrep_core::HitKind::Asgrep)),
        "this fixture must exercise only discovery, not rescuing siblings"
    );
    response
}

#[test]
fn hybrid_discovery_retains_ci_residuals_and_first_matching_lines() {
    for (query, files, expected) in [
        (
            "qzz qyy",
            vec![
                ("a.rs", "// unrelated\n// qzz\n"),
                ("b.rs", "// unrelated\n// aqzzb\n"),
            ],
            vec![("a.rs", 2), ("b.rs", 2)],
        ),
        (
            "qzz qyy",
            vec![("b.rs", "// unrelated\n// AQZZB\n")],
            vec![("b.rs", 2)],
        ),
        (
            "kqzz qyy",
            vec![("b.rs", "// unrelated\n// KQZZ\n// kqzz\n")],
            vec![("b.rs", 2)],
        ),
        (
            "äzzz qyy",
            vec![("b.rs", "// unrelated\n// ÄZZZ\n")],
            vec![("b.rs", 2)],
        ),
    ] {
        let session = ast_sgrep_testkit::isolated_index_session();
        for (path, body) in files {
            session.write(path, body);
        }
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        for warm in [false, true] {
            let response = discovery_response(&session, query, warm);
            let got: std::collections::BTreeSet<_> = response
                .hits
                .iter()
                .map(|hit| (hit.file.as_str(), hit.line_start))
                .collect();
            assert_eq!(
                got,
                expected.iter().copied().collect(),
                "CI residual/representative: {query}, warm={warm}"
            );
        }
    }
}

#[test]
fn hybrid_discovery_counts_new_files_per_term_without_mixed_script_replay() {
    use std::collections::BTreeSet;
    for case in ["flood", "overlap", "mixed"] {
        let session = ast_sgrep_testkit::isolated_index_session();
        let mut expected = BTreeSet::new();
        let shared = if case == "overlap" { 32 } else { 100 };
        for n in 0..shared {
            let body = match case {
                "flood" => "// qaaa\n",
                "overlap" => "// qaaa qbbb\n// qaaa qbbb\n",
                _ => "// qaaa qbbb\n",
            };
            session.write(format!("a{n:03}.rs"), body);
            if n < if case == "mixed" { 64 } else { 32 } {
                expected.insert(format!("a{n:03}.rs"));
            }
        }
        let query = match case {
            "flood" => {
                // Both qbbb trigrams have df 102, so warm rarity does not
                // move the later term first and mask first-term starvation.
                session.write("padding.rs", "// qbb bbb\n".repeat(101));
                session.write("z_target.rs", "// qbbb\n");
                expected.insert("z_target.rs".into());
                "qaaa qbbb"
            }
            "overlap" => {
                for n in 0..64 {
                    session.write(format!("z{n:03}.rs"), "// qbbb\n");
                    if n < 32 {
                        expected.insert(format!("z{n:03}.rs"));
                    }
                }
                "qaaa qbbb"
            }
            _ => {
                session.write("z_unicode.rs", "// ÄZZZ\n");
                expected.insert("z_unicode.rs".into());
                "qaaa qbbb äzzz"
            }
        };
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        for warm in [false, true] {
            let response = discovery_response(&session, query, warm);
            let got: BTreeSet<_> = response.hits.iter().map(|hit| hit.file.clone()).collect();
            assert_eq!(got, expected, "new-file term budget: {case}, warm={warm}");
        }
    }
}

#[test]
fn hybrid_discovery_preserves_extra_reserve_and_preferred_files() {
    use std::collections::BTreeSet;
    let session = ast_sgrep_testkit::isolated_index_session();
    for term in ["qaaa", "qbbb", "qccc"] {
        for n in 0..40 {
            session.write(format!("{term}/{n:03}.rs"), format!("// {term}\n"));
        }
    }
    session.write("extras.rs", "// cache embed embedding\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    for warm in [false, true] {
        let response = discovery_response(&session, "qaaa qbbb qccc embeddings", warm);
        assert_eq!(response.hits.len(), 75, "own-term reserve, warm={warm}");
        assert!(
            response.hits.iter().all(|hit| hit.file != "extras.rs"),
            "extras are not user evidence"
        );
        let fallback = discovery_response(
            &session,
            "missingone missingtwo missingthree embeddings",
            warm,
        );
        assert!(
            fallback.hits.iter().any(|hit| hit.file == "extras.rs"),
            "extras remain useful without a user foothold"
        );
    }
    for token in [true, false] {
        let session = ast_sgrep_testkit::isolated_index_session();
        let marker = if token { "qzzmarker" } else { "aqzzmarkerb" };
        for n in 0..33 {
            session.write(format!("tests/a{n:03}.rs"), format!("// {marker}\n"));
        }
        session.write("z.rs", format!("// {marker}\n"));
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        for warm in [false, true] {
            let response = discovery_response(&session, "qzzmarker qzzabsent", warm);
            let files: BTreeSet<_> = response.hits.iter().map(|hit| hit.file.as_str()).collect();
            assert_eq!(
                files.len(),
                32,
                "preferred bounded page, token={token}, warm={warm}"
            );
            assert!(
                files.contains("z.rs"),
                "preferred code must precede nonpreferred candidates"
            );
        }
    }
}

#[test]
fn hybrid_language_filter_precedes_warm_distinct_file_admission() {
    for token_posting in [false, true] {
        for requested_language_present in [false, true] {
            let session = ast_sgrep_testkit::isolated_index_session();
            let marker = if token_posting {
                "qzzmarker qzzother"
            } else {
                "aqzzmarkerb aqzzotherb"
            };
            // More than a warmed term's 32-file window, all path-sorted
            // before the requested-language file. Comments carry no defs.
            for file in 0..40 {
                session.write(format!("a{file:02}.py"), format!("# {marker}\n"));
            }
            let rust = if requested_language_present {
                format!("// {marker}\n")
            } else {
                "// unrelated\n".to_owned()
            };
            session.write("z.rs", rust);
            session.index_all(IndexOptions {
                embed_semantic: false,
                ..session.index_options()
            });
            for lang_filter in [Some("rust"), None] {
                let options = SearchOptions {
                    use_embed: false,
                    lang_filter: lang_filter.map(str::to_owned),
                    limit: 50,
                    ..session.search_options()
                };
                let cold = session.searcher(options.clone());
                let warm = session.searcher(options);
                warm.warm_search_path().unwrap();
                for (lane, searcher) in [("cold", &cold), ("warm", &warm)] {
                    let response = searcher.search("qzzmarker qzzother").unwrap();
                    if lang_filter.is_none() {
                        assert!(!response.hits.is_empty(), "unfiltered {lane} control");
                        continue;
                    }
                    let files: std::collections::BTreeSet<_> =
                        response.hits.iter().map(|hit| hit.file.as_str()).collect();
                    let expected = if requested_language_present {
                        std::collections::BTreeSet::from(["z.rs"])
                    } else {
                        std::collections::BTreeSet::new()
                    };
                    assert_eq!(files, expected,
                        "{lane} language admission, posting={token_posting}, requested={requested_language_present}");
                    assert!(response
                        .hits
                        .iter()
                        .all(|hit| hit.language.as_deref() == Some("rust")));
                }
            }
        }
    }
}

#[test]
fn cold_symbol_quotas_survive_more_than_five_hundred_functions() {
    use ast_sgrep_core::search::passes::symbol::symbol_pass_for_files;
    use ast_sgrep_core::{HitKind, ParsedQuery};
    use std::collections::HashSet;
    for classes in [0usize, 1, 20] {
        let session = ast_sgrep_testkit::isolated_index_session();
        let mut body = String::new();
        for n in 0..501 {
            body.push_str(&format!("def qzz_f{n:04}():\n    return 0\n"));
        }
        for n in 0..classes {
            body.push_str(&format!("class qzz_C{n:04}:\n    \"\"\"π\"\"\"\n"));
        }
        session.write("over.py", &body);
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let options = SearchOptions {
            limit: 32,
            use_embed: false,
            lang_filter: Some("python".into()),
            ..session.search_options()
        };
        let store = session.open_store();
        let raw = symbol_pass_for_files(
            &store,
            &options,
            &ParsedQuery::parse("qzz"),
            &HashSet::from(["over.py".to_owned()]),
            false,
        )
        .unwrap();
        let minorities = classes.min(16);
        assert_eq!(
            raw.len(),
            32,
            "unused minority slots must return to functions"
        );
        let names: HashSet<_> = raw.iter().filter_map(|hit| hit.symbol.as_deref()).collect();
        for n in 0..minorities {
            assert!(
                names.contains(format!("qzz_C{n:04}").as_str()),
                "cold symbol admission lost reserved minority beyond the global window"
            );
        }
        for n in 0..32 - minorities {
            assert!(
                names.contains(format!("qzz_f{n:04}").as_str()),
                "canonical function refill"
            );
        }
        assert!(
            raw.iter()
                .all(|hit| hit.excerpt.is_empty() && hit.byte_span.is_none()),
            "file-restricted producer must remain lazy"
        );
        let cold = session.searcher(options.clone());
        let warm = session.searcher(options);
        warm.warm_search_path().unwrap();
        // Conceptual ingress excludes structural/anchor siblings that could
        // otherwise mask the cold producer's missing type candidates.
        let cold = cold.search("qzz probezz unseenzz").unwrap();
        let warm = warm.search("qzz probezz unseenzz").unwrap();
        assert_eq!(
            cold.hits.iter().map(hit_key).collect::<Vec<_>>(),
            warm.hits.iter().map(hit_key).collect::<Vec<_>>(),
            "cold/warm over-window quota parity"
        );
        if classes > 0 {
            assert!(
                cold.hits.iter().any(|hit| hit.kind == HitKind::Def
                    && hit.symbol.as_deref() == Some("qzz_C0000")
                    && hit.excerpt.contains('π')),
                "public page must expose indexed minority evidence"
            );
        }
        assert_eq!(
            cold.returned_excerpt_bytes,
            cold.hits
                .iter()
                .map(|hit| hit.excerpt.len() as u64)
                .sum::<u64>()
        );
    }
}

#[test]
fn c4_cold_and_warmed_mixed_kind_symbol_query_agree() {
    let temp = setup();
    let root = temp.path();
    let cold = searcher_for(root);
    let warmed = searcher_for(root);
    warmed.warm_search_path().unwrap();
    // 60 functions + 5 classes share `qzz` (over the 50-row symbol budget):
    // both paths must select the same quota-balanced rows.
    let cold_hits: Vec<_> = cold
        .search("qzz")
        .unwrap()
        .hits
        .iter()
        .map(hit_key)
        .collect();
    assert!(
        !cold_hits.is_empty(),
        "fixture must answer hybrid qzz; both-empty agreement proves nothing"
    );
    let warm_hits: Vec<_> = warmed
        .search("qzz")
        .unwrap()
        .hits
        .iter()
        .map(hit_key)
        .collect();
    assert_eq!(
        cold_hits, warm_hits,
        "cold/warm mixed-kind symbol divergence for qzz"
    );
}

#[test]
fn unicode_case_insensitive_literal_matches_in_cold_and_warm_lanes() {
    for filler_lines in [0, 1000] {
        let session = ast_sgrep_testkit::isolated_index_session();
        let body = format!(
            "let value = \"ÄPFEL KELVIN İX\";\n{}",
            "// unrelated filler\n".repeat(filler_lines)
        );
        session.write("unicode.rs", &body);
        session.write("a.py", "# ÄPFEL KELVIN İX\n".repeat(120));
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let options = SearchOptions {
            use_embed: false,
            case_insensitive: true,
            lang_filter: Some("rust".into()),
            limit: 8,
            ..session.search_options()
        };
        let cold = session.searcher(options.clone());
        let warm = session.searcher(options);
        warm.warm_search_path().unwrap();
        for needle in ["äpfel", "kelvin", "i\u{307}x"] {
            for (lane, searcher) in [("cold", &cold), ("warm", &warm)] {
                for (mode, response) in [
                    ("literal", searcher.search_literal(needle).unwrap()),
                    ("word", searcher.search_word(needle).unwrap()),
                ] {
                    let locations: Vec<_> = response
                        .hits
                        .iter()
                        .map(|hit| (hit.file.as_str(), hit.line_start))
                        .collect();
                    assert_eq!(
                        locations,
                        vec![("unicode.rs", 1)],
                        "{lane} CI {mode} {needle}, filler lines {filler_lines}"
                    );
                }
            }
        }
        assert!(
            !cold.store().has_resident_line_corpus(),
            "cold Unicode verification must not materialize the RAM corpus"
        );
    }
}

#[test]
fn literal_and_word_matches_do_not_cross_packed_record_boundaries() {
    for filler_lines in [0, 1000] {
        let session = ast_sgrep_testkit::isolated_index_session();
        session.write("a.rs", "// alpha_end\n// beta_start\n// seam_tail");
        session.write("b.rs", "// seam_head\n");
        session.write("c.rs", "// intact_marker\n");
        session.write("z.rs", "// filler\n".repeat(filler_lines));
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        for case_insensitive in [false, true] {
            let options = SearchOptions {
                use_embed: false,
                case_insensitive,
                ..session.search_options()
            };
            let cold = session.searcher(options.clone());
            let warm = session.searcher(options);
            warm.warm_search_path().unwrap();
            assert!(warm.store().has_resident_line_corpus());
            for (lane, searcher) in [("cold", &cold), ("warm", &warm)] {
                for needle in ["alpha_end\n// beta_start", "seam_tail\n// seam_head"] {
                    for (mode, response) in [
                        ("literal", searcher.search_literal(needle).unwrap()),
                        ("word", searcher.search_word(needle).unwrap()),
                        (
                            "literal prefix",
                            searcher.search(&format!("literal:{needle}")).unwrap(),
                        ),
                        (
                            "word prefix",
                            searcher.search(&format!("word:{needle}")).unwrap(),
                        ),
                    ] {
                        assert!(
                            response.hits.is_empty(),
                            "{lane} {mode} must not invent {needle:?} across records; \
                             CI={case_insensitive}, filler={filler_lines}: {:?}",
                            response.hits
                        );
                    }
                }
                for response in [
                    searcher.search_literal("intact_marker").unwrap(),
                    searcher.search_word("intact_marker").unwrap(),
                ] {
                    let locations: Vec<_> = response
                        .hits
                        .iter()
                        .map(|hit| (hit.file.as_str(), hit.line_start))
                        .collect();
                    assert_eq!(locations, vec![("c.rs", 1)]);
                }
            }
        }
    }
}

#[test]
fn literal_and_word_preserve_bytes_on_both_sides_of_embedded_nul() {
    for filler_lines in [0, 1000] {
        let session = ast_sgrep_testkit::isolated_index_session();
        session.write(
            "n.rs",
            "// before_marker\0after_marker z\n// before_marker\n",
        );
        session.write("a.py", "# before_marker\0after_marker z\n");
        session.write("z.rs", "// filler\n".repeat(filler_lines));
        session.index_all(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        for (lang_filter, expected) in [
            (None, vec![("a.py", 1), ("n.rs", 1)]),
            (Some("rust"), vec![("n.rs", 1)]),
        ] {
            for case_insensitive in [false, true] {
                let options = SearchOptions {
                    use_embed: false,
                    lang_filter: lang_filter.map(str::to_owned),
                    case_insensitive,
                    ..session.search_options()
                };
                let cold = session.searcher(options.clone());
                let warm = session.searcher(options);
                warm.warm_search_path().unwrap();
                for (lane, searcher) in [("cold", &cold), ("warm", &warm)] {
                    // The real FTS-backed regex path confirms indexing retains
                    // text after NUL; ignoring the file would hide this defect.
                    let regex = searcher.search_regex("after_marker").unwrap();
                    let locations: Vec<_> = regex
                        .hits
                        .iter()
                        .map(|hit| (hit.file.as_str(), hit.line_start))
                        .collect();
                    assert_eq!(locations, expected, "regex control, {lane}");
                    for (needle, present) in [
                        ("after_marker", true),
                        ("z", true),
                        ("before_marker\0after_marker", true),
                        ("before_marker\0absent", false),
                    ] {
                        for (mode, response) in [
                            ("literal", searcher.search_literal(needle).unwrap()),
                            ("word", searcher.search_word(needle).unwrap()),
                        ] {
                            let locations: Vec<_> = response
                                .hits
                                .iter()
                                .map(|hit| (hit.file.as_str(), hit.line_start))
                                .collect();
                            let wanted = if present {
                                expected.clone()
                            } else {
                                Vec::new()
                            };
                            assert_eq!(
                                locations, wanted,
                                "{lane} {mode} {needle:?}, CI={case_insensitive}, \
                                 lang={lang_filter:?}, filler={filler_lines}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn short_word_query_survives_more_than_sixteen_windows_of_substrings() {
    let session = ast_sgrep_testkit::isolated_index_session();
    let body = format!("{}// t\n", "// atoll\n".repeat(1700));
    session.write("word.rs", body);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let options = SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    };
    let cold = session.searcher(options.clone());
    let warm = session.searcher(options);
    warm.warm_search_path().unwrap();
    for (lane, searcher) in [("cold", &cold), ("warm", &warm)] {
        let direct = searcher.search_word("t").unwrap();
        assert_eq!(direct.hits.len(), 1, "{lane} direct word:t");
        assert_eq!(direct.hits[0].score, 1.0, "rank counts accepted hits only");
        let response = searcher.search("word:t").unwrap();
        let locations: Vec<_> = response
            .hits
            .iter()
            .map(|hit| (hit.file.as_str(), hit.line_start))
            .collect();
        assert_eq!(locations, vec![("word.rs", 1701)], "{lane} word:t");
    }
}

#[test]
fn definition_boost_does_not_invent_case_sensitive_literal_or_word_matches() {
    let session = ast_sgrep_testkit::isolated_index_session();
    session.write("needle.rs", "fn needle() {}\n");
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let options = SearchOptions {
        use_embed: false,
        case_insensitive: false,
        ..session.search_options()
    };
    let searcher = session.searcher(options.clone());
    assert!(!searcher.search("defs:Needle").unwrap().hits.is_empty());
    assert!(searcher.search_literal("Needle").unwrap().hits.is_empty());
    for query in ["literal:needle", "word:needle"] {
        assert!(
            searcher.search(query).unwrap().hits.iter().any(|hit| {
                hit.kind == ast_sgrep_core::search::HitKind::Def
                    && hit.symbol.as_deref() == Some("needle")
            }),
            "{query} must retain exact-case definition enrichment"
        );
    }
    for query in ["literal:Needle", "word:Needle"] {
        let response = searcher.search(query).unwrap();
        assert!(
            response.hits.is_empty(),
            "{query} must not borrow case-folded definition hits: {:?}",
            response.hits
        );
    }
    let folded = session.searcher(SearchOptions {
        case_insensitive: true,
        ..options
    });
    for query in ["literal:Needle", "word:Needle"] {
        assert!(!folded.search(query).unwrap().hits.is_empty(), "{query}");
    }
}

/// Rerank pools must survive every producer's earlier admission cut. The
/// ordinary budgets remain independent of an inert rerank_top_k setting.
#[test]
fn rerank_pools_survive_connected_candidate_admission() {
    use ast_sgrep_core::search::passes::{embed, lexical, literal, symbol};
    use ast_sgrep_core::{EmbedBackend, HitKind, ParsedQuery, Searcher};
    let session = ast_sgrep_testkit::isolated_index_session();
    for file in 0..250 {
        session.write(
            format!("r{file:03}.rs"),
            "pub fn needle() {\n    use needle;\n    probe();\n    // filler\n}\n",
        );
    }
    session.index_all(IndexOptions {
        embed_semantic: true,
        embed_backend: EmbedBackend::Semantic,
        ..session.index_options()
    });
    let store = session.open_store();
    let mut counts = Vec::new();
    for rerank in [false, true] {
        let options = SearchOptions {
            limit: 1,
            use_embed: true,
            use_rerank: rerank,
            rerank_top_k: 250,
            ann_threshold: Some(usize::MAX),
            ..session.search_options()
        };
        let literal_query = ParsedQuery::parse("literal:probe()");
        counts.push((
            "cold literal",
            rerank,
            literal::literal_pass(&store, &options, &literal_query)
                .unwrap()
                .len(),
            if rerank { 250 } else { 1 },
        ));
        let mut folded = options.clone();
        folded.case_insensitive = true;
        counts.push((
            "SQL literal",
            rerank,
            literal::literal_pass(&store, &folded, &literal_query)
                .unwrap()
                .len(),
            if rerank { 250 } else { 1 },
        ));
        counts.push((
            "lexical",
            rerank,
            lexical::lexical_pass(&store, &options, &ParsedQuery::parse("probe"))
                .unwrap()
                .len(),
            if rerank { 250 } else { 100 },
        ));
        counts.push((
            "defs",
            rerank,
            symbol::search_defs(&store, &options, &ParsedQuery::parse("defs:needle"))
                .unwrap()
                .len(),
            if rerank { 250 } else { 32 },
        ));
        let callers =
            symbol::search_callers(&store, &options, &ParsedQuery::parse("callers:probe")).unwrap();
        counts.push((
            "callers",
            rerank,
            callers
                .iter()
                .filter(|hit| hit.kind == HitKind::Caller)
                .count(),
            if rerank { 250 } else { 32 },
        ));
        counts.push((
            "imports",
            rerank,
            symbol::search_imports(&store, &options, &ParsedQuery::parse("imports:needle"))
                .unwrap()
                .len(),
            if rerank { 250 } else { 200 },
        ));
        counts.push((
            "embedding",
            rerank,
            embed::embed_pass_with_context(
                &store,
                &options,
                &ParsedQuery::parse("needle probe"),
                None,
            )
            .unwrap()
            .len(),
            if rerank { 250 } else { 50 },
        ));
        // Count-only exits before neural reranking. The public with_store
        // seam exercises real pattern dispatch without a model or feature bypass
        // claim for the validated Searcher::new constructor.
        let mut counted = options.clone();
        counted.use_embed = false;
        counted.count_only = true;
        let response = Searcher::with_store(session.open_store(), counted)
            .search("pattern:probe")
            .unwrap();
        counts.push((
            "pattern dispatch",
            rerank,
            response
                .counts
                .iter()
                .map(|(_, count)| *count as usize)
                .sum(),
            if rerank { 250 } else { 1 },
        ));
    }
    // Explicitly warm a separate store only after the cold paths above.
    let warm = Searcher::with_store(
        session.open_store(),
        SearchOptions {
            use_embed: false,
            ..session.search_options()
        },
    );
    warm.warm_search_path().unwrap();
    for rerank in [false, true] {
        let options = SearchOptions {
            limit: 1,
            use_embed: false,
            use_rerank: rerank,
            rerank_top_k: 250,
            ..session.search_options()
        };
        counts.push((
            "warm literal",
            rerank,
            literal::literal_pass(
                warm.store(),
                &options,
                &ParsedQuery::parse("literal:probe()"),
            )
            .unwrap()
            .len(),
            if rerank { 250 } else { 1 },
        ));
    }
    let mismatches: Vec<_> = counts
        .into_iter()
        .filter(|(_, _, actual, expected)| actual != expected)
        .collect();
    assert!(
        mismatches.is_empty(),
        "candidate admission mismatches: {mismatches:?}"
    );
}
