use crate::Cli;
use clap::{Parser, Subcommand};

pub(crate) use crate::doctor::{DoctorArgs, run_doctor};

#[path = "capabilities.rs"]
mod capabilities;
#[path = "typo_recovery.rs"]
mod typo_recovery;

pub(crate) use self::capabilities::{capabilities_json, command_example};
pub(crate) use self::typo_recovery::{edit_distance, query_looks_like_subcommand_typo, rewrite_typos};
use self::capabilities::BARE_DEFAULT_EXAMPLE;
use self::typo_recovery::MODE_GAP_NAMES;

const TOOL: &str = "asgrep";
#[derive(Parser, Clone)]
pub(crate) struct CapabilitiesArgs {
    #[arg(long)]
    pub(crate) json: bool,
}
#[derive(Subcommand, Clone)]
pub(crate) enum RobotDocsCommand {
    Guide,
}
#[derive(Parser, Clone)]
pub(crate) struct RobotDocsArgs {
    #[command(subcommand)]
    command: Option<RobotDocsCommand>,
}

pub(crate) fn run_capabilities(cli: &Cli, args: &CapabilitiesArgs) -> anyhow::Result<()> {
    let _ = (cli.json, args.json);
    crate::print_machine_json("capabilities", capabilities_json(cli)?)
}
pub(crate) fn run_robot_docs(cli: &Cli, args: &RobotDocsArgs) -> anyhow::Result<()> {
    match args.command.as_ref().unwrap_or(&RobotDocsCommand::Guide) {
        RobotDocsCommand::Guide => emit_robot_guide(cli),
    }
}

/// Agent handbook body (markdown). Single source for human stdout and --json envelope.
pub(crate) fn robot_guide_markdown() -> &'static str {
    // Handbook text is kept in sync with clap/capabilities (hceb): prefer capabilities --json
    // for the authoritative command/flag catalog derived from Cli::command().
    r#"# asgrep — agent handbook (robot-docs guide)
