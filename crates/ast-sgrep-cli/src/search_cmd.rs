//! Search / keyword / semantic / chain command helpers.

use crate::machine::{print_machine_json, print_machine_json_with_style, write_stdout_line};
use crate::{
    ensure_existing_root, ensure_nonempty_index, open_searcher, resolve_root_index, usage_error, Cli,
};
use anyhow::Context;
use ast_sgrep_core::{
    chain::{expand_chain, ChainConfig},
    format_hit_line, IndexStore, SearchResponse, Searcher,
};
use std::path::Path;

pub(crate) fn run_chain(root: &Path, cli: &Cli, query: &str) -> anyhow::Result<()> {
    let root = ensure_existing_root(root, cli)?;
    let (_, index_path) = resolve_root_index(cli, &root);
    // 0obi: honor the requested durability profile on the read path too.
    let store = IndexStore::open_with_durability(
        &root,
        index_path.as_deref(),
        cli.durability.unwrap_or_default(),
    )
    .context("failed to open index")?;
    ensure_nonempty_index(&root, store.status()?.file_count)?;
    let config = ChainConfig {
        limit: ast_sgrep_core::clamp_output_limit(
            cli.limit,
            ChainConfig::default().limit,
        ),
        top_n: 1,
        ..ChainConfig::default()
    };
    let r = expand_chain(&store, query, &config).context("chain search failed")?;
    if cli.json {
        return print_machine_json("chain", &r);
    }
    // Human output: agents often pipe through head; never panic on broken pipe.
    write_stdout_line(&format!(
        "chain {:?}: {} nodes, {} edges (max depth {})",
        r.query, r.node_count, r.edge_count, r.max_depth
    ))?;
    write_stdout_line("nodes:")?;
    for n in &r.nodes {
        let sym = n.symbol.as_deref().unwrap_or("<file>");
        write_stdout_line(&format!(
            "  depth {} score {:.4} {}:{}-{} {sym}",
            n.depth, n.score, n.file, n.line_start, n.line_end
        ))?;
    }
    write_stdout_line("edges:")?;
    for e in &r.edges {
        let from = e.from_symbol.as_deref().unwrap_or("<file>");
        let to = e.to_symbol.as_deref().unwrap_or("<file>");
        write_stdout_line(&format!(
            "  depth {} {:?}: {}::{from} -> {}::{to}",
            e.depth, e.label, e.from_file, e.to_file
        ))?;
    }
    Ok(())
}

pub(crate) fn run_keyword_search(root: &Path, cli: &Cli, query: &str) -> anyhow::Result<()> {
    let response = open_searcher(root, cli)?
        .search_lexical(query)
        .context("keyword search failed")?;
    if !cli.search_machine_output() {
        for hit in &response.hits {
            write_stdout_line(&format_hit_line(hit))?;
        }
        return Ok(());
    }
    let format = resolve_output_format(
        cli.active_tuning().format.as_deref(),
        ast_sgrep_plugins::OutputFormat::Native,
    )?;
    print_search_response("keyword", &response, format, cli)
}

/// Whether this invocation runs the semantic channel (flag or global tuning).
fn uses_semantic_channel(cli: &Cli, semantic: bool) -> bool {
    semantic || cli.active_tuning().semantic_only
}

pub(crate) fn run_search(
    root: &Path,
    cli: &Cli,
    query: &str,
    semantic: bool,
) -> anyhow::Result<()> {
    let semantic_ch = uses_semantic_channel(cli, semantic);
    let ctx = if semantic_ch {
        "semantic search failed"
    } else {
        "search failed"
    };
    let response =
        do_search_with_cli(&open_searcher(root, cli)?, query, semantic, cli).context(ctx)?;
    if !cli.search_machine_output() {
        for hit in &response.hits {
            write_stdout_line(&format_hit_line(hit))?;
        }
        return Ok(());
    }
    let tuning = cli.active_tuning();
    let default = if semantic_ch {
        ast_sgrep_plugins::OutputFormat::Agent
    } else {
        ast_sgrep_plugins::OutputFormat::Native
    };
    let format = resolve_output_format(tuning.format.as_deref(), default)?;
    print_search_response(
        if semantic_ch { "semantic" } else { "search" },
        &response,
        format,
        cli,
    )
}

fn print_search_response(
    command: &str,
    response: &ast_sgrep_core::SearchResponse,
    format: ast_sgrep_plugins::OutputFormat,
    cli: &Cli,
) -> anyhow::Result<()> {
    // 6a3i: compact mode answers a miss with a diagnostic envelope instead of
    // an empty result set the caller has to interpret.
    let tuning = cli.active_tuning();
    let value = if format == ast_sgrep_plugins::OutputFormat::Compact && response.hits.is_empty() {
        ast_sgrep_plugins::to_compact_miss_json(&response.query, &miss_context(command, cli))
    } else if let (ast_sgrep_plugins::OutputFormat::Compact, Some(max_tokens)) =
        (format, tuning.budget_tokens)
    {
        // m38g: budget mode picks per-result detail under one response ceiling.
        ast_sgrep_plugins::to_budgeted_compact_json(
            response,
            ast_sgrep_plugins::OutputBudget {
                max_tokens,
                default_detail: ast_sgrep_plugins::DetailLevel::Full,
            },
        )
    } else {
        ast_sgrep_plugins::format_response_with_budget(
            response,
            format,
            cli.active_tuning().excerpt_lines,
            ast_sgrep_plugins::CompactBudget {
                per_result_tokens: cli.active_tuning().snippet_tokens,
                response_tokens: cli.active_tuning().response_snippet_tokens,
            },
        )
    };
    print_machine_json_with_style(
        command,
        value,
        format == ast_sgrep_plugins::OutputFormat::Compact,
        true,
        0,
    )
}

/// Describe a zero-hit CLI search: which channel ran, and what scoped it (6a3i).
fn miss_context(command: &str, cli: &Cli) -> ast_sgrep_plugins::MissContext {
    let mut scope = Vec::new();
    if let Some(lang) = &cli.lang {
        scope.push(("lang".to_owned(), lang.clone()));
    }
    ast_sgrep_plugins::MissContext {
        // Report the CHANNEL that ran, not the command name the user typed.
        tried: vec![match command {
            "keyword" => "lexical".to_owned(),
            "semantic" => "semantic".to_owned(),
            // The default path fuses channels rather than running just one.
            _ => "hybrid".to_owned(),
        }],
        unavailable: Vec::new(),
        scope,
        indexed_files: None,
    }
}

fn resolve_output_format(
    raw: Option<&str>,
    default: ast_sgrep_plugins::OutputFormat,
) -> anyhow::Result<ast_sgrep_plugins::OutputFormat> {
    match raw {
        Some(raw) => ast_sgrep_plugins::OutputFormat::parse(raw).ok_or_else(|| {
            usage_error(format!(
                "unknown output format {raw:?}; expected native, agent, agent-capsule, compact, github, or gitlab"
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

pub(crate) fn do_search_with_cli(
    s: &Searcher,
    q: &str,
    semantic: bool,
    cli: &Cli,
) -> anyhow::Result<SearchResponse> {
    // `--semantic-only` / ASGREP_SEMANTIC_ONLY forces the semantic channel (ziij).
    do_search(s, q, semantic || cli.active_tuning().semantic_only)
}
