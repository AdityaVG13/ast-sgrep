//! Capsule format: refs + previews by default, bodies only on request; hit order matches agent format.
use ast_sgrep_core::search::{HitKind, HitSignal, SearchHit};
use ast_sgrep_core::SearchResponse;
use ast_sgrep_plugins::{
    format_response_with, format_response_with_budget, to_github_json, to_gitlab_json,
    CompactBudget, OutputFormat,
};
fn sample() -> SearchResponse {
    let long = "x".repeat(300);
    SearchResponse {
        query: "renewal flow".into(),
        limit: 5,
        hits: vec![
            SearchHit {
                kind: HitKind::Def,
                file: "src/auth.rs".into(),
                line_start: 10,
                line_end: 42,
                symbol: Some("auth_refresh".into()),
                caller: None,
                callee: None,
                language: Some("rust".into()),
                score: 5.5,
                signal: HitSignal::Structural,
                contributors: vec![HitKind::Def, HitKind::Embed],
                margin: 0.0,
                excerpt: "fn auth_refresh() {\n    renew_token();\n    log();\n}".into(),
            },
            SearchHit {
                kind: HitKind::Caller,
                file: "src/session.rs".into(),
                line_start: 7,
                line_end: 7,
                symbol: None,
                caller: Some("open_session".into()),
                callee: Some("auth_refresh".into()),
                language: Some("rust".into()),
                score: 3.2,
                signal: HitSignal::Structural,
                contributors: vec![HitKind::Caller],
                margin: 0.0,
                excerpt: format!("   \n{long}"),
            },
        ],
        counts: Vec::new(),
        read_bytes_estimate: 1_000,
        returned_excerpt_bytes: 350,
        prevented_read_bytes: 650,
    }
}
#[test]
fn capsule_hits_carry_refs_and_previews_without_bodies() {
    let response = sample();
    let capsule = format_response_with(&response, OutputFormat::AgentCapsule, 0);
    assert_eq!(capsule["mode"], "capsule");
    assert_eq!(capsule["hit_count"], 2);
    let hits = capsule["hits"].as_array().expect("hits");
    assert_eq!(hits[0]["ref"], "src/auth.rs#L10-L42");
    assert_eq!(hits[0]["symbol"], "auth_refresh");
    assert_eq!(hits[0]["preview"], "fn auth_refresh() {");
    assert_eq!(hits[0]["signal"], "structural");
    assert_eq!(hits[0]["contributors"], serde_json::json!(["def", "embed"]));
    assert_eq!(hits[0]["margin"], 0.0);
    assert!(hits[0].get("excerpt").is_none(), "no body by default");
    assert_eq!(hits[1]["symbol"], serde_json::Value::Null);
    assert_eq!(hits[1]["caller"], "open_session");
    assert_eq!(hits[1]["callee"], "auth_refresh");
    let preview = hits[1]["preview"].as_str().expect("preview");
    assert!(preview.chars().count() <= 121, "len {}", preview.len());
    assert!(preview.starts_with('x'));
    let agent = format_response_with(&response, OutputFormat::Agent, 0);
    assert_ne!(capsule["returned_excerpt_bytes"], 350);
    assert_eq!(agent["prevented_read_bytes"], 650);
    assert_eq!(agent["hits"][0]["signal"], "structural");
    assert_eq!(
        agent["hits"][0]["contributors"],
        serde_json::json!(["def", "embed"])
    );
    assert_eq!(agent["hits"][0]["semantic"], true);
    assert_eq!(agent["hits"][0]["margin"], 0.0);
    assert_eq!(capsule["prevented_read_bytes"], 650);
}
fn decoded_compact_identities(value: &serde_json::Value) -> Vec<(String, u32, u32, String)> {
    let paths = value["p"].as_object().expect("path dictionary");
    value["h"]
        .as_array()
        .expect("compact hits")
        .iter()
        .map(|row| {
            let row = row.as_array().expect("compact row");
            let id = row[0].as_str().expect("compact id");
            let (path_id, span) = id.rsplit_once(':').expect("path id and span");
            let (start, end) = span.split_once('-').expect("start and end");
            (
                paths[path_id].as_str().expect("path").to_owned(),
                start.parse().expect("start"),
                end.parse().expect("end"),
                row[3].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect()
}

fn response_identities(response: &SearchResponse) -> Vec<(String, u32, u32, String)> {
    response
        .hits
        .iter()
        .map(|hit| {
            (
                hit.file.clone(),
                hit.line_start,
                hit.line_end,
                hit.symbol
                    .as_deref()
                    .or(hit.callee.as_deref())
                    .or(hit.caller.as_deref())
                    .unwrap_or("")
                    .to_owned(),
            )
        })
        .collect()
}

#[test]
fn compact_hits_preserve_ranked_identity_and_enforce_budgets() {
    let response = sample();
    let compact = format_response_with_budget(
        &response,
        OutputFormat::Compact,
        0,
        CompactBudget {
            per_result_tokens: 7,
            response_tokens: 10,
        },
    );
    assert_eq!(
        decoded_compact_identities(&compact),
        response_identities(&response)
    );
    assert_eq!(compact["p"].as_object().expect("paths").len(), 2);
    assert_eq!(compact["zb"], serde_json::json!([7, 10, 10]));
    assert_eq!(compact["zt"], 2);
    for row in compact["h"].as_array().expect("hits") {
        assert!(row[4].as_str().expect("snippet").len() <= 7);
        assert!(!row[0].as_str().expect("id").contains("src/"));
    }

    let again = format_response_with_budget(
        &response,
        OutputFormat::Compact,
        0,
        CompactBudget {
            per_result_tokens: 7,
            response_tokens: 10,
        },
    );
    assert_eq!(compact, again, "short IDs and path ordering are stable");
}

#[test]
fn compact_utf8_budgets_never_split_codepoints() {
    let mut response = sample();
    response.hits.truncate(1);
    response.hits[0].excerpt = "🦀rust".into();
    let compact = format_response_with_budget(
        &response,
        OutputFormat::Compact,
        0,
        CompactBudget {
            per_result_tokens: 3,
            response_tokens: 3,
        },
    );
    assert_eq!(compact["h"][0][4], "");
    assert_eq!(compact["zb"][2], 0);
    assert_eq!(compact["zt"], 1);
}

#[test]
fn compact_fixed_query_set_halves_conservative_token_units() {
    let mut cases = Vec::new();
    for query in ["renewal flow", "session caller", "token refresh"] {
        let mut response = sample();
        response.query = query.into();
        for (index, hit) in response.hits.iter_mut().enumerate() {
            hit.excerpt = format!(
                "fn result_{index}() {{\n{}\n}}",
                "    perform_identity_preserving_work();\n".repeat(40)
            );
        }
        cases.push(response);
    }

    let mut native_units = 0_usize;
    let mut compact_units = 0_usize;
    let mut hit_count = 0_usize;
    for response in &cases {
        let native = format_response_with(response, OutputFormat::Native, 0);
        let compact = format_response_with(response, OutputFormat::Compact, 0);
        assert_eq!(
            decoded_compact_identities(&compact),
            response_identities(response)
        );
        native_units += serde_json::to_vec(&native).expect("native JSON").len();
        compact_units += serde_json::to_vec(&compact).expect("compact JSON").len();
        hit_count += response.hits.len();
    }
    assert!(
        compact_units * 2 <= native_units,
        "compact must save >=50%: native={native_units} compact={compact_units}"
    );
    eprintln!(
        "fixed_query_token_units_per_result native={:.1} compact={:.1} reduction={:.1}%",
        native_units as f64 / hit_count as f64,
        compact_units as f64 / hit_count as f64,
        100.0 * (1.0 - compact_units as f64 / native_units as f64)
    );
}

#[test]
fn github_page_at_limit_is_marked_incomplete() {
    let mut response = sample();
    response.limit = response.hits.len();
    let github = to_github_json(&response);
    assert_eq!(github["total_count"], response.hits.len());
    assert_eq!(github["incomplete_results"], true);
    assert_eq!(github["items"][0]["metadata"]["signal"], "structural");
    assert_eq!(
        github["items"][0]["metadata"]["contributors"],
        serde_json::json!(["def", "embed"])
    );
    assert_eq!(github["items"][0]["metadata"]["margin"], 0.0);
}
#[test]
fn agent_suggested_next_is_executable_asgrep_only() {
    let response = sample();
    let agent = format_response_with(&response, OutputFormat::Agent, 0);
    let suggested = agent["suggested_next"]
        .as_array()
        .expect("suggested_next")
        .iter()
        .map(|v| v.as_str().expect("string").to_owned())
        .collect::<Vec<_>>();
    assert!(!suggested.is_empty());
    for cmd in &suggested {
        assert!(
            cmd.starts_with("asgrep "),
            "suggested_next must be executable asgrep commands, got: {cmd}"
        );
        assert!(
            !cmd.contains("ast-grep") && !cmd.starts_with("rg ") && !cmd.starts_with("pattern:"),
            "suggested_next must not recommend non-asgrep myths, got: {cmd}"
        );
    }
}
#[test]
fn gitlab_projection_documents_absent_repository_context() {
    let hits = to_gitlab_json(&sample())["data"]
        .as_array()
        .expect("data")
        .clone();
    assert!(
        hits.iter().all(|h| h["ref"] == "HEAD") && hits.iter().all(|h| h["project_id"].is_null())
    );
    assert!(hits.iter().all(|hit| hit["meta"]["signal"] == "structural"));
    assert!(hits
        .iter()
        .all(|hit| hit["meta"]["contributors"].is_array()));
    assert!(hits.iter().all(|hit| hit["meta"]["margin"] == 0.0));
}

/// kxmc: the MCP surface moved from pretty `AgentCapsule` to minified `Compact`.
/// This pins the saving so a future edit cannot quietly give it back.
///
/// Run with `--nocapture` to print the measured byte counts.
#[test]
fn compact_minified_is_much_smaller_than_pretty_capsule() {
    let response = many_file_sample();
    let old = serde_json::to_string_pretty(&format_response_with(
        &response,
        OutputFormat::AgentCapsule,
        0,
    ))
    .expect("capsule serializes");
    let new = serde_json::to_string(&format_response_with_budget(
        &response,
        OutputFormat::Compact,
        0,
        CompactBudget::default(),
    ))
    .expect("compact serializes");

    let saved = 100 - (new.len() * 100 / old.len());
    println!("pretty capsule = {} bytes", old.len());
    println!("minified compact = {} bytes", new.len());
    println!("saved = {saved}%");

    assert!(
        new.len() * 2 < old.len(),
        "compact must be under half of pretty capsule: {} vs {}",
        new.len(),
        old.len()
    );
    // No path may be repeated per hit the way `file` plus `ref` used to be.
    // With root folding (am4a) a path is stored as root plus suffix, so assert
    // on the resolved paths rather than raw substrings.
    let compact: serde_json::Value = serde_json::from_str(&new).expect("compact parses");
    for (_, path) in ast_sgrep_plugins::resolve_compact_paths(&compact) {
        assert!(
            new.matches(&path).count() <= 1,
            "path {path} emitted more than once"
        );
        let name = path.rsplit('/').next().expect("file name");
        assert_eq!(new.matches(name).count(), 1, "{name} emitted more than once");
    }
}

/// Ten hits over three files: the shape where per-hit key repetition dominates.
fn many_file_sample() -> SearchResponse {
    let files = [
        "crates/ast-sgrep-core/src/search/mod.rs",
        "crates/ast-sgrep-core/src/search/types.rs",
        "crates/ast-sgrep-core/src/store/sqlite.rs",
    ];
    let hits = (0..10)
        .map(|index| SearchHit {
            kind: HitKind::Def,
            file: files[index % files.len()].into(),
            line_start: index as u32 * 10 + 1,
            line_end: index as u32 * 10 + 9,
            symbol: Some(format!("handler_{index}")),
            caller: None,
            callee: None,
            language: Some("rust".into()),
            score: 9.0 - index as f64,
            signal: HitSignal::Exact,
            contributors: vec![HitKind::Def],
            margin: 0.1,
            excerpt: format!("fn handler_{index}(session: &Session) -> Result<Token> {{\n    rotate(session)\n}}"),
        })
        .collect();
    SearchResponse {
        query: "session rotate".into(),
        limit: 10,
        hits,
        counts: Vec::new(),
        read_bytes_estimate: 4_000,
        returned_excerpt_bytes: 800,
        prevented_read_bytes: 3_200,
    }
}

/// am4a: shared directory prefixes are emitted once in `r`, and every folded
/// entry reconstructs its original path exactly.
#[test]
fn compact_path_table_folds_shared_roots_and_round_trips() {
    let response = many_file_sample();
    let compact = format_response_with_budget(
        &response,
        OutputFormat::Compact,
        0,
        CompactBudget::default(),
    );
    let text = serde_json::to_string(&compact).expect("compact serializes");

    let roots = compact["r"].as_array().expect("root table present");
    assert_eq!(
        roots.len(),
        1,
        "the byte-optimal root set is the single shared prefix: {roots:?}"
    );
    assert_eq!(roots[0], "crates/ast-sgrep-core/src/");
    // The shared prefix now appears once for the whole envelope.
    assert_eq!(text.matches("crates/ast-sgrep-core/src/").count(), 1);

    let resolved: std::collections::BTreeMap<_, _> =
        ast_sgrep_plugins::resolve_compact_paths(&compact)
            .into_iter()
            .collect();
    let expected: std::collections::BTreeSet<_> =
        response.hits.iter().map(|hit| hit.file.clone()).collect();
    let actual: std::collections::BTreeSet<_> = resolved.values().cloned().collect();
    assert_eq!(actual, expected, "round trip lost or altered a path");

    // Every hit id still resolves through the table.
    for hit in compact["h"].as_array().expect("hits") {
        let id = hit[0].as_str().expect("id");
        let (path_id, _) = id.rsplit_once(':').expect("id shape");
        assert!(resolved.contains_key(path_id), "unresolved id {id}");
    }
}

/// am4a: folding must never inflate. Paths with nothing in common stay
/// verbatim and no root table is emitted.
#[test]
fn compact_path_table_skips_folding_when_it_would_not_help() {
    let mut response = many_file_sample();
    for (index, hit) in response.hits.iter_mut().enumerate() {
        hit.file = format!("{index}.rs");
    }
    let compact = format_response_with_budget(
        &response,
        OutputFormat::Compact,
        0,
        CompactBudget::default(),
    );
    assert!(compact.get("r").is_none(), "no root table expected");
    for entry in compact["p"].as_object().expect("path table").values() {
        assert!(entry.is_string(), "unfolded entries stay plain strings");
    }
    let resolved = ast_sgrep_plugins::resolve_compact_paths(&compact);
    assert_eq!(resolved.len(), response.hits.len().min(10));
}