## Agent triad (start here)
1. `asgrep capabilities --json` — authoritative command/flag/env contract (derived from clap).
2. `asgrep robot-docs guide` — this handbook.
3. `asgrep --robot-triage` (alias: `asgrep doctor --robot-triage`, `asgrep --robot-next`) — health + recovery in one call.
## Quick start
1. `asgrep --json --format compact "natural language intent" .` — ranked hits with bounded snippets. Search is read-only; it does not auto-index.
2. `asgrep index . --json` — build or refresh the index. Pass `--auto-index` on search only when you explicitly want search to write.
## Indexed source / freshness
- Do not spawn `rg` on indexed source. Use `literal:<term>` for exact substring presence and unprefixed search for ranked code navigation.
- For a long-running CLI session, run `asgrep watch <root>`. A pending batch starts after the debounce quiet period or after at most three debounce windows under continuous events; indexing time still depends on the project.
- Pi and Code Mode refresh before search, with a configurable 30-second correctness lease by default. LSP applies document open/change/save/close notifications before processing the next request.
- Ripgrep remains the tool for logs and unindexed or unsupported files. ast-sgrep never spawns it as a compatibility layer.
## Subcommands
See `capabilities --json` → `commands` (complete clap catalog). Notable: `search`/`find`/`query`, `keyword`, `semantic`, `chain`, `call-path`, `index`/`reindex` (`--dry-run`), `codemod`, `status`, `bench`, `watch`, `eval`, `doctor`, `version`.
## Integrations / sibling binaries
- `asgrep-mcp` — MCP stdio server (`ASGREP_ROOT`, tools: keyword/ast/semantic search, index_repo, code_read)
- `asgrep-lsp` — Language Server Protocol server
- `ast-sgrep` — alias of the `asgrep` executable
## Root specification
- Canonical: positional `ROOT` on the subcommand (or bare-search ROOT).
- Alias: `--root ROOT`. Conflicting `--root` + positional ROOT → usage error.
## JSON / automation
- `--json` / `-j` emit one JSON value on stdout. `--format` implies `--json`. Prefer `--format compact` for bounded LLM consumption.
- Recovered spellings: `--jason`, `--machine`, `--output-json`, `--format=json` → `--json`.
- Machine mode emits no duplicate stderr diagnostics. Agent envelopes omit TTY and wall-clock fields.
## Index cancel / dry-run
- `asgrep index --dry-run` / `asgrep reindex --dry-run` report planned work without mutating the index.
- `asgrep codemod --pattern 'legacy($ARG)' --rewrite 'modern($ARG)' --dry-run .` emits a JSON edit plan without writing. Apply with `--yes` (alias `--force`): `asgrep codemod --yes --pattern 'legacy($ARG)' --rewrite 'modern($ARG)' .` commits one source transaction, then a separate index refresh. If refresh fails, source edits remain applied and the command reports `asgrep index` as recovery.
- Index writes are transactional; an interrupted uncommitted write is rolled back when SQLite recovers.
## Exit codes
- 0 success · 1 usage · 2 index/search failure
## Confirmation / first-try recovery
- Source writes: `asgrep codemod` apply requires `--yes` (alias `--force`). Always plan with `--dry-run` first.
- `--help` after-help names the triad, `-j/--json`, and the `--yes` gate.
- Typos: nearby flag/command spellings rewrite before clap (`--jsno` → `--json`, `capabilites` → `capabilities`, `docs` → `robot-docs`, `--dryrun` → `--dry-run`).
## Environment
See `capabilities --json` → `environment`. Common: `ASGREP_INDEX_PATH`, `ASGREP_LIMIT`, `ASGREP_NO_EMBED`, `ASGREP_NO_AUTO_INDEX`, `ASGREP_DURABILITY`, `NO_COLOR`, `CI`, `TERM=dumb`, `SOURCE_DATE_EPOCH` (bench history timestamps). `CI=1` and `TERM=dumb` suppress progress chatter (`asgrep: indexing …`) so logs stay quiet without `--json`.
## Ops footguns (privileged sinks)
- `ASGREP_INDEX_PATH` / `--index-path` is a **privileged sink**: any absolute writable path is accepted. Treat it like a database URL; do not point it at untrusted locations.
- Index rebuilds are in-place on the default `.asgrep/` DB or a pinned `ASGREP_INDEX_PATH` (SQLite transactional rollback). There is no build-then-swap generation layout. Pinning only chooses which file; it does not change atomicity.
- `ASGREP_DURABILITY=fast-unsafe` (or `--durability fast-unsafe`) opts into power-loss corruption risk during write batches. `asgrep doctor` / `status` surface it; MCP/Code Mode inherit the env.
- MCP and Code Mode / NAPI jail tool `root` under the configured workspace (`escapes configured workspace`). Host duty remains: set `ASGREP_ROOT` / Session root intentionally; NAPI inherits Session (not a free root).
## Common mistakes
- Empty index: run `asgrep index <root> --json`. Search does not auto-index; pass `--auto-index` only when search may write.
- Missing ROOT is an operational error; it is never reported as an empty result.
- Full rebuild: prefer `asgrep reindex --dry-run <root> --json` before `reindex`.
- Output format is not `json`: use `--json` and optionally `--format compact` (not `--format json`).
- Piping: `asgrep --json … | head` is safe (broken pipe exits cleanly); always put data flags on asgrep, not the pipe consumer.
- Watch + long-lived MCP/Code Mode on the same index: writers bump `writer_generation` beside the index home; warm Searchers poll and reopen. Prefer one shared `ASGREP_INDEX_PATH`. See `docs/index-consistency.md`.
"#
}

pub(crate) fn print_robot_guide() {
    print!("{}", robot_guide_markdown());
}

/// Emit handbook: markdown on stdout, or machine JSON envelope when `--json`.
pub(crate) fn emit_robot_guide(cli: &Cli) -> anyhow::Result<()> {
    if cli.json {
        return crate::print_machine_json(
            "robot-docs",
            serde_json::json!({
                "topic": "guide",
                "format": "markdown",
                "body": robot_guide_markdown(),
            }),
        );
    }
    print_robot_guide();
    Ok(())
}

