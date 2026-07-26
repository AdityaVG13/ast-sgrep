use ast_sgrep_core::chain::{expand_chain, ChainConfig, EdgeLabel};
use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::store::IndexStore;
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use rusqlite::params;
use std::fs;

struct SymbolCase {
    file: &'static str,
    source: &'static str,
    symbol: &'static str,
}

fn count_rows(store: &IndexStore, sql: &str, value: &str) -> usize {
    store
        .connection()
        .query_row(sql, params![value], |row| row.get::<_, i64>(0))
        .unwrap() as usize
}

#[test]
fn indexed_graph_keys_are_retrievable_across_modes() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    let symbols = [
        SymbolCase {
            file: "camel.js",
            source: "export function camelCase() {}\nexport function useCamel() { camelCase(); }\n",
            symbol: "camelCase",
        },
        SymbolCase {
            file: "snake.ts",
            source: "export function snake_case(): void {}\nexport function useSnake(): void { snake_case(); }\n",
            symbol: "snake_case",
        },
        SymbolCase {
            file: "upper.py",
            source: "def UPPER_CASE():\n    pass\n\ndef use_upper():\n    UPPER_CASE()\n",
            symbol: "UPPER_CASE",
        },
        SymbolCase {
            file: "pascal.rs",
            source: "fn PascalCase() {}\nfn use_pascal() { PascalCase(); }\n",
            symbol: "PascalCase",
        },
    ];
    for case in &symbols {
        fs::write(corpus.path().join(case.file), case.source).unwrap();
    }
    fs::write(
        corpus.path().join("imports.ts"),
        "import './camelCase';\nimport './snake_case';\nimport './UPPER_CASE';\nimport './PascalCase';\n",
    )
    .unwrap();

    let index_path = index_dir.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    let indexed = indexer.index_all().unwrap();
    assert_eq!(indexed.files_indexed, symbols.len() + 1);

    let store = IndexStore::open(corpus.path(), Some(&index_path)).unwrap();
    let searcher = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        limit: 64,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();

    for case in &symbols {
        let indexed_defs = count_rows(
            &store,
            "SELECT COUNT(*) FROM symbols WHERE lower(name) = lower(?)",
            case.symbol,
        );
        let indexed_callers = count_rows(
            &store,
            "SELECT COUNT(*) FROM callers WHERE lower(callee) = lower(?)",
            case.symbol,
        );
        assert!(
            indexed_defs > 0,
            "{} has no indexed definition",
            case.symbol
        );
        assert!(indexed_callers > 0, "{} has no indexed caller", case.symbol);

        let variants = [
            case.symbol.to_string(),
            case.symbol.to_lowercase(),
            case.symbol.to_uppercase(),
        ];
        for variant in variants {
            let defs = searcher.search(&format!("defs:{variant}")).unwrap();
            let callers = searcher.search(&format!("callers:{variant}")).unwrap();
            assert_eq!(
                defs.hits
                    .iter()
                    .filter(|hit| hit.kind == HitKind::Def)
                    .count(),
                indexed_defs,
                "defs:{variant} diverged from the indexed definition count"
            );
            assert_eq!(
                callers
                    .hits
                    .iter()
                    .filter(|hit| hit.kind == HitKind::Caller)
                    .count(),
                indexed_callers,
                "callers:{variant} diverged from the indexed caller count"
            );

            let chain = expand_chain(
                &store,
                &variant,
                &ChainConfig {
                    top_n: 8,
                    max_depth: 1,
                    limit: 64,
                    ..ChainConfig::default()
                },
            )
            .unwrap();
            assert!(
                chain
                    .edges
                    .iter()
                    .any(|edge| edge.label == EdgeLabel::CalledBy),
                "chain {variant} omitted the indexed CalledBy edge"
            );
        }
    }

    for symbol in symbols.map(|case| case.symbol) {
        let module = format!("./{symbol}");
        let indexed_imports = count_rows(
            &store,
            "SELECT COUNT(*) FROM imports WHERE lower(module_path) = lower(?)",
            &module,
        );
        assert!(indexed_imports > 0, "{module} has no indexed import");
        for variant in [module.clone(), module.to_lowercase(), module.to_uppercase()] {
            let imports = searcher.search(&format!("imports:{variant}")).unwrap();
            assert_eq!(
                imports
                    .hits
                    .iter()
                    .filter(|hit| hit.kind == HitKind::Import)
                    .count(),
                indexed_imports,
                "imports:{variant} diverged from the indexed import count"
            );
        }
    }
}
