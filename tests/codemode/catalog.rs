use ast_sgrep_codemode::adapters::{
    anthropic_tools, cloudflare_connector, openai_tools, surface_manifest,
};
use ast_sgrep_codemode::{catalog_describe, catalog_search, tool_catalog};
use ast_sgrep_testkit::assert_golden_json_at;
use std::path::{Path, PathBuf};

#[test]
fn catalog_exposes_core_and_discovery_tools() {
    let names: Vec<_> = tool_catalog().iter().map(|t| t.name).collect();
    for required in [
        "search",
        "semantic",
        "chain",
        "defs",
        "callers",
        "index_status",
        "index_repo",
        "filter_hits",
        "select",
        "catalog_search",
        "catalog_describe",
    ] {
        assert!(names.contains(&required), "missing {required}");
    }
}

#[test]
fn progressive_discovery_search_and_describe() {
    let found = catalog_search("chain graph");
    assert!(found.iter().any(|t| t.name == "chain"));
    let def = catalog_describe("search").expect("search");
    assert!(def.input_schema["properties"]["query"].is_object());
    assert!(catalog_describe("nope").is_none());
}

#[test]
fn adapters_emit_host_shaped_tool_lists() {
    let manifest = surface_manifest();
    assert_eq!(manifest["surface"], "codemode");

    let anthropic = anthropic_tools();
    let tools = anthropic.as_array().expect("array");
    assert_eq!(tools[0]["name"], "code_execution");
    assert!(tools.iter().any(|t| t["name"] == "search"));
    assert!(tools
        .iter()
        .any(|t| t["name"] == "search" && t["allowed_callers"].is_array()));

    let openai = openai_tools();
    let otools = openai.as_array().expect("array");
    assert_eq!(otools[0]["type"], "programmatic_tool_calling");
    assert!(otools.iter().any(|t| t["name"] == "chain"));

    let cf = cloudflare_connector();
    assert_eq!(cf["name"], "ast-sgrep");
    assert_eq!(cf["progressiveDiscovery"]["search"], "catalog_search");
    assert!(cf["methods"].as_array().unwrap().len() >= 10);
}

fn catalog_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/codemode/fixtures")
        .join(name)
}

/// nz7i.3: freeze ToolDef catalog and host adapter lists.
#[test]
fn catalog_and_host_adapters_match_goldens() {
    let catalog = serde_json::to_value(tool_catalog()).expect("catalog serializes");
    assert_golden_json_at(&catalog_fixture("tool_catalog.json"), &catalog);
    assert_golden_json_at(&catalog_fixture("anthropic_tools.json"), &anthropic_tools());
    assert_golden_json_at(&catalog_fixture("openai_tools.json"), &openai_tools());
    assert_golden_json_at(
        &catalog_fixture("cloudflare_connector.json"),
        &cloudflare_connector(),
    );
}
