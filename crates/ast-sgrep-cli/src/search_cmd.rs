//! Search / chain / semantic command runners.

use crate::{
    open_searcher, print_machine_json, resolve_root_index, usage_error, Cli,
};
use anyhow::Context;
use ast_sgrep_core::{
    chain::{expand_chain, ChainConfig},
    format_hit_line, IndexStore, SearchResponse, Searcher,
};
use std::path::Path;

pub(crate) fn run_chain(root: &Path, cli: &Cli, query: &str) -> anyhow::Result<()> {
    let (root, index_path) = resolve_root_index(cli, root);
    let store = IndexStore::open(&root, index_path.as_deref()).context("failed to open index")?;
    let config = ChainConfig {
        limit: cli.limit.unwrap_or(ChainConfig::default().limit),
        top_n: 1,
        ..ChainConfig::default()
    };
    let r = expand_chain(&store, query, &config).context("chain search failed")?;
    if cli.json {
        return print_machine_json("chain", &r);
    }
    println!(
        "chain {:?}: {} nodes, {} edges (max depth {})",
        r.query, r.node_count, r.edge_count, r.max_depth
    );
    println!("nodes:");
    for n in &r.nodes {
        let sym = n.symbol.as_deref().unwrap_or("<file>");
        println!(
            "  depth {} score {:.4} {}:{}-{} {sym}",
            n.depth, n.score, n.file, n.line_start, n.line_end
        );
    }
    println!("edges:");
    for e in &r.edges {
        let from = e.from_symbol.as_deref().unwrap_or("<file>");
        let to = e.to_symbol.as_deref().unwrap_or("<file>");
        println!(
            "  depth {} {:?}: {}::{from} -> {}::{to}",
            e.depth, e.label, e.from_file, e.to_file
        );
    }
    Ok(())
}
pub(crate) fn run_search(root: &Path, cli: &Cli, query: &str, semantic: bool) -> anyhow::Result<()> {
    let ctx = if semantic {
        "semantic search failed"
    } else {
        "search failed"
    };
    let response = do_search(&open_searcher(root, cli)?, query, semantic).context(ctx)?;
    if !cli.json {
        for hit in &response.hits {
            println!("{}", format_hit_line(hit));
        }
        return Ok(());
    }
    let default = if semantic {
        ast_sgrep_plugins::OutputFormat::Agent
    } else {
        ast_sgrep_plugins::OutputFormat::Native
    };
    let format = resolve_output_format(cli.format.as_deref(), default)?;
    print_machine_json(
        if semantic { "semantic" } else { "search" },
        ast_sgrep_plugins::format_response_with(&response, format, cli.excerpt_lines),
    )
}
fn resolve_output_format(
    raw: Option<&str>,
    default: ast_sgrep_plugins::OutputFormat,
) -> anyhow::Result<ast_sgrep_plugins::OutputFormat> {
    match raw {
        Some(raw) => ast_sgrep_plugins::OutputFormat::parse(raw).ok_or_else(|| {
            usage_error(format!(
                "unknown output format {raw:?}; expected native, agent, agent-capsule, github, or gitlab"
            ))
        }),
        None => Ok(default),
    }
}

pub(crate) fn do_search(s: &Searcher, q: &str, semantic: bool) -> anyhow::Result<SearchResponse> {
    if semantic {
        Ok(s.search_semantic(q)?)
    } else {
        Ok(s.search(q)?)
    }
}
