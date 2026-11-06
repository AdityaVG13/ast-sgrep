//! F7: one tool-vocab table — rename-consistency across every surface.
//!
//! INTENT: `ToolName::parse` aliases, the cross-surface session vocabulary,
//! the known-tools list, and the adapters' discovery filters must all derive
//! from a single per-tool record, so renaming a tool is one edit. This test
//! pins that every surface agrees with the record table.
//! KILLS: alias-table drift (rename in one place, stale in another),
//! adapter filter drift (tool hidden/shown inconsistently), mutating-flag
//! disagreement between dispatch and catalog.

use ast_sgrep_codemode::adapters::{
    anthropic_discovery_tools, anthropic_tools, cloudflare_connector, openai_tools,
    surface_manifest,
};
use ast_sgrep_codemode::tools::{tool_records, ToolName};
use ast_sgrep_codemode::{catalog_describe, tool_catalog};
use serde_json::json;
use std::collections::{HashMap, HashSet};

#[test]
fn every_record_parses_and_roundtrips() {
    let records = tool_records();
    // 15 tools: one record each, no more, no less.
    assert_eq!(records.len(), 15, "tool record count drifted");
    let mut canonicals = HashSet::new();
    for record in records {
        // Rename-consistency: canonical parses to its own tool and back.
        assert!(canonicals.insert(record.canonical), "dup {}", record.canonical);
        assert_eq!(
            ToolName::parse(record.canonical),
            Some(record.tool),
            "canonical must parse: {}",
            record.canonical
        );
        assert_eq!(record.tool.as_str(), record.canonical);
        // Every exact alias dispatches to the same tool.
        for alias in record.aliases {
            assert_ne!(*alias, record.canonical);
            assert_eq!(
                ToolName::parse(alias),
                Some(record.tool),
                "alias {alias} must parse to {}",
                record.canonical
            );
        }
        // Session-vocabulary spellings are recover-tier: they must NOT parse
        // (call_tool falls through to recover_tool_name, which stamps
        // `recovered`), and they must not collide with another tool's
        // canonical or exact alias.
        for alias in record.session_aliases {
            assert_eq!(
                ToolName::parse(alias),
                None,
                "session alias {alias} must stay recover-tier"
            );
        }
    }
    // No alias collisions across records.
    let mut owners: HashMap<&str, &str> = HashMap::new();
    for record in records {
        for name in std::iter::once(record.canonical).chain(record.aliases.iter().copied()) {
            let prev = owners.insert(name, record.canonical);
            assert!(prev.is_none(), "alias {name} owned twice");
        }
    }
}

#[test]
fn session_vocab_runs_stamped_from_the_same_records() {
    // Every session_alias in the table resolves end-to-end through
    // recover_tool_name and stamps `recovered` with the canonical name.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("seed.py"), "def seed():\n    pass\n").expect("fixture");
    let (_index, mut session) = ast_sgrep_testkit::indexed_session_at(temp.path());
    let mut covered = 0;
    for record in tool_records() {
        for alias in record.session_aliases {
            covered += 1;
            let args = match record.tool {
                ToolName::Find | ToolName::Search | ToolName::Semantic => {
                    json!({"query": "word:seed"})
                }
                _ => panic!("session alias on unexpected tool {}", record.canonical),
            };
            let ok = session
                .call(alias, args)
                .expect("session alias must run");
            assert_eq!(
                ok["recovered"],
                json!(record.canonical),
                "alias {alias} must stamp recovered"
            );
        }
    }
    assert!(covered >= 3, "session vocab must be covered");
}

#[test]
fn known_tools_catalog_and_suggest_agree_with_records() {
    // The catalog exposes exactly the record canonicals, and its read_only
    // flag is the negation of the record mutating flag.
    let catalog = tool_catalog();
    let canonicals: HashSet<&str> = tool_records().iter().map(|r| r.canonical).collect();
    let names: HashSet<&str> = catalog.iter().map(|t| t.name).collect();
    assert_eq!(names, canonicals, "catalog must match record canonicals");
    for def in &catalog {
        let record = tool_records()
            .iter()
            .find(|r| r.canonical == def.name)
            .expect("record");
        assert_eq!(
            def.read_only, !record.mutating,
            "{}: catalog read_only must negate mutating",
            def.name
        );
        // Every catalog name roundtrips through parse/as_str.
        assert_eq!(ToolName::parse(def.name).unwrap().as_str(), def.name);
    }
    // Mutating set is exactly edit + index_repo: a recovered (non-exact)
    // name must never execute these.
    let mutating: HashSet<&str> = tool_records()
        .iter()
        .filter(|r| r.mutating)
        .map(|r| r.canonical)
        .collect();
    assert_eq!(mutating, HashSet::from(["edit", "index_repo"]));
    assert!(catalog_describe("search").is_some());
    assert!(catalog_describe("nope").is_none());
}

#[test]
fn adapter_discovery_filters_derive_from_records() {
    let discoverable: HashSet<&str> = tool_records()
        .iter()
        .filter(|r| r.discoverable)
        .map(|r| r.canonical)
        .collect();
    let discovery: HashSet<&str> = tool_records()
        .iter()
        .filter(|r| !r.discoverable)
        .map(|r| r.canonical)
        .collect();
    assert_eq!(discovery, HashSet::from(["catalog_search", "catalog_describe"]));

    let strip_host = |tools: &serde_json::Value| -> HashSet<String> {
        tools
            .as_array()
            .expect("array")
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(str::to_string))
            .collect()
    };
    // Always-on lists carry exactly the discoverable tools.
    let anthropic = strip_host(&anthropic_tools());
    assert!(anthropic.contains("code_execution"));
    let openai = strip_host(&openai_tools());
    for name in &discoverable {
        assert!(anthropic.contains(*name), "anthropic missing {name}");
        assert!(openai.contains(*name), "openai missing {name}");
    }
    for name in &discovery {
        assert!(!anthropic.contains(*name), "anthropic leaks {name}");
        assert!(!openai.contains(*name), "openai leaks {name}");
    }
    // Progressive-discovery list carries exactly the hidden tools.
    let progressive = strip_host(&anthropic_discovery_tools());
    let progressive_set: HashSet<&str> =
        progressive.iter().map(String::as_str).collect();
    assert_eq!(progressive_set, discovery);
    // Cloudflare exposes every tool and points progressive discovery at
    // exactly the hidden pair, in record order.
    let cf = cloudflare_connector();
    let methods = strip_host(&cf["methods"]);
    let canonicals: HashSet<String> =
        tool_records().iter().map(|r| r.canonical.to_string()).collect();
    assert_eq!(methods, canonicals);
    let ordered: Vec<&str> = tool_records()
        .iter()
        .filter(|r| !r.discoverable)
        .map(|r| r.canonical)
        .collect();
    assert_eq!(cf["progressiveDiscovery"]["search"], json!(ordered[0]));
    assert_eq!(cf["progressiveDiscovery"]["describe"], json!(ordered[1]));
    assert_eq!(surface_manifest()["defaults"]["progressive_discovery"], json!(ordered));
}
