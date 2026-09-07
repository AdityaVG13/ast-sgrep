//! Search / keyword / semantic / chain command helpers.

use crate::machine::{print_machine_json, print_machine_json_with_style, write_stdout_line};
use crate::{ensure_existing_root, open_indexed_store, open_searcher, usage_error, Cli};
use anyhow::Context;
use ast_sgrep_core::{
    call_path::{find_call_path, CallPathConfig},
    chain::{expand_chain, ChainConfig},
    format_hit_line, SearchResponse, Searcher,
};
use std::path::Path;

pub(crate) fn run_chain(root: &Path, cli: &Cli, query: &str) -> anyhow::Result<()> {
    let root = ensure_existing_root(root, cli)?;
    let store = open_indexed_store(&root, cli)?;
    let config = ChainConfig {
        limit: ast_sgrep_core::clamp_output_limit(cli.limit, ChainConfig::default().limit),
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

pub(crate) fn run_call_path(args: &crate::cli_args::CallPathArgs, cli: &Cli) -> anyhow::Result<()> {
    let root = ensure_existing_root(&args.root, cli)?;
    let store = open_indexed_store(&root, cli)?;
    let response = find_call_path(
        &store,
        &args.source,
        &args.sink,
        &CallPathConfig {
            max_depth: args.max_depth,
            max_nodes: usize::try_from(args.max_nodes).expect("clap max-nodes bound fits usize"),
            max_edges: usize::try_from(args.max_edges).expect("clap max-edges bound fits usize"),
        },
    )
    .context("call-path search failed")?;
    if cli.json {
        return print_machine_json("call-path", &response);
    }
    if !response.found {
        let suffix = if response.truncated {
            "; resource cap reached"
        } else {
            ""
        };
        write_stdout_line(&format!(
            "no call path from {:?} to {:?} within depth {}{} (call graph only)",
            response.source, response.sink, response.max_depth, suffix
        ))?;
        return Ok(());
    }
    write_stdout_line(&format!(
        "call-graph path {:?} -> {:?}: {} hops (no value flow)",
        response.source,
        response.sink,
        response.path.len()
    ))?;
    for hop in &response.path {
        write_stdout_line(&format!(
            "  {}:{} {} -> {} [{}]",
            hop.file,
            hop.line,
            hop.caller,
            hop.callee,
            hop.resolution.as_str()
        ))?;
    }
    Ok(())
}

pub(crate) fn run_keyword_search(root: &Path, cli: &Cli, query: &str) -> anyhow::Result<()> {
    let response = open_searcher(root, cli)?
        .search_lexical(query)
        .context("keyword search failed")?;
    if !cli.search_machine_output() {
        if cli.active_tuning().files_with_matches {
            for path in files_from_hits(&response) {
                write_stdout_line(&path)?;
            }
            return Ok(());
        }
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

/// F-SG-RUN-FILES-WITH-MATCHES (pass 31): the boolean-listing result set —
/// matching paths, sorted and deduped (a file with N hits is listed once).
/// The sg-parity lane compares PATH-SET equality vs `sg run
/// --files-with-matches`; no perf claim (hits are still computed, this only
/// reshapes output).
fn files_from_hits(response: &SearchResponse) -> Vec<String> {
    let mut files: Vec<String> = response.hits.iter().map(|h| h.file.clone()).collect();
    files.sort();
    files.dedup();
    files
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
        if cli.active_tuning().files_with_matches {
            for path in files_from_hits(&response) {
                write_stdout_line(&path)?;
            }
            return Ok(());
        }
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

/// EXP-013 (GA-21, pass 29): multi-pattern ingress — N `--pattern` flags, ONE
/// process (one index open, one supervisor floor), ONE envelope. Each pattern
/// runs the exact single-pattern path (`Searcher::search` on its `pattern:`
/// token), so per-pattern hit sets are identical to N sequential invocations;
/// hits are grouped by pattern in flag order and tagged via `SearchHit::symbol`
/// (grouped hits). `--limit` applies per pattern. Fail-closed (H-CONF-006
/// rule): a pattern the single invocation would reject rejects the whole
/// batch — no partial envelope.
pub(crate) fn run_multi_pattern_search(
    root: &Path,
    cli: &Cli,
    patterns: &[String],
) -> anyhow::Result<()> {
    let searcher = open_searcher(root, cli)?;
    let response = searcher
        .search_multi_pattern(patterns)
        .context("multi-pattern search failed")?;
    if !cli.search_machine_output() {
        if cli.active_tuning().files_with_matches {
            for path in files_from_hits(&response) {
                write_stdout_line(&path)?;
            }
            return Ok(());
        }
        for hit in &response.hits {
            write_stdout_line(&format_hit_line(hit))?;
        }
        return Ok(());
    }
    let format = resolve_output_format(
        cli.active_tuning().format.as_deref(),
        ast_sgrep_plugins::OutputFormat::Native,
    )?;
    print_search_response("search", &response, format, cli)
}

fn print_search_response(
    command: &str,
    response: &ast_sgrep_core::SearchResponse,
    format: ast_sgrep_plugins::OutputFormat,
    cli: &Cli,
) -> anyhow::Result<()> {
    let tuning = cli.active_tuning();
    let preview = tuning.preview.unwrap_or_default();
    let mut value = render_search_json(command, response, format, preview, cli);
    // F-SG-RUN-FILES-WITH-MATCHES (pass 31): machine envelopes carry the same
    // sorted/deduped path set as a top-level `files` array (P3 decision: paths
    // ARRAY, additive — hits and per-format schemas untouched; the field only
    // appears when the flag is passed).
    if tuning.files_with_matches {
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "files".into(),
                serde_json::Value::Array(
                    files_from_hits(response)
                        .into_iter()
                        .map(serde_json::Value::String)
                        .collect(),
                ),
            );
        }
    }
    print_machine_json_with_style(
        command,
        value,
        format == ast_sgrep_plugins::OutputFormat::Compact,
        true,
        0,
    )
}

fn render_search_json(
    command: &str,
    response: &ast_sgrep_core::SearchResponse,
    format: ast_sgrep_plugins::OutputFormat,
    preview: crate::cli_args::PreviewMode,
    cli: &Cli,
) -> serde_json::Value {
    use ast_sgrep_plugins::{
        format_response_with_budget, to_budgeted_compact_json, to_compact_miss_json, CompactBudget,
        DetailLevel, OutputBudget, OutputFormat,
    };
    use crate::cli_args::PreviewMode;

    let tuning = cli.active_tuning();

    // Compact miss envelope is cheaper than an empty hit list for agents.
    if format == OutputFormat::Compact && response.hits.is_empty() {
        return to_compact_miss_json(&response.query, &miss_context(command, cli));
    }

    // Explicit token budget wins over preview defaults for compact output.
    if format == OutputFormat::Compact {
        if let Some(max_tokens) = tuning.budget_tokens {
            let detail = match preview {
                PreviewMode::Full => DetailLevel::Full,
                _ => DetailLevel::Block,
            };
            return to_budgeted_compact_json(
                response,
                OutputBudget {
                    max_tokens,
                    default_detail: detail,
                },
            );
        }
        return match preview {
            PreviewMode::None => {
                let mut env = format_response_with_budget(
                    response,
                    OutputFormat::Compact,
                    0,
                    CompactBudget {
                        per_result_tokens: 0,
                        response_tokens: 0,
                    },
                );
                blank_compact_snippets(&mut env);
                env
            }
            PreviewMode::Short => format_response_with_budget(
                response,
                OutputFormat::Compact,
                0,
                CompactBudget {
                    per_result_tokens: tuning.snippet_tokens,
                    response_tokens: tuning.response_snippet_tokens,
                },
            ),
            PreviewMode::Full => to_budgeted_compact_json(
                response,
                OutputBudget {
                    max_tokens: 8_192,
                    default_detail: DetailLevel::Full,
                },
            ),
        };
    }

    let (excerpt, budget) = match preview {
        PreviewMode::None => (
            0,
            CompactBudget {
                per_result_tokens: 0,
                response_tokens: 0,
            },
        ),
        PreviewMode::Short => (
            tuning.excerpt_lines,
            CompactBudget {
                per_result_tokens: tuning.snippet_tokens,
                response_tokens: tuning.response_snippet_tokens,
            },
        ),
        PreviewMode::Full => (
            tuning.excerpt_lines.max(3),
            CompactBudget {
                per_result_tokens: tuning.snippet_tokens.max(256),
                response_tokens: tuning.response_snippet_tokens.max(2_048),
            },
        ),
    };
    format_response_with_budget(response, format, excerpt, budget)
}

fn blank_compact_snippets(envelope: &mut serde_json::Value) {
    let Some(hits) = envelope.get_mut("h").and_then(|v| v.as_array_mut()) else {
        return;
    };
    for hit in hits {
        let Some(row) = hit.as_array_mut() else {
            continue;
        };
        if let Some(snippet) = row.get_mut(4) {
            *snippet = serde_json::Value::String(String::new());
        }
    }
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
