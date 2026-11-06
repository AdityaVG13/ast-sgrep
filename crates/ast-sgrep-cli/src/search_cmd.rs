//! Search / keyword / semantic / chain command helpers.

use crate::machine::{print_machine_json, print_machine_json_with_style, write_stdout_line};
use crate::{open_indexed_store, open_searcher, usage_error, Cli};
use anyhow::Context;
use ast_sgrep_core::{
    call_path::{find_call_path, CallPathConfig},
    chain::{expand_chain, ChainConfig},
    format_hit_line, SearchResponse, Searcher,
};
use std::path::Path;

pub(crate) fn run_chain(
    root: &Path,
    cli: &Cli,
    query: &str,
    max_work: usize,
) -> anyhow::Result<()> {
    let store = open_indexed_store(root, cli)?;
    let config = ChainConfig {
        limit: ast_sgrep_core::clamp_output_limit(cli.limit, ChainConfig::default().limit),
        top_n: 1,
        max_work,
        ..ChainConfig::default()
    };
    let r = expand_chain(&store, query, &config).context("chain search failed")?;
    if cli.json {
        return print_machine_json("chain", &r);
    }
    // Human output: agents often pipe through head; never panic on broken pipe.
    let suffix = if r.truncated {
        "; traversal incomplete; node_count is a lower bound"
    } else {
        ""
    };
    write_stdout_line(&format!(
        "chain {:?}: {} nodes, {} edges (max depth {}){}",
        r.query, r.node_count, r.edge_count, r.max_depth, suffix
    ))?;
    if let Some(work) = &r.work {
        write_stdout_line(&format!(
            "  omitted exploration: {:?}; work {}/{}",
            r.truncation_reasons, work.used, work.limit
        ))?;
    }
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
    let store = open_indexed_store(&args.root, cli)?;
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

/// Open the searcher for a possibly-FILE root (grep `QUERY FILE` habit).
/// For a file root, try each indexed ancestor nearest-first (stale or
/// foreign-layout indexes are skipped by falling through to the next
/// candidate), scoping with an exact-path filter relative to the winning
/// base. The remap note rides the machine envelope or stderr on SUCCESS
/// only — a total failure surfaces the last (outermost) error untouched.
/// An explicit --file-filter always wins (base still remaps, no note).
fn open_searcher_file_aware(root: &Path, cli: &Cli) -> anyhow::Result<ast_sgrep_core::Searcher> {
    if !root.is_file() {
        return open_searcher(root, cli);
    }
    // Canonicalize: the ancestor walk must terminate at the real filesystem
    // root, not at the empty string a relative path's ancestors end with.
    let root = &root
        .canonicalize()
        .with_context(|| format!("failed to resolve file root: {}", root.display()))?;
    let file_dir = root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let mut candidates: Vec<std::path::PathBuf> = file_dir
        .ancestors()
        .filter(|dir| dir.join(".asgrep").join("index.db").is_file())
        .map(Path::to_path_buf)
        .collect();
    if !candidates.contains(&file_dir) {
        candidates.push(file_dir);
    }
    let mut last_err = None;
    for base in candidates.iter() {
        let mut probe = cli.clone();
        if probe.tuning.file_filter.is_none() {
            if let Ok(rel) = root.strip_prefix(base) {
                let filter = rel.to_string_lossy().replace('\\', "/");
                if !filter.is_empty() {
                    probe.tuning.file_filter = Some(filter);
                }
            }
        }
        match open_searcher(base, &probe) {
            Ok(searcher) => {
                // Note only when a real index answered (the unindexed
                // file-dir fallback teaches through its own error path).
                if base.join(".asgrep").join("index.db").is_file()
                    && cli.tuning.file_filter.is_none()
                {
                    let note = format!(
                        "note: root `{}` is a file -- searched it via `{}` with an \
                         exact-path file filter. Next time: asgrep QUERY --root {} \
                         --file-filter <path>",
                        root.display(),
                        base.display(),
                        base.display()
                    );
                    crate::machine::push_pending_note(note.clone());
                    if !cli.machine_output_requested() {
                        eprintln!("{note}");
                    }
                }
                return Ok(searcher);
            }
            Err(error) => last_err = Some(error),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no candidate roots for {}", root.display())))
}

pub(crate) fn run_keyword_search(root: &Path, cli: &Cli, query: &str) -> anyhow::Result<()> {
    let response = open_searcher_file_aware(root, cli)?
        .search_lexical(query)
        .context("keyword search failed")?;
    print_search_response(
        "keyword",
        &response,
        ast_sgrep_plugins::OutputFormat::Native,
        cli,
    )
}

/// Sorted/deduped file paths with hits, in path order. Single source for the
/// ordering of both the boolean listing below and the per-file counts —
/// a file with N hits is listed once.
fn sorted_hit_files(response: &SearchResponse) -> Vec<String> {
    let mut files: Vec<String> = response.hits.iter().map(|h| h.file.clone()).collect();
    files.sort();
    files.dedup();
    files
}

/// The boolean-listing result set — matching paths, sorted and deduped (a
/// file with N hits is listed once). The parity lane compares PATH-SET
/// equality vs the reference's `--files-with-matches`; no perf claim (hits
/// are still computed, this only reshapes output).
fn files_from_hits(response: &SearchResponse) -> Vec<String> {
    sorted_hit_files(response)
}

/// The grep `-c` result set — `(path, matches)` sorted by path. Derived from
/// the same hit set as `files_from_hits`; zero-match files stay absent.
fn counts_from_hits(response: &SearchResponse) -> Vec<(String, u64)> {
    let mut counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for hit in &response.hits {
        *counts.entry(hit.file.clone()).or_insert(0) += 1;
    }
    // Ordering comes from the shared helper, not a second sort.
    sorted_hit_files(response)
        .into_iter()
        .map(|file| {
            let count = counts[&file];
            (file, count)
        })
        .collect()
}

/// Human-readable hit output: the boolean path listing when the flag is set,
/// one line per hit otherwise. Shared by the single- and multi-pattern paths.
fn print_human_hits(
    response: &SearchResponse,
    files_with_matches: bool,
    count: bool,
    miss: Option<ast_sgrep_plugins::MissContext>,
) -> anyhow::Result<()> {
    // A silent zero-hit answer is indistinguishable from a no-op: classify
    // the miss in human output too (the machine envelopes carry `why`).
    if response.hits.is_empty() {
        if let Some(ctx) = miss {
            let reason = ctx.reason();
            let first_filter = ctx.scope.first().map(|(name, _)| name.as_str());
            write_stdout_line(&format!(
                "no hits for `{}` (tried: {}) -- {}. Next: {}",
                response.query,
                ctx.tried.join("+"),
                reason.as_str(),
                reason.next_step(first_filter)
            ))?;
        }
        return Ok(());
    }
    if count {
        // grep -c habit: `path:count`, one line per file with hits.
        for (file, count) in counts_from_hits(response) {
            write_stdout_line(&format!("{file}:{count}"))?;
        }
        return Ok(());
    }
    if files_with_matches {
        for path in files_from_hits(response) {
            write_stdout_line(&path)?;
        }
        return Ok(());
    }
    for hit in &response.hits {
        write_stdout_line(&format_hit_line(hit))?;
    }
    Ok(())
}

pub(crate) fn run_search(
    root: &Path,
    cli: &Cli,
    query: &str,
    semantic: bool,
) -> anyhow::Result<()> {
    // Use the same resolved channel for execution, diagnostics, and output.
    let semantic_ch = semantic || cli.active_tuning().semantic_only;
    let ctx = if semantic_ch {
        "semantic search failed"
    } else {
        "search failed"
    };
    let response =
        do_search(&open_searcher_file_aware(root, cli)?, query, semantic_ch).context(ctx)?;
    let default = if semantic_ch {
        ast_sgrep_plugins::OutputFormat::Agent
    } else {
        ast_sgrep_plugins::OutputFormat::Native
    };
    print_search_response(
        if semantic_ch { "semantic" } else { "search" },
        &response,
        default,
        &cli,
    )
}

/// Multi-pattern ingress — N `--pattern` flags, ONE process (one index open,
/// one supervisor floor), ONE envelope. Each pattern runs the exact
/// single-pattern path (`Searcher::search` on its `pattern:` token), so
/// per-pattern hit sets are identical to N sequential invocations; hits are
/// grouped by pattern in flag order and tagged via `SearchHit::symbol`
/// (grouped hits). `--limit` applies per pattern. Fail-closed: a pattern the
/// single invocation would reject rejects the whole batch — no partial
/// envelope.
pub(crate) fn run_multi_pattern_search(
    root: &Path,
    cli: &Cli,
    patterns: &[String],
) -> anyhow::Result<()> {
    let searcher = open_searcher_file_aware(root, cli)?;
    let response = searcher
        .search_multi_pattern(patterns)
        .context("multi-pattern search failed")?;
    print_search_response(
        "search",
        &response,
        ast_sgrep_plugins::OutputFormat::Native,
        &cli,
    )
}

/// Insert an additive top-level ARRAY field into a machine envelope object
/// (no-op when the envelope is not an object). Shared by the `files` and
/// `counts` ingress paths so the two inserts cannot drift apart.
fn insert_array_field(
    value: &mut serde_json::Value,
    key: &str,
    iter: impl IntoIterator<Item = serde_json::Value>,
) {
    if let Some(object) = value.as_object_mut() {
        object.insert(
            key.into(),
            serde_json::Value::Array(iter.into_iter().collect()),
        );
    }
}

fn print_search_response(
    command: &str,
    response: &ast_sgrep_core::SearchResponse,
    default_format: ast_sgrep_plugins::OutputFormat,
    cli: &Cli,
) -> anyhow::Result<()> {
    if !cli.search_machine_output() {
        return print_human_hits(
            response,
            cli.active_tuning().files_with_matches,
            cli.active_tuning().count,
            Some(miss_context(command, cli, &response.query)),
        );
    }
    let tuning = cli.active_tuning();
    let format = resolve_output_format(tuning.format.as_deref(), default_format)?;
    let preview = tuning.preview.unwrap_or_default();
    let mut value = render_search_json(command, response, format, preview, cli);
    // Machine envelopes carry the same sorted/deduped path set as a
    // top-level `files` array (paths ARRAY, additive — hits and per-format
    // schemas untouched; the field only appears when the flag is passed).
    if tuning.files_with_matches {
        insert_array_field(
            &mut value,
            "files",
            files_from_hits(response)
                .into_iter()
                .map(serde_json::Value::String),
        );
    }
    // grep -c habit: same additive pattern as `files` above.
    if tuning.count {
        insert_array_field(
            &mut value,
            "counts",
            counts_from_hits(response)
                .into_iter()
                .map(|(file, count)| serde_json::json!({ "file": file, "count": count })),
        );
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
    use crate::cli_args::PreviewMode;
    use ast_sgrep_plugins::{
        format_response_with_budget, to_budgeted_compact_json, to_compact_miss_json, CompactBudget,
        DetailLevel, OutputBudget, OutputFormat,
    };

    let tuning = cli.active_tuning();

    let miss = miss_context(command, cli, &response.query);
    // Compact miss envelope is cheaper than an empty hit list for agents.
    if format == OutputFormat::Compact && response.hits.is_empty() {
        return to_compact_miss_json(&response.query, &miss);
    }

    // Explicit token budget wins over preview size defaults, not explicit none.
    if format == OutputFormat::Compact {
        if let Some(max_tokens) = tuning.budget_tokens {
            let detail = match preview {
                PreviewMode::None => DetailLevel::Metadata,
                PreviewMode::Full => DetailLevel::Full,
                PreviewMode::Short => DetailLevel::Block,
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
    let mut envelope = format_response_with_budget(response, format, excerpt, budget);
    // Zero-hit answers classify the miss on every machine format, not only
    // the compact envelope whose schema has always documented `why` — a
    // silent empty hit list is indistinguishable from a no-op.
    if response.hits.is_empty() {
        let reason = miss.reason();
        let first_filter = miss.scope.first().map(|(name, _)| name.as_str());
        envelope["why"] = serde_json::Value::String(reason.as_str().to_owned());
        envelope["next"] = serde_json::Value::String(reason.next_step(first_filter).to_owned());
    }
    envelope
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

/// Command name -> search channel reported in `MissContext::tried`.
/// The default path fuses channels rather than running just one.
const MISS_CHANNEL_TABLE: &[(&str, &str)] =
    &[("keyword", "lexical"), ("semantic", "semantic")];

/// Describe a zero-hit CLI search: which channel ran, and what scoped it (6a3i).
fn miss_context(command: &str, cli: &Cli, query: &str) -> ast_sgrep_plugins::MissContext {
    let mut scope = Vec::new();
    if let Some(lang) = &cli.lang {
        scope.push(("lang".to_owned(), lang.clone()));
    }
    ast_sgrep_plugins::MissContext {
        // Report the CHANNEL that ran, not the command name the user typed.
        tried: vec![MISS_CHANNEL_TABLE
            .iter()
            .find(|(name, _)| *name == command)
            .map(|(_, channel)| (*channel).to_owned())
            .unwrap_or_else(|| "hybrid".to_owned())],
        unavailable: Vec::new(),
        scope,
        indexed_files: None,
        // Bodyless-args pattern misses teach their contract instead of a
        // bare 0 (br-yjw).
        pattern_miss: ast_sgrep_plugins::is_bodyless_fn_args_pattern(query),
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