pub(crate) fn env_flag_truthy(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref().map(str::trim),
        Some("1")
            | Some("true")
            | Some("TRUE")
            | Some("yes")
            | Some("YES")
            | Some("on")
            | Some("ON")
    )
}

/// CI / dumb terminals: keep stderr for errors, drop "indexing ..." progress chatter.
pub(crate) fn suppress_progress() -> bool {
    if env_flag_truthy("CI") {
        return true;
    }
    std::env::var("TERM")
        .ok()
        .is_some_and(|term| term.eq_ignore_ascii_case("dumb"))
}

/// Teach missing-QUERY and other clap usage gaps with an exact recoverable command.
pub(crate) fn augment_clap_usage_message(msg: &str, command: &str) -> String {
    let mut msg = msg.to_string();
    let missing_query = msg.contains("required arguments were not provided")
        && (msg.contains("<QUERY>") || msg.contains("QUERY"));
    if missing_query {
        // Every row comes from COMMAND_EXAMPLES; commands without a row fall
        // back to the bare-default row (no subcommand).
        let row = command_example(command)
            .or_else(|| command_example(BARE_DEFAULT_EXAMPLE))
            .expect("COMMAND_EXAMPLES carries a bare-default row");
        msg.push('\n');
        msg.push_str(&format!("Example: {row}"));
        msg.push_str("\nTip: QUERY is required; optional ROOT defaults to `.`.");
    }
    if command == "codemod" && msg.contains("required arguments were not provided") {
        msg.push('\n');
        // Single-sourced from COMMAND_EXAMPLES so the tip cannot drift.
        let row = command_example("codemod").expect("COMMAND_EXAMPLES carries a codemod row");
        msg.push_str(&format!("Example: {row}"));
        msg.push_str(
            "\nTip: always plan with --dry-run first; apply requires --yes (alias --force).",
        );
    }
    if msg.contains("unexpected argument") && msg.contains("[ROOT]") {
        msg.push('\n');
        msg.push_str(
            "Tip: one ROOT per query -- ROOT is a single directory (positional or --root). Two trees need two searches, or pass their common parent and scope it with in:path.",
        );
    }
    // grep/rg habit flags with no equivalent mode (`-w -F -v -c -m`, plus
    // `--word-regexp`/`--fixed-strings`): clap's "pass it as a value" tip is
    // actively wrong here — replace it with the mode gap and a real command.
    let mut gap_letters: Vec<&str> = Vec::new();
    for (letter, name) in MODE_GAP_NAMES {
        let short = format!("'-{letter}'");
        let long_names = match letter {
            'w' => vec!["'--word-regexp'"],
            'F' => vec!["'--fixed-strings'", "'--fixed-string'"],
            _ => vec![],
        };
        if msg.contains(&short) || long_names.iter().any(|l| msg.contains(l)) {
            gap_letters.push(name);
        }
    }
    if !gap_letters.is_empty() {
        // Strip clap's value tip for these tokens: it teaches the wrong move.
        if let Some(tip_start) = msg.find("tip: to pass ") {
            let tip_end = msg[tip_start..]
                .find('\n')
                .map(|end| tip_start + end)
                .unwrap_or(msg.len());
            msg.replace_range(tip_start..tip_end, "");
        }
        msg.push('\n');
        msg.push_str(&format!(
            "Tip: no {} mode in asgrep -- queries are fused lexical+structural+semantic, not regex. \
             Nearest canonical: asgrep search <query> --limit N | --files-with-matches | --file-filter <glob>",
            gap_letters.join(" / ")
        ));
    }
    msg
}

pub(crate) fn print_agent_help_footer() {
    eprintln!("\nAgent surfaces: {TOOL} capabilities --json | {TOOL} robot-docs guide | {TOOL} doctor --robot-triage");
    eprintln!(
        "Exit codes: 0=ok, 1=usage, 2=operation failed. Use --json for machine-readable stdout."
    );
}
