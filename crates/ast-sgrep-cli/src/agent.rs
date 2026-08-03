use crate::{index_options, Cli};
use anyhow::Context;
use ast_sgrep_core::Indexer;
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use std::io::{self, IsTerminal};
use std::path::Path;
const TOOL: &str = "asgrep";
#[derive(Parser)]
pub(crate) struct CapabilitiesArgs {
    #[arg(long)]
    pub(crate) json: bool,
}
#[derive(Subcommand)]
pub(crate) enum RobotDocsCommand {
    Guide,
}
#[derive(Parser)]
pub(crate) struct RobotDocsArgs {
    #[command(subcommand)]
    command: Option<RobotDocsCommand>,
}
#[derive(Parser)]
pub(crate) struct DoctorArgs {
    #[arg(long)]
    pub(crate) json: bool,
    #[arg(long = "robot-triage")]
    pub(crate) robot_triage: bool,
}
pub(crate) fn run_capabilities(cli: &Cli, args: &CapabilitiesArgs) -> anyhow::Result<()> {
    let _ = (cli.json, args.json);
    crate::print_machine_json("capabilities", capabilities_json(cli)?)
}
pub(crate) fn run_robot_docs(_cli: &Cli, args: &RobotDocsArgs) -> anyhow::Result<()> {
    match args.command.as_ref().unwrap_or(&RobotDocsCommand::Guide) {
        RobotDocsCommand::Guide => {
            print_robot_guide();
            Ok(())
        }
    }
}
pub(crate) fn run_doctor(cli: &Cli, root: &Path, args: &DoctorArgs) -> anyhow::Result<()> {
    let _ = (cli.json, args.json, args.robot_triage);
    let triage = doctor_triage_json(cli, root)?;
    let healthy = triage
        .get("healthy")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if healthy {
        crate::print_machine_json("doctor", triage)
    } else {
        // Fail-closed: never emit ok:true / exit 0 when healthy:false (s6ze.1).
        crate::print_machine_json_status("doctor", triage, false, 2)?;
        std::process::exit(2);
    }
}
pub(crate) fn capabilities_json(_cli: &Cli) -> anyhow::Result<Value> {
    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Polyglot hybrid code search (lexical + structural + semantic)",
        "agent_contract": {"stdout": "one data payload in machine/default-agent modes", "stderr": "empty in machine modes; human diagnostics otherwise", "deterministic": "stable JSON key ordering via serde_json; disable color with NO_COLOR=1"},
        "commands": [
            {"name": "search", "aliases": ["find", "query"], "usage": "asgrep search \"QUERY\" [ROOT] --format compact (or omit search)", "robot_output": "--json is implied by --format"},
            {"name": "keyword", "usage": "asgrep keyword \"QUERY\" [ROOT] [--json]", "robot_output": "--format implies JSON; formats: native|agent|agent-capsule|compact|github|gitlab"},
            {"name": "semantic", "usage": "asgrep semantic \"QUERY\" [ROOT] [--json]", "robot_output": "--format implies JSON; --json alone defaults to agent"},
            {"name": "index", "usage": "asgrep index [ROOT] [--json]"}, {"name": "status", "usage": "asgrep status [ROOT] [--json]"},
            {"name": "reindex", "usage": "asgrep reindex [ROOT] [--json]"},
            {"name": "bench", "usage": "asgrep bench [ROOT] --suite default --fixture sample --json"},
            {"name": "watch", "usage": "asgrep watch [ROOT] --debounce-ms 300"},
            {"name": "chain", "usage": "asgrep chain \"SYMBOL\" [ROOT] --json"},
            {"name": "eval", "usage": "asgrep eval --gold FILE [ROOT] --json"},
            {"name": "capabilities", "usage": "asgrep capabilities"},
            {"name": "version", "usage": "asgrep version --json"},
            {"name": "robot-docs", "usage": "asgrep robot-docs"},
            {"name": "doctor", "usage": "asgrep doctor [ROOT]"},
        ],
        "global_flags": ["--json", "--robot-help", "--root", "--limit", "--index-path", "--lang", "--format", "--excerpt-lines", "--snippet-tokens", "--response-snippet-tokens", "--no-embed", "--cloud-embed", "--ollama-embed", "--neural-embed", "--semantic-only", "--tantivy", "--ann-threshold", "--ann-probes", "--rerank", "--rerank-top-k"],
        "environment": ["ASGREP_LIMIT", "ASGREP_INDEX_PATH", "ASGREP_NO_EMBED", "ASGREP_CLOUD_EMBED", "ASGREP_OLLAMA_EMBED", "ASGREP_NEURAL_EMBED", "ASGREP_SEMANTIC_ONLY", "ASGREP_TANTIVY", "ASGREP_ANN_THRESHOLD", "ASGREP_ANN_PROBES", "ASGREP_RERANK", "ASGREP_RERANK_TOP_K", "NO_COLOR", "CI"],
        "environment_bool_values": ["1", "0", "true", "false", "yes", "no", "on", "off"],
        "sibling_binaries": [{"name":"asgrep-mcp","purpose":"MCP stdio server"},{"name":"asgrep-lsp","purpose":"Language Server Protocol server"}],
        "aliases": ["ast-sgrep"],
        "output_limits": {"max_results": 1000, "max_excerpt_lines": 100, "default_snippet_tokens": 96, "default_response_snippet_tokens": 768, "max_snippet_tokens": 4096, "max_response_snippet_tokens": 65536, "max_error_message_chars": 4096},
        "search_formats": ["native", "agent", "agent-capsule", "compact", "github", "gitlab"],
        "exit_codes": [{"code": 0, "meaning": "success"}, {"code": 1, "meaning": "user input / usage error"}, {"code": 2, "meaning": "index or search operation failed"}],
        "canonical_tasks": ["asgrep index . && asgrep --json --format compact \"where is auth refreshed\" .", "asgrep status . --json", "asgrep doctor . --robot-triage"],
    }))
}
fn doctor_triage_json(cli: &Cli, root: &Path) -> anyhow::Result<Value> {
    crate::ensure_unambiguous_root(root, cli)?;
    let root = crate::effective_root(cli, root);
    let mut issues = Vec::<Value>::new();
    let mut next = Vec::<&'static str>::new();
    let status = if !root.is_dir() {
        issues.push(json!({"kind": "missing_root", "message": format!("project root does not exist or is not a directory: {}", root.display())}));
        None
    } else {
        match Indexer::new(index_options(&root, cli)).context("failed to open index for doctor") {
            Ok(idx) => match idx.store().status() {
                Ok(status) => Some(status),
                Err(e) => {
                    issues.push(json!({"kind": "status_read", "message": e.to_string()}));
                    None
                }
            },
            Err(e) => {
                issues.push(json!({"kind": "index_open", "message": e.to_string()}));
                None
            }
        }
    };
    if status.is_none() {
        next.push("asgrep index . --json");
    } else if let Some(ref st) = status {
        if st.file_count == 0 {
            issues.push(
                json!({"kind": "empty_index", "message": "index exists but indexes zero files"}),
            );
            next.push("asgrep index . --json");
        }
        if !st.semantic_ivf_present && st.semantic_chunk_count > 0 {
            issues.push(json!({"kind": "semantic_ivf_missing", "message": "semantic chunks present but IVF sidecar not built (may be below ANN threshold)"}));
        }
    }
    if next.is_empty() {
        next.push("asgrep --json --format compact \"<your query>\" .");
    }
    next.extend(["asgrep capabilities --json", "asgrep robot-docs guide"]);
    Ok(
        json!({"robot_triage": true, "root": root, "index_path": cli.index_path, "status": status, "issues": issues, "suggested_commands": next, "healthy": issues.is_empty(), "tty": io::stdout().is_terminal()}),
    )
}
pub(crate) fn print_robot_guide() {
    print!(
        r#"# asgrep — agent handbook (robot-docs guide)
## Quick start
1. `asgrep index . --json` — build or refresh the index (required once per checkout).
2. `asgrep doctor . --robot-triage` — one-shot health + suggested commands.
3. `asgrep --json --format compact "natural language intent" .` — ranked hits with bounded snippets.
## Subcommands
- `search` (`find`, `query`) or a bare query; `keyword`; `semantic`; `chain`
- `index`, `status`, `reindex`, `bench`, `watch`, `eval --gold FILE`
- `capabilities` — machine-readable contract (JSON by default)
- `robot-docs` — this document (`--robot-help` is an alias)
- `doctor [ROOT]` — machine-readable triage (JSON by default)
## Integrations
- `asgrep-mcp` — MCP stdio server for agents
- `asgrep-lsp` — Language Server Protocol server
- `ast-sgrep` — alias of the `asgrep` executable
## JSON / automation
- Pass `--json` on any read-side command; `--format` implies JSON.
- Prefer `--format compact` for bounded LLM consumption; expand selected IDs with `code_read`.
- Machine mode emits one JSON value on stdout and no duplicate stderr diagnostics.
## Exit codes
- 0 success
- 1 usage / unknown subcommand / missing required args
- 2 index or search failure
## Environment
`ASGREP_INDEX_PATH`, `ASGREP_LIMIT`, `ASGREP_NO_EMBED`, `NO_COLOR`, `CI`
## Common mistakes
- Missing or empty index: run `asgrep index . --json` before searching.
- Missing ROOT is an operational error; it is never reported as an empty result.
"#
    );
}
pub(crate) fn query_looks_like_subcommand_typo(query: &str) -> Option<&'static str> {
    let q = query.trim();
    if q.is_empty() || q.contains(' ') {
        return None;
    }
    let lower = q.to_ascii_lowercase();
    const ALIASES: &[(&str, &str)] = &[
        ("capability", "capabilities"),
        ("robot_docs", "robot-docs"),
        ("robotdocs", "robot-docs"),
    ];
    if let Some((_, canonical)) = ALIASES.iter().find(|(alias, _)| *alias == lower) {
        return Some(*canonical);
    }
    const COMMANDS: &[&str] = &[
        "index",
        "status",
        "reindex",
        "search",
        "keyword",
        "semantic",
        "chain",
        "bench",
        "watch",
        "capabilities",
        "version",
        "robot-docs",
        "doctor",
        "eval",
    ];
    COMMANDS
        .iter()
        .map(|command| (*command, edit_distance(&lower, command)))
        .min_by_key(|(_, distance)| *distance)
        .filter(|(command, distance)| *distance <= 1 || is_adjacent_transposition(&lower, command))
        .map(|(command, _)| command)
}

fn is_adjacent_transposition(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mismatches = left
        .bytes()
        .zip(right.bytes())
        .enumerate()
        .filter_map(|(index, (left, right))| (left != right).then_some((index, left, right)))
        .collect::<Vec<_>>();
    matches!(
        mismatches.as_slice(),
        [(first_index, first_left, first_right), (second_index, second_left, second_right)]
            if *second_index == *first_index + 1
                && first_left == second_right
                && first_right == second_left
    )
}

fn edit_distance(left: &str, right: &str) -> usize {
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_byte) in left.bytes().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_byte) in right.bytes().enumerate() {
            current[right_index + 1] = if left_byte == right_byte {
                previous[right_index]
            } else {
                1 + previous[right_index]
                    .min(previous[right_index + 1])
                    .min(current[right_index])
            };
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}
pub(crate) fn print_agent_help_footer() {
    eprintln!("\nAgent surfaces: {TOOL} capabilities --json | {TOOL} robot-docs guide | {TOOL} doctor --robot-triage");
    eprintln!(
        "Exit codes: 0=ok, 1=usage, 2=operation failed. Use --json for machine-readable stdout."
    );
}
