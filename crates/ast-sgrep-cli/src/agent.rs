use crate::{index_options, Cli};
use anyhow::Context;
use ast_sgrep_core::Indexer;
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use std::io::{self, IsTerminal};
use std::path::Path;
const TOOL: &str = "asgrep";
const SCHEMA: &str = "1.0.0";
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
    command: RobotDocsCommand,
}
#[derive(Parser)]
pub(crate) struct DoctorArgs {
    #[arg(long)]
    pub(crate) json: bool,
    #[arg(long = "robot-triage")]
    pub(crate) robot_triage: bool,
}
pub(crate) fn run_capabilities(cli: &Cli, args: &CapabilitiesArgs) -> anyhow::Result<()> {
    if !cli.json && !args.json {
        eprintln!("hint: agents should run `{TOOL} capabilities --json` (stdout is JSON only)");
        return Err(crate::usage_error(
            "capabilities requires --json for deterministic agent output",
        ));
    }
    crate::print_machine_json("capabilities", capabilities_json(cli)?)
}
pub(crate) fn run_robot_docs(_cli: &Cli, args: &RobotDocsArgs) -> anyhow::Result<()> {
    match args.command {
        RobotDocsCommand::Guide => {
            print_robot_guide();
            Ok(())
        }
    }
}
pub(crate) fn run_doctor(cli: &Cli, root: &Path, args: &DoctorArgs) -> anyhow::Result<()> {
    if cli.json || args.robot_triage || args.json {
        return crate::print_machine_json("doctor", doctor_triage_json(cli, root)?);
    }
    eprintln!("hint: use `{TOOL} doctor --json` or `{TOOL} doctor --robot-triage` for agent-readable triage");
    Err(crate::usage_error(
        "doctor requires --json or --robot-triage",
    ))
}
pub(crate) fn capabilities_json(_cli: &Cli) -> anyhow::Result<Value> {
    Ok(json!({
        "schema_version": SCHEMA, "tool": TOOL, "version": env!("CARGO_PKG_VERSION"),
        "description": "Polyglot hybrid code search (lexical + structural + semantic)",
        "agent_contract": {"stdout": "data payloads only when --json / robot modes are set", "stderr": "human hints and diagnostics", "deterministic": "stable JSON key ordering via serde_json; disable color with NO_COLOR=1"},
        "commands": [
            {"name": "search", "usage": "asgrep [--json] [--format agent] \"QUERY\" [ROOT]", "robot_output": "--json [--format native|agent|agent-capsule|github|gitlab]"},
            {"name": "semantic", "usage": "asgrep semantic \"QUERY\" [ROOT] [--json]", "robot_output": "--json (defaults to agent format)"},
            {"name": "index", "usage": "asgrep index [ROOT] [--json]"}, {"name": "status", "usage": "asgrep status [ROOT] [--json]"},
            {"name": "reindex", "usage": "asgrep reindex [ROOT] [--json]"}, {"name": "capabilities", "usage": "asgrep capabilities --json"},
            {"name": "version", "usage": "asgrep version --json"}, {"name": "robot-docs", "usage": "asgrep robot-docs guide"},
            {"name": "doctor", "usage": "asgrep doctor [ROOT] --json | --robot-triage"},
        ],
        "global_flags": ["--json", "--root", "--limit", "--index-path", "--lang", "--format", "--no-embed", "--tantivy", "--ann-threshold"],
        "environment": ["ASGREP_LIMIT", "ASGREP_INDEX_PATH", "ASGREP_NO_EMBED", "ASGREP_CLOUD_EMBED", "ASGREP_OLLAMA_EMBED", "ASGREP_SEMANTIC_ONLY", "ASGREP_TANTIVY", "ASGREP_ANN_THRESHOLD", "NO_COLOR", "CI"],
        "output_limits": {"max_results": 1000, "max_excerpt_lines": 100, "max_error_message_chars": 4096},
        "search_formats": ["native", "agent", "agent-capsule", "github", "gitlab"],
        "exit_codes": [{"code": 0, "meaning": "success"}, {"code": 1, "meaning": "user input / usage error"}, {"code": 2, "meaning": "index or search operation failed"}],
        "canonical_tasks": ["asgrep index . && asgrep --json --format agent \"where is auth refreshed\" .", "asgrep status . --json", "asgrep doctor . --robot-triage"],
    }))
}
fn doctor_triage_json(cli: &Cli, root: &Path) -> anyhow::Result<Value> {
    let root = crate::effective_root(cli, root);
    let mut issues = Vec::<Value>::new();
    let mut next = Vec::<&'static str>::new();
    let status =
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
        next.push("asgrep --json --format agent \"<your query>\" .");
    }
    next.extend(["asgrep capabilities --json", "asgrep robot-docs guide"]);
    Ok(
        json!({"schema_version": SCHEMA, "tool": TOOL, "robot_triage": true, "root": root, "index_path": cli.index_path, "status": status, "issues": issues, "suggested_commands": next, "healthy": issues.is_empty(), "tty": io::stdout().is_terminal()}),
    )
}
fn print_robot_guide() {
    print!(
        r#"# asgrep — agent handbook (robot-docs guide)
## Quick start
1. `asgrep index . --json` — build or refresh the index (required once per checkout).
2. `asgrep doctor . --robot-triage` — one-shot health + suggested commands.
3. `asgrep --json --format agent "natural language intent" .` — ranked hits with follow-up hints.
## Subcommands (always use explicit subcommands; bare tokens are treated as search queries)
- `index`, `status`, `reindex`, `semantic`, `bench`, `watch`
- `capabilities --json` — machine-readable contract
- `robot-docs guide` — this document
- `doctor --json` or `doctor --robot-triage` — triage bundle
## JSON / automation
- Pass `--json` on any read-side command for stdout JSON.
- Prefer `--format agent` for LLM consumption.
- Diagnostics and hints go to stderr; parse stdout only.
## Exit codes
- 0 success
- 1 usage / unknown subcommand / missing required args
- 2 index or search failure
## Environment
`ASGREP_INDEX_PATH`, `ASGREP_LIMIT`, `ASGREP_NO_EMBED`, `NO_COLOR`, `CI`
## Common mistakes
- `asgrep capabilities` without `capabilities` subcommand runs a search for the word "capabilities".
  Use: `asgrep capabilities --json`
- Missing index: run `asgrep index .` before searching.
"#
    );
}
pub(crate) fn query_looks_like_subcommand_typo(query: &str) -> Option<&'static str> {
    let q = query.trim();
    if q.is_empty() || q.contains(' ') {
        return None;
    }
    let lower = q.to_ascii_lowercase();
    const EXACT: &[&str] = &[
        "index",
        "status",
        "reindex",
        "bench",
        "watch",
        "semantic",
        "capabilities",
        "capability",
        "robot-docs",
        "robot_docs",
        "doctor",
        "help",
    ];
    if let Some(c) = EXACT.iter().find(|c| lower == **c) {
        return Some(*c);
    }
    const TYPOS: &[(&str, &str)] = &[
        ("capabilites", "capabilities"),
        ("capabilty", "capabilities"),
        ("statu", "status"),
        ("indx", "index"),
        ("reindx", "reindex"),
        ("sematic", "semantic"),
        ("doctr", "doctor"),
    ];
    TYPOS.iter().find(|(t, _)| lower == *t).map(|(_, f)| *f)
}
pub(crate) fn print_agent_help_footer() {
    eprintln!("\nAgent surfaces: {TOOL} capabilities --json | {TOOL} robot-docs guide | {TOOL} doctor --robot-triage");
    eprintln!(
        "Exit codes: 0=ok, 1=usage, 2=operation failed. Use --json for machine-readable stdout."
    );
}
