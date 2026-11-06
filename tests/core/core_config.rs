//! F4: one `CoreConfig` env registry resolved at construction.
//!
//! KILLS: scattered-construction (two ASGREP_* knobs must flow from a single
//! construction point instead of ad-hoc per-site `std::env` reads). Fails to
//! compile before F4 lands (no such single point exists); passes after.

use ast_sgrep_core::{CoreConfig, IndexOptions, Indexer, SearchOptions, Searcher};

/// Save the current value (if any) so the process env is restored on exit.
fn stash(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn restore(name: &str, prior: Option<String>) {
    match prior {
        Some(value) => std::env::set_var(name, value),
        None => std::env::remove_var(name),
    }
}

#[test]
fn two_env_knobs_flow_from_one_construction_point() {
    let prior_no_embed = stash("ASGREP_NO_EMBED");
    let prior_semantic_only = stash("ASGREP_SEMANTIC_ONLY");
    std::env::set_var("ASGREP_NO_EMBED", "1");
    std::env::set_var("ASGREP_SEMANTIC_ONLY", "1");

    let result = (|| -> ast_sgrep_core::Result<()> {
        // ONE construction point resolves both knobs.
        let config = CoreConfig::from_env();
        assert!(!config.use_embed, "ASGREP_NO_EMBED=1 must disable embed");
        assert!(
            config.use_semantic_only,
            "ASGREP_SEMANTIC_ONLY=1 must enable semantic-only"
        );

        // Options derived from that same point agree with it.
        let from_config = SearchOptions::from_core_config(&config);
        assert_eq!(from_config.use_embed, config.use_embed);
        assert_eq!(from_config.use_semantic_only, config.use_semantic_only);
        assert_eq!(from_config.limit, config.default_limit);

        // The Searcher built under this env carries the resolved config.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("corpus");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src").join("main.rs"), "pub fn config_needle() {}\n").unwrap();
        let db = tmp.path().join("index.db");
        let mut indexer = Indexer::new(IndexOptions {
            root: root.clone(),
            index_path: Some(db.clone()),
            embed_semantic: false,
            ..IndexOptions::default()
        })?;
        indexer.index_all()?;
        let searcher = Searcher::new(SearchOptions {
            root,
            index_path: Some(db),
            ..SearchOptions::default()
        })?;
        assert_eq!(searcher.core_config().use_embed, config.use_embed);
        assert_eq!(
            searcher.core_config().use_semantic_only,
            config.use_semantic_only
        );
        // Behavior is unchanged: the needle still answers through the walk.
        let hits = searcher.search("config_needle")?.hits;
        assert!(
            hits.iter().any(|h| h.file.ends_with("main.rs")),
            "search must still serve hits with the resolved config"
        );
        Ok(())
    })();

    restore("ASGREP_NO_EMBED", prior_no_embed);
    restore("ASGREP_SEMANTIC_ONLY", prior_semantic_only);
    result.unwrap();
}
