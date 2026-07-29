//! Regression for bead ast-sgrep-5wkz (F-07): resolve_module_path must be
//! language-aware so chain Imports edges resolve for Python/JS/TS/Go, not only Rust.
use ast_sgrep_core::chain::{expand_chain, ChainConfig, EdgeLabel};
use ast_sgrep_core::store::{ImportRow, SymbolRow, UpsertFileInput};
use ast_sgrep_core::IndexStore;
use tempfile::TempDir;

fn upsert(
    store: &IndexStore,
    path: &str,
    language: &str,
    hash: &str,
    lines: &[(u32, String)],
    symbols: &[SymbolRow],
    imports: &[ImportRow],
) {
    store
        .upsert_file(UpsertFileInput {
            rel_path: path,
            language: Some(language),
            mtime_secs: 1,
            mtime_nanos: 0,
            content_hash: hash,
            lines,
            eol: "\n",
            symbols,
            callers: &[],
            imports,
            pattern_nodes: &[],
            semantic_chunks: &[],
            embed_semantic: false,
            embed_backend: ast_sgrep_embed::EmbedPreference::Auto,
        })
        .unwrap();
}

fn sym(name: &str, line: u32) -> SymbolRow {
    SymbolRow {
        name: name.into(),
        kind: "function".into(),
        line_start: line,
        line_end: line,
        byte_start: 0,
        byte_end: 0,
    }
}

#[test]
fn resolve_python_dotted_and_package_init() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    upsert(
        &store,
        "pkg/util.py",
        "python",
        "h1",
        &[(1, "def helper(): pass".into())],
        &[sym("helper", 1)],
        &[],
    );
    upsert(
        &store,
        "pkg/sub/__init__.py",
        "python",
        "h2",
        &[(1, "def init_fn(): pass".into())],
        &[sym("init_fn", 1)],
        &[],
    );
    upsert(
        &store,
        "app.py",
        "python",
        "h3",
        &[(1, "from pkg.util import helper".into())],
        &[sym("main", 2)],
        &[ImportRow {
            module_path: "pkg.util".into(),
            line_no: 1,
        }],
    );

    let resolved = store.resolve_module_path("app.py", "pkg.util").unwrap();
    assert!(
        resolved.iter().any(|p| p == "pkg/util.py"),
        "python dotted import must resolve to pkg/util.py; got {resolved:?}"
    );

    let pkg = store.resolve_module_path("app.py", "pkg.sub").unwrap();
    assert!(
        pkg.iter().any(|p| p == "pkg/sub/__init__.py"),
        "python package import must resolve __init__.py; got {pkg:?}"
    );
}

#[test]
fn resolve_typescript_relative_and_index() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    upsert(
        &store,
        "src/utils/index.ts",
        "typescript",
        "h1",
        &[(1, "export function util() {}".into())],
        &[sym("util", 1)],
        &[],
    );
    upsert(
        &store,
        "src/app.ts",
        "typescript",
        "h2",
        &[(1, "import { util } from './utils';".into())],
        &[sym("run", 2)],
        &[ImportRow {
            module_path: "./utils".into(),
            line_no: 1,
        }],
    );

    let resolved = store.resolve_module_path("src/app.ts", "./utils").unwrap();
    assert!(
        resolved.iter().any(|p| p == "src/utils/index.ts"),
        "TS relative import must resolve index.ts; got {resolved:?}"
    );
}

#[test]
fn resolve_go_import_path_suffix() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    upsert(
        &store,
        "pkg/util/util.go",
        "go",
        "h1",
        &[(1, "package util\nfunc Helper() {}".into())],
        &[sym("Helper", 2)],
        &[],
    );
    upsert(
        &store,
        "cmd/main.go",
        "go",
        "h2",
        &[(1, "package main\nimport \"example.com/demo/pkg/util\"".into())],
        &[sym("main", 3)],
        &[ImportRow {
            module_path: "example.com/demo/pkg/util".into(),
            line_no: 2,
        }],
    );

    let resolved = store
        .resolve_module_path("cmd/main.go", "example.com/demo/pkg/util")
        .unwrap();
    assert!(
        resolved.iter().any(|p| p == "pkg/util/util.go"),
        "Go import path suffix must resolve local package file; got {resolved:?}"
    );
}

#[test]
fn resolve_rust_crate_path_still_works() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    upsert(
        &store,
        "crate/src/util.rs",
        "rust",
        "h1",
        &[(1, "pub fn helper() {}".into())],
        &[sym("helper", 1)],
        &[],
    );
    upsert(
        &store,
        "crate/src/main.rs",
        "rust",
        "h2",
        &[(1, "use crate::util::helper;".into())],
        &[sym("main", 2)],
        &[ImportRow {
            module_path: "crate::util".into(),
            line_no: 1,
        }],
    );

    let resolved = store
        .resolve_module_path("crate/src/main.rs", "crate::util")
        .unwrap();
    assert!(
        resolved.iter().any(|p| p == "crate/src/util.rs"),
        "Rust crate:: path must still resolve; got {resolved:?}"
    );
}

#[test]
fn chain_imports_edge_resolves_for_typescript() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    upsert(
        &store,
        "lib.ts",
        "typescript",
        "h1",
        &[(1, "export function greet() {}".into())],
        &[sym("greet", 1)],
        &[],
    );
    upsert(
        &store,
        "main.ts",
        "typescript",
        "h2",
        &[
            (1, "import { greet } from './lib';".into()),
            (2, "export function run() { greet(); }".into()),
        ],
        &[sym("run", 2)],
        &[ImportRow {
            module_path: "./lib".into(),
            line_no: 1,
        }],
    );

    let chain = expand_chain(
        &store,
        "run",
        &ChainConfig {
            top_n: 8,
            max_depth: 1,
            limit: 32,
            ..ChainConfig::default()
        },
    )
    .unwrap();
    assert!(
        chain.edges.iter().any(|e| {
            e.label == EdgeLabel::Imports
                && e.from_file == "main.ts"
                && e.to_file == "lib.ts"
        }),
        "chain must emit Imports edge main.ts -> lib.ts; edges={:#?}",
        chain.edges
    );
}
