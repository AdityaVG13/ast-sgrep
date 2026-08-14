use super::*;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn write_scip(name: &str, contents: &[u8]) -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join(name);
    fs::write(&path, contents).unwrap();
    (temp, path)
}

#[test]
fn missing_scip_index_degrades() {
    let load = load_scip_index(Path::new("/tmp/asgrep-kgvi1-missing.scip.json"));
    let reason = load.degraded_reason().expect("must degrade");
    assert!(reason.contains("not found"), "unexpected: {reason}");
}

#[test]
fn malformed_json_degrades() {
    let (_temp, path) = write_scip("bad.json", b"{");
    let load = load_scip_index(&path);
    let reason = load.degraded_reason().expect("must degrade");
    assert!(reason.contains("malformed"), "unexpected: {reason}");
}

#[test]
fn protobuf_or_binary_degrades() {
    let (_temp, path) = write_scip("index.scip", &[0x0a, 0x04, b's', b'c', b'i', b'p']);
    let load = load_scip_index(&path);
    let reason = load.degraded_reason().expect("must degrade");
    assert!(
        reason.contains("protobuf") || reason.contains("binary"),
        "unexpected: {reason}"
    );
}

#[test]
fn valid_json_fixture_loads_definition_occurrence() {
    let json = r#"{
        "documents": [{
            "relative_path": "src/auth.rs",
            "occurrences": [{
                "symbol": "rust+crate+auth+refresh().",
                "symbol_roles": 1,
                "range": [10, 0, 10, 7]
            }]
        }]
    }"#;
    let (_temp, path) = write_scip("index.json", json.as_bytes());
    match load_scip_index(&path) {
        ScipLoad::Loaded(index) => {
            assert_eq!(index.documents.len(), 1);
            assert_eq!(index.documents[0].relative_path, "src/auth.rs");
            let occ = &index.documents[0].occurrences[0];
            assert!(occ.is_definition());
            assert_eq!(occ.symbol, "rust+crate+auth+refresh().");
            assert_eq!(occ.range, vec![10, 0, 10, 7]);
        }
        ScipLoad::Degraded { reason } => panic!("fixture must load, got {reason}"),
    }
}

#[test]
fn camel_case_relative_path_alias_loads() {
    let json = r#"{"documents":[{"relativePath":"a.rs","occurrences":[]}]}"#;
    let (_temp, path) = write_scip("camel.json", json.as_bytes());
    match load_scip_index(&path) {
        ScipLoad::Loaded(index) => assert_eq!(index.documents[0].relative_path, "a.rs"),
        ScipLoad::Degraded { reason } => panic!("alias must load, got {reason}"),
    }
}

#[test]
fn scip_symbol_ident_takes_last_identifier() {
    assert_eq!(
        scip_symbol_ident("rust+crate+auth+refresh().").as_deref(),
        Some("refresh")
    );
    assert_eq!(scip_symbol_ident("send").as_deref(), Some("send"));
    assert_eq!(scip_symbol_ident("").as_deref(), None);
}

#[test]
fn occurrence_line_is_one_based() {
    let occ = ScipOccurrence {
        symbol: "send".into(),
        symbol_roles: 0,
        range: vec![1, 4, 1, 8],
    };
    assert_eq!(occ.start_line_1based(), Some(2));
}
