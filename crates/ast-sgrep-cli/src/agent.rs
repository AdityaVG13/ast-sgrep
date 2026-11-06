use crate::Cli;
use anyhow::Context;
use ast_sgrep_core::semantic_ann::should_use_ann;
use ast_sgrep_core::{IndexStore, StoreError, INDEX_SCHEMA_VERSION};
use clap::{CommandFactory, Parser, Subcommand};
use serde_json::{json, Value};
use std::path::Path;
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
#[derive(Parser, Clone)]
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
pub(crate) fn run_robot_docs(cli: &Cli, args: &RobotDocsArgs) -> anyhow::Result<()> {
    match args.command.as_ref().unwrap_or(&RobotDocsCommand::Guide) {
        RobotDocsCommand::Guide => emit_robot_guide(cli),
    }
}
pub(crate) fn run_doctor(cli: &Cli, root: &Path, args: &DoctorArgs) -> anyhow::Result<()> {
    let _ = (cli.json, args.json, args.robot_triage);
    let triage = doctor_triage_json(cli, root)?;
    let healthy = triage
        .get("healthy")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !healthy {
        // Fail-closed: never emit ok:true / exit 0 when healthy:false (s6ze.1).
        crate::print_machine_json_status("doctor", triage, false, 2)?;
        std::process::exit(2);
    }
    crate::print_machine_json("doctor", triage)
}

/// Static capabilities skeleton (was one 60-line `json!` literal: 26KiB of
/// .text for static data). Dynamic holes — version, description, the
/// clap-derived catalog, and the two `MAX_*` consts — are inserted below so
/// they can never drift from their sources. Regenerate from a trusted binary:
/// `capabilities --json` minus the envelope and the 7 dynamic keys.
const CAPABILITIES_SKELETON: &str = include_str!("capabilities_data.json");

pub(crate) fn capabilities_json(_cli: &Cli) -> anyhow::Result<Value> {
    let command = crate::Cli::command();
    let (commands, global_flags, search_tuning_flags) = clap_catalog(&command);
    let mut caps: Value = serde_json::from_str(CAPABILITIES_SKELETON)
        .map_err(|e| anyhow::anyhow!("checked-in capabilities skeleton must parse: {e}"))?;
    let obj = caps
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("capabilities skeleton must be an object"))?;
    obj.insert(
        "version".to_string(),
        Value::String(env!("CARGO_PKG_VERSION").to_string()),
    );
    obj.insert(
        "description".to_string(),
        Value::String(
            command
                .get_about()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "Polyglot hybrid code search".into()),
        ),
    );
    obj.insert("commands".to_string(), Value::Array(commands));
    obj.insert(
        "global_flags".to_string(),
        Value::Array(global_flags.into_iter().map(Value::String).collect()),
    );
    obj.insert(
        "search_tuning_flags".to_string(),
        Value::Array(search_tuning_flags.into_iter().map(Value::String).collect()),
    );
    let limits = obj
        .get_mut("output_limits")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow::anyhow!("capabilities skeleton must carry output_limits"))?;
    limits.insert(
        "max_results".to_string(),
        json!(ast_sgrep_core::MAX_OUTPUT_RESULTS),
    );
    limits.insert(
        "max_excerpt_lines".to_string(),
        json!(ast_sgrep_core::MAX_EXCERPT_LINES),
    );
    Ok(caps)
}

/// Key for the bare-default row in [`COMMAND_EXAMPLES`] (no subcommand).
const BARE_DEFAULT_EXAMPLE: &str = "bare";

/// Per-command example command lines for the clap catalog and clap usage
/// teaching. Single source — to annotate a command, add a row here, not a
/// new literal at the use site.
const COMMAND_EXAMPLES: &[(&str, &str)] = &[
    (
        "search",
        r#"asgrep search --json --format compact "auth refresh" ."#,
    ),
    ("keyword", r#"asgrep keyword --json "auth refresh" ."#),
    (
        "semantic",
        r#"asgrep semantic --json "where is auth refreshed" ."#,
    ),
    (
        "chain",
        r#"asgrep chain "callers:process_request" ."#,
    ),
    (
        "call-path",
        r#"asgrep call-path main validate_input ."#,
    ),
    (
        "codemod",
        r#"asgrep codemod --dry-run --pattern 'legacy($ARG)' --rewrite 'modern($ARG)' ."#,
    ),
    // Bare default (no subcommand): the fallback for commands without a row.
    (
        BARE_DEFAULT_EXAMPLE,
        r#"asgrep --json --format compact "auth refresh" ."#,
    ),
];

/// Example command line for `name` (`None` when the command has no example row).
pub(crate) fn command_example(name: &str) -> Option<&'static str> {
    COMMAND_EXAMPLES
        .iter()
        .find(|(row, _)| *row == name)
        .map(|(_, example)| *example)
}

fn clap_catalog(command: &clap::Command) -> (Vec<Value>, Vec<String>, Vec<String>) {
    const SEARCH_TUNING: &[&str] = &[
        "--no-embed",
        "--semantic-only",
        "--tantivy",
        "--ann-threshold",
        "--ann-probes",
        "--format",
        "--excerpt-lines",
        "--snippet-tokens",
        "--response-snippet-tokens",
        // m38g: whole-response token budget that picks per-result detail.
        "--budget-tokens",
    ];
    let mut global_flags = Vec::new();
    let mut search_tuning_flags = Vec::new();
    for arg in command.get_arguments() {
        // Hidden args are retired stubs (parse-only, rejected later); never
        // advertise them to agents.
        if arg.is_hide_set() {
            continue;
        }
        let Some(long) = arg.get_long() else { continue };
        let flag = format!("--{long}");
        if arg.is_global_set() {
            global_flags.push(flag);
            if let Some(short) = arg.get_short() {
                global_flags.push(format!("-{short}"));
            }
        } else if SEARCH_TUNING.iter().any(|s| *s == flag) {
            search_tuning_flags.push(flag);
        } else if matches!(
            flag.as_str(),
            "--json" | "--robot-help" | "--root" | "--limit" | "--index-path" | "--lang"
        ) {
            // Non-global copies still documented as agent-visible globals when present on root.
            global_flags.push(flag);
        }
    }
    global_flags.sort();
    global_flags.dedup();
    search_tuning_flags.sort();
    search_tuning_flags.dedup();

    let mut commands = Vec::new();
    for sub in command.get_subcommands() {
        let name = sub.get_name().to_string();
        let aliases: Vec<String> = sub.get_all_aliases().map(str::to_string).collect();
        let about = sub.get_about().map(|s| s.to_string()).unwrap_or_default();
        let mut flags = Vec::new();
        for arg in sub.get_arguments() {
            if arg.is_hide_set() {
                continue;
            }
            if let Some(long) = arg.get_long() {
                flags.push(format!("--{long}"));
            }
        }
        flags.sort();
        flags.dedup();
        let mut entry = json!({
            "name": name,
            "about": about,
            "usage": format!("asgrep {name}"),
            "flags": flags,
        });
        if !aliases.is_empty() {
            entry["aliases"] = json!(aliases);
        }
        if crate::is_search_family(name.as_str()) {
            entry["robot_output"] = json!("--format implies --json; formats: native|agent|agent-capsule|compact|github|gitlab");
            if let Some(example) = command_example(&name) {
                entry["example"] = json!(example);
            }
        }
        if name == "reindex" {
            entry["safe_mutating"] = json!({
                "kind": "full_rebuild",
                "prefer_first": "asgrep reindex --dry-run <ROOT> --json",
                "note": "forces a full in-place transactional rewrite; dry-run reports plan without writing"
            });
        }
        if name == "index" {
            entry["safe_mutating"] = json!({
                "kind": "incremental",
                "prefer_first": "asgrep index <ROOT> --json",
                "note": "incremental refresh with transactional index writes"
            });
        }
        if name == "codemod" {
            entry["safe_mutating"] = json!({
                "kind": "source_rewrite",
                "prefer_first": "asgrep codemod --dry-run --pattern '<pattern>' --rewrite '<rewrite>' <ROOT>",
                "apply": "asgrep codemod --yes --pattern '<pattern>' --rewrite '<rewrite>' <ROOT>",
                "note": "apply requires --yes (alias --force); --dry-run emits a JSON plan without writing source"
            });
            if let Some(example) = command_example("codemod") {
                entry["example"] = json!(example);
            }
        }
        commands.push(entry);
    }
    commands.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    (commands, global_flags, search_tuning_flags)
}
/// Doctor issue when FastUnsafe durability is active.
fn doctor_fast_unsafe_issue(
    cli: &Cli,
    status: Option<&ast_sgrep_core::IndexStatus>,
) -> Option<Value> {
    let from_status = status
        .map(|st| st.durability.as_str())
        .filter(|d| *d == "fast-unsafe");
    let from_cli = cli
        .durability
        .filter(|d| *d == ast_sgrep_core::Durability::FastUnsafe)
        .map(|_| "fast-unsafe");
    from_status.or(from_cli)?;
    Some(json!({
        "kind": "durability_fast_unsafe",
        "message": "ASGREP_DURABILITY=fast-unsafe (or --durability fast-unsafe) is active: power loss during a write batch can corrupt the index. Prefer balanced/strict outside trusted CI speed paths; MCP/Code Mode inherit this env."
    }))
}

fn doctor_index_open_issue(message: String, root: &Path, next: &mut Vec<String>) -> Value {
    if let Some((on_disk, supported)) = StoreError::parse_schema_mismatch(&message) {
        if on_disk > supported {
            next.push("asgrep version --json".to_string());
        } else {
            next.push(format!("asgrep reindex {} --json", root.display()));
        }
        json!({
            "kind": "schema_mismatch",
            "on_disk": on_disk,
            "supported": supported,
            "message": message
        })
    } else {
        json!({"kind": "index_open", "message": message})
    }
}

fn doctor_triage_json(cli: &Cli, root: &Path) -> anyhow::Result<Value> {
    crate::ensure_unambiguous_root(root, cli)?;
    let root = crate::effective_root(cli, root);
    let mut issues = Vec::<Value>::new();
    let mut next = Vec::<String>::new();
    let supported = INDEX_SCHEMA_VERSION;
    let status = if !root.is_dir() {
        issues.push(json!({"kind": "missing_root", "message": format!("project root does not exist or is not a directory: {}", root.display())}));
        None
    } else {
        match IndexStore::peek_schema_version(&root, cli.index_path.as_deref()) {
            Ok(on_disk) if on_disk > supported => {
                issues.push(json!({
                    "kind": "schema_mismatch",
                    "on_disk": on_disk,
                    "supported": supported,
                    "message": format!(
                        "index schema on disk is {on_disk}; this binary supports {supported}. Install a newer asgrep (asgrep version --json → index_schema_version)."
                    )
                }));
                next.push("asgrep version --json".to_string());
                None
            }
            Ok(on_disk) if on_disk < supported => {
                issues.push(json!({
                    "kind": "schema_mismatch",
                    "on_disk": on_disk,
                    "supported": supported,
                    "message": format!(
                        "index schema on disk is {on_disk}; this binary supports {supported}. Rebuild with: asgrep reindex {} --json",
                        root.display()
                    )
                }));
                next.push(format!("asgrep reindex {} --json", root.display()));
                match IndexStore::open_readonly(&root, cli.index_path.as_deref()) {
                    Ok(store) => match store.status() {
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
            }
            Ok(_) => match IndexStore::open_readonly(&root, cli.index_path.as_deref())
                .context("failed to open index for doctor")
            {
                Ok(store) => match store.status() {
                    Ok(status) => Some(status),
                    Err(e) => {
                        issues.push(json!({"kind": "status_read", "message": e.to_string()}));
                        None
                    }
                },
                Err(e) => {
                    issues.push(doctor_index_open_issue(format!("{e:#}"), &root, &mut next));
                    None
                }
            },
            Err(e) => {
                issues.push(doctor_index_open_issue(e.to_string(), &root, &mut next));
                None
            }
        }
    };
    if let Some(issue) = doctor_fast_unsafe_issue(cli, status.as_ref()) {
        issues.push(issue);
        next.push("unset ASGREP_DURABILITY  # or: asgrep --durability balanced …".to_string());
    }
    let root_display = root.display().to_string();
    if status.is_none()
        && !next
            .iter()
            .any(|c| c.contains("reindex") || c.contains("version --json"))
    {
        next.push(format!("asgrep index {root_display} --json"));
    } else if let Some(ref st) = status {
        if st.file_count == 0 {
            issues.push(
                json!({"kind": "empty_index", "message": "index exists but indexes zero files"}),
            );
            next.push(format!("asgrep index {root_display} --json"));
        }
        if !st.semantic_ivf_present && should_use_ann(st.semantic_chunk_count, None) {
            issues.push(json!({"kind": "semantic_ivf_missing", "message": "semantic chunks present but IVF sidecar not built"}));
        }
    }
    if next.is_empty() {
        next.push(format!(
            "asgrep --json --format compact \"<your query>\" {root_display}"
        ));
    }
    next.extend([
        "asgrep capabilities --json".to_string(),
        "asgrep robot-docs guide".to_string(),
    ]);
    // H-PERF-001 option (b): supervision is opt-in; doctor reports the active
    // policy so agents can discover (or restore) the CPU-limit contract.
    let supervision = if crate::supervisor::supervision_requested() {
        json!({"mode": "duty_cycle", "cpu_limit_percent": crate::supervisor::cpu_limit_percent()})
    } else {
        json!({
            "mode": "off",
            "opt_in": "set ASGREP_CPU_LIMIT_PERCENT=1..80 to enable duty-cycle supervision (SIGSTOP/CONT in a 10 ms window)",
        })
    };
    Ok(
        json!({"robot_triage": true, "root": root, "index_path": cli.index_path, "status": status, "issues": issues, "suggested_commands": next, "supervision": supervision, "healthy": issues.is_empty()}),
    )
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
const KNOWN_LONG_FLAGS: &[&str] = &[
    "json",
    "robot-help",
    "root",
    "limit",
    "index-path",
    "lang",
    "durability",
    "no-auto-index",
    "auto-index",
    "no-embed",
    "neural-embed",
    "semantic-only",
    "tantivy",
    "ann-threshold",
    "ann-probes",
    "rerank",
    "rerank-top-k",
    "format",
    "preview",
    "excerpt-lines",
    "snippet-tokens",
    "response-snippet-tokens",
    "file-filter",
    "files-with-matches",
    "count",
    "path",
    "budget-tokens",
    "help",
    "version",
    "dry-run",
    "pattern",
    "rewrite",
    "max-depth",
    "max-nodes",
    "max-edges",
    "max-work",
    "query",
    "iterations",
    "suite",
    "fixture",
    "queries-file",
    "skip-index",
    "debounce-ms",
    "scip",
    "robot-triage",
    "robot-next",
    "requests",
    "yes",
    "force",
    "target",
];

const COLOR_NOOPS: &[&str] = &["color", "colour", "no-color", "no-colour"];

fn closest_long_flag(name: &str) -> Option<&'static str> {
    if let Some(flag) = KNOWN_LONG_FLAGS.iter().copied().find(|flag| *flag == name) {
        return Some(flag);
    }
    KNOWN_LONG_FLAGS
        .iter()
        .copied()
        .map(|flag| (flag, edit_distance(name, flag)))
        .min_by_key(|(_, distance)| *distance)
        .filter(|(flag, distance)| {
            is_adjacent_transposition(name, flag)
                || *distance <= 1
                || (*distance == 2
                    && name.len().abs_diff(flag.len()) <= 1
                    && name.len().min(flag.len()) >= 4)
        })
        .map(|(flag, _)| flag)
}

fn inject_robot_triage(
    raw: Vec<std::ffi::OsString>,
    warnings: &mut Vec<String>,
) -> Vec<std::ffi::OsString> {
    let has_doctor = raw
        .iter()
        .take_while(|arg| arg.as_os_str() != "--")
        .any(|arg| arg == "doctor");
    let mut has_triage = false;
    let mut passthrough = false;
    let mut rewritten = Vec::with_capacity(raw.len() + 1);
    for arg in raw {
        if arg == "--" {
            passthrough = true;
        }
        if passthrough {
            rewritten.push(arg);
            continue;
        }
        let text = arg.to_str().unwrap_or("");
        if matches!(text, "--robot-next" | "--robot-diagnose") {
            warnings.push(
                "note: recovered mega-command as `--robot-triage`. Next time: asgrep doctor --robot-triage"
                    .into(),
            );
            rewritten.push(std::ffi::OsString::from("--robot-triage"));
            has_triage = true;
            continue;
        }
        if text == "--robot-triage" {
            has_triage = true;
        }
        rewritten.push(arg);
    }
    if has_triage && !has_doctor && !rewritten.is_empty() {
        rewritten.insert(1, std::ffi::OsString::from("doctor"));
    }
    rewritten
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

/// Commands a typo may be silently recovered INTO: read-only info commands,
/// plus any command when the argv carries explicit help intent (the recovered
/// run cannot mutate anything). Mutating commands (index, reindex, codemod…)
/// are excluded so a near-miss token never performs writes.
const RECOVERABLE_INFO_COMMANDS: &[&str] = &[
    "status",
    "capabilities",
    "version",
    "robot-docs",
    "doctor",
    "read",
];

fn positional_recovery_is_safe(canonical: &str, help_intent: bool) -> bool {
    RECOVERABLE_INFO_COMMANDS.contains(&canonical) || help_intent
}

/// grep-style reachability habit: `chain SYMBOL --to TARGET` is the call-path
/// query (SOURCE SINK) in different clothing. Recover the canonical shape
/// only when it is unambiguous (global flags may precede the verb; at most
/// one positional; `--to` target last or attached); anything else keeps the
/// chain verb intact so clap's own errors still name the real flags.
fn recover_chain_to_call_path(
    parser: &clap::Command,
    raw: &[std::ffi::OsString],
) -> Option<Vec<std::ffi::OsString>> {
    // Locate the `chain` verb: skip argv[0], global flags, and flag values
    // (the parser knows which long flags take values).
    let mut chain_at = None;
    let mut expect_value = false;
    for (position, arg) in raw.iter().enumerate().skip(1) {
        let text = arg.to_str()?;
        if expect_value {
            expect_value = false;
            continue;
        }
        if let Some(rest) = text.strip_prefix("--") {
            if !text.contains('=') {
                let name = rest.split('=').next().unwrap_or(rest);
                expect_value = parser
                    .get_arguments()
                    .any(|spec| spec.get_long() == Some(name) && spec.get_action().takes_values());
            }
            continue;
        }
        if text.starts_with('-') && text != "-" {
            continue;
        }
        if text == "chain" {
            chain_at = Some(position);
        }
        break;
    }
    let start = chain_at? + 1;
    let mut flags = Vec::new();
    let mut query: Option<std::ffi::OsString> = None;
    let total = raw.len();
    let mut index = start;
    while index < total {
        let text = raw[index].to_str()?;
        if text == "--to" {
            if index + 1 >= total {
                return None;
            }
            let query = query?;
            let mut out = raw[..start - 1].to_vec();
            out.push(std::ffi::OsString::from("call-path"));
            out.extend(flags);
            out.push(query);
            out.push(raw[index + 1].clone());
            out.extend(raw[index + 2..].iter().cloned());
            return Some(out);
        }
        if let Some(target) = text.strip_prefix("--to=") {
            let query = query?;
            let mut out = raw[..start - 1].to_vec();
            out.push(std::ffi::OsString::from("call-path"));
            out.extend(flags);
            out.push(query);
            out.push(std::ffi::OsString::from(target));
            out.extend(raw[index + 1..].iter().cloned());
            return Some(out);
        }
        if text.starts_with('-') && text != "-" {
            flags.push(raw[index].clone());
        } else if query.is_none() {
            query = Some(raw[index].clone());
        } else {
            // A second positional is a ROOT — the intent is not the simple
            // reachability habit; leave the argv alone.
            return None;
        }
        index += 1;
    }
    None
}

/// Recover Levenshtein-1 / transposition flag and subcommand typos before clap.
pub(crate) fn rewrite_typos(
    raw: Vec<std::ffi::OsString>,
) -> (Vec<std::ffi::OsString>, Vec<String>) {
    let mut warnings = Vec::new();
    let raw = inject_robot_triage(raw, &mut warnings);
    let help_intent = raw
        .iter()
        .skip(1)
        .take_while(|arg| arg.as_os_str() != "--")
        .any(|arg| matches!(arg.to_str(), Some("-h") | Some("--help")));
    let parser = Cli::command();
    let raw = match recover_chain_to_call_path(&parser, &raw) {
        Some(rewritten) => {
            warnings.push(
                "note: recovered `chain <sym> --to <target>` onto `call-path <source> <sink>`. \
                 Next time: asgrep call-path <source> <sink>"
                    .to_string(),
            );
            rewritten
        }
        None => raw,
    };
    let mut out = Vec::with_capacity(raw.len());
    let mut passthrough = false;
    let mut saw_positional = false;
    // ast-grep batch habit (`sg run -p PATTERN ROOT`): when argv was routed
    // onto search BY RECOVERY (bridge, -p, or a run/scan vocab reroute) and
    // the query rides --pattern flags, the first bare positional is the ROOT.
    // Explicit `codemod --pattern P .` / `search --pattern P QUERY` keep their
    // own positional grammar (the search help pins QUERY as the slot).
    let mut search_via_recovery = false;
    let mut pattern_seen = false;
    let mut pattern_root_seen = false;
    // grep `-w WORD` habit: the word-boundary mode maps onto the `word:`
    // query channel; the prefix applies to the first surviving QUERY
    // positional, never after a non-search subcommand was named.
    let mut word_prefix = false;
    let mut word_prefixed = false;
    let mut nonsearch_subcommand = false;
    let mut value_pending = false;
    let mut inserted_search_subcommand = false;
    let mut index = 0;
    while index < raw.len() {
        let arg = raw[index].clone();
        let next_text = raw.get(index + 1).and_then(|next| next.to_str());
        index += 1;
        if index == 1 || passthrough {
            out.push(arg);
            continue;
        }
        if value_pending {
            value_pending = false;
            out.push(arg);
            continue;
        }
        if arg == "--" {
            passthrough = true;
            out.push(arg);
            continue;
        }
        let Some(text) = arg.to_str() else {
            out.push(arg);
            continue;
        };
        if let Some(rest) = text.strip_prefix("--") {
            let (name, value) = match rest.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (rest, None),
            };
            let folded = name.to_ascii_lowercase();
            if COLOR_NOOPS.contains(&folded.as_str()) {
                warnings.push(format!(
                    "note: ignored `--{name}`; asgrep is monochrome. Use NO_COLOR=1. Machine data: asgrep --json …"
                ));
                continue;
            }
            const JSON_ALIASES: &[&str] = &["machine", "output-json", "output_json"];
            if JSON_ALIASES.contains(&folded.as_str())
                || (folded == "format" && value.is_some_and(|v| v.eq_ignore_ascii_case("json")))
            {
                warnings.push(format!(
                    "note: recovered `--{rest}` as `--json`. Next time: --json"
                ));
                out.push(std::ffi::OsString::from("--json"));
                continue;
            }
            // `--format json` (detached value): recover before the enum rejects it.
            if folded == "format" && value.is_none() && next_text == Some("json") {
                warnings.push(
                    "note: recovered `--format json` as `--json`. Next time: --json".to_string(),
                );
                out.push(std::ffi::OsString::from("--json"));
                index += 1; // consume the detached `json` value
                continue;
            }
            // `--json=<anything>`: --json takes no value; the envelope is
            // already pretty-printed, so any value spelling is stripped.
            if folded == "json" && value.is_some() {
                warnings.push(format!(
                    "note: recovered `--{rest}` as `--json` (--json takes no value; \
                     the envelope is already pretty-printed). Next time: --json"
                ));
                out.push(std::ffi::OsString::from("--json"));
                continue;
            }
            // grep/rg habit vocabulary with an exact asgrep equivalent.
            if let Some((canonical, takes_value)) = habit_long_flag(&folded) {
                let rewritten = match value {
                    Some(value) => format!("--{canonical}={value}"),
                    None => format!("--{canonical}"),
                };
                warnings.push(format!(
                    "note: recovered `--{name}` as `--{canonical}`. Next time: {rewritten}"
                ));
                value_pending = value.is_none() && takes_value;
                out.push(std::ffi::OsString::from(rewritten));
                continue;
            }
            // grep/rg habit vocabulary that is already the default: accept and teach.
            if let Some(note) = habit_noop_long_flag(&folded) {
                warnings.push(format!("note: ignored `--{name}`; {note}"));
                continue;
            }
            // ast-grep habit: a root-level `--pattern` belongs to the search
            // subcommand. Recover by routing argv onto `search` (exact
            // semantics, so this is an alias-grade recovery, not a rejection).
            if folded == "pattern" && !saw_positional && !inserted_search_subcommand {
                warnings.push(
                    "note: recovered root-level `--pattern` onto the search subcommand. \
                     Next time: asgrep search --pattern <pattern>"
                        .to_string(),
                );
                out.insert(1, std::ffi::OsString::from("search"));
                inserted_search_subcommand = true;
                search_via_recovery = true;
            }
            let canonical = closest_long_flag(&folded);
            let option_name = canonical.map_or(folded.as_str(), |name| name);
            value_pending = value.is_none()
                && parser.get_arguments().any(|spec| {
                    spec.get_long() == Some(option_name) && spec.get_action().takes_values()
                });
            if let Some(canonical) = canonical {
                if canonical != folded {
                    let rewritten = match value {
                        Some(value) => format!("--{canonical}={value}"),
                        None => format!("--{canonical}"),
                    };
                    warnings.push(format!(
                        "note: recovered `--{name}` as `--{canonical}`. Next time: {rewritten}"
                    ));
                    out.push(std::ffi::OsString::from(rewritten));
                    continue;
                }
            }
            if folded == "pattern" {
                pattern_seen = true;
                // --pattern is a search-level flag, so the root-level parser
                // spec cannot tell us it takes a value; mark it explicitly or
                // the pattern value would be mistaken for a positional.
                if value.is_none() {
                    value_pending = true;
                }
            }
            out.push(arg);
            continue;
        }
        if text.starts_with('-') && text != "-" {
            // ast-grep habit: bare `-p PATTERN` is the same surface as the
            // repeatable `--pattern`. Recover the exact short form only;
            // clusters containing `p` keep failing intact so clap's error
            // names the whole token.
            // grep/rg context habits: `-A N` / `-B N` (and attached `-A3`)
            // map onto the single window knob `--excerpt-lines N`. The
            // window opens AROUND each hit — after/before are not separate
            // knobs, and the note says so. Attached forms must be fully
            // numeric (`-A3`); anything else fails intact so clap names the
            // whole token.
            // grep `-m N` (max-count) habit: asgrep has no per-file cap —
            // queries are fused, not per-file regex — but the global hit cap
            // `--limit N` is the nearest exact knob. Same approximation
            // class as `-A`/`-B`: recover with the note naming the
            // difference.
            if text == "-m" {
                warnings.push(
                    "note: recovered `-m` as `--limit` (global hit cap; asgrep has no \
                     per-file match cap — queries are fused). Next time: --limit N"
                        .to_string(),
                );
                value_pending = true;
                out.push(std::ffi::OsString::from("--limit"));
                continue;
            }
            if let Some(count) = text
                .strip_prefix("-m")
                .filter(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
            {
                warnings.push(format!(
                    "note: recovered `{text}` as `--limit {count}` (global hit cap; asgrep has \
                     no per-file match cap — queries are fused). Next time: --limit {count}"
                ));
                out.push(std::ffi::OsString::from(format!("--limit={count}")));
                continue;
            }
            if text == "-A" || text == "-B" {
                warnings.push(format!(
                    "note: recovered `{text}` as `--excerpt-lines` (one window around each \
                     hit; after/before are not separate knobs). Next time: --excerpt-lines N"
                ));
                value_pending = true;
                out.push(std::ffi::OsString::from("--excerpt-lines"));
                continue;
            }
            if let Some(count) = text
                .strip_prefix("-A")
                .or_else(|| text.strip_prefix("-B"))
                .filter(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
            {
                warnings.push(format!(
                    "note: recovered `{text}` as `--excerpt-lines {count}`. Next time: \
                     --excerpt-lines {count}"
                ));
                out.push(std::ffi::OsString::from(format!("--excerpt-lines={count}")));
                continue;
            }
            if text == "-p" {
                warnings.push(
                    "note: recovered `-p` as `--pattern`. Next time: asgrep search --pattern <pattern>"
                        .to_string(),
                );
                if !saw_positional && !inserted_search_subcommand {
                    out.insert(1, std::ffi::OsString::from("search"));
                    inserted_search_subcommand = true;
                    search_via_recovery = true;
                }
                pattern_seen = true;
                value_pending = true;
                out.push(std::ffi::OsString::from("--pattern"));
                continue;
            }
            // grep/rg habit short flags: recover clusters of known letters.
            if let Some(cluster) = text.strip_prefix('-') {
                if !cluster.is_empty()
                    && !cluster.starts_with('-')
                    && cluster.chars().all(|c| HABIT_SHORT_LETTERS.contains(&c))
                {
                    let letters: Vec<char> = cluster.chars().collect();
                    // A cluster containing a mode-gap letter (-w -F -v -c -m)
                    // must fail intact so clap's error names the whole token;
                    // augment teaches the gap instead of the value tip.
                    if !letters.iter().any(|c| MODE_GAP_SHORTS.contains(c)) {
                        let last = letters.len() - 1;
                        for (position, letter) in letters.iter().enumerate() {
                            match letter {
                                'l' => {
                                    warnings.push(
                                        "note: recovered `-l` as `--files-with-matches`. Next time: --files-with-matches"
                                            .to_string(),
                                    );
                                    out.push(std::ffi::OsString::from("--files-with-matches"));
                                }
                                'w' => {
                                    warnings.push(
                                        "note: recovered `-w` as the `word:` query channel. \
                                         Next time: asgrep word:QUERY"
                                            .to_string(),
                                    );
                                    word_prefix = true;
                                }
                                'F' => {
                                    warnings.push(
                                        "note: ignored `-F`; queries are grammar tokens, \
                                         never regex, so fixed-string is the default"
                                            .to_string(),
                                    );
                                }
                                'c' => {
                                    warnings.push(
                                        "note: recovered `-c` as `--count`. Next time: --count"
                                            .to_string(),
                                    );
                                    out.push(std::ffi::OsString::from("--count"));
                                }
                                'C' if position == last => {
                                    warnings.push(
                                        "note: recovered `-C` as `--excerpt-lines`. Next time: --excerpt-lines N"
                                            .to_string(),
                                    );
                                    value_pending = true;
                                    out.push(std::ffi::OsString::from("--excerpt-lines"));
                                }
                                'g' if position == last => {
                                    warnings.push(
                                        "note: recovered `-g` as `--file-filter`. Next time: --file-filter <glob>"
                                            .to_string(),
                                    );
                                    value_pending = true;
                                    out.push(std::ffi::OsString::from("--file-filter"));
                                }
                                't' if position == last => {
                                    warnings.push(
                                        "note: recovered `-t` as `--lang`. Next time: --lang <lang>"
                                            .to_string(),
                                    );
                                    value_pending = true;
                                    out.push(std::ffi::OsString::from("--lang"));
                                }
                                'e' if position == last => {
                                    warnings.push(
                                        "note: recovered `-e` as the positional QUERY. Next time: asgrep search <query>"
                                            .to_string(),
                                    );
                                    value_pending = true; // the pattern passes through as QUERY
                                }
                                'i' => warnings.push(
                                    "note: ignored `-i`; search is case-insensitive by default"
                                        .to_string(),
                                ),
                                'n' => warnings.push(
                                    "note: ignored `-n`; every hit already carries file:line"
                                        .to_string(),
                                ),
                                _ => unreachable!("cluster filtered to known letters"),
                            }
                        }
                        continue;
                    }
                }
            }
            out.push(arg);
            continue;
        }
        if !saw_positional {
            saw_positional = true;
            if let Some(canonical) = query_looks_like_subcommand_typo(text) {
                // An exact non-search subcommand (`-w read REF`): the word
                // channel must never prefix a ref or non-query positional.
                if canonical == text
                    && !crate::cli_args::Commands::is_search_channel_name(text)
                {
                    nonsearch_subcommand = true;
                }
                if canonical != text
                    && (positional_recovery_is_safe(canonical, help_intent)
                        || vocab_recovery_is_read_only(canonical))
                {
                    warnings.push(format!(
                        "note: recovered `{text}` as `{canonical}`. Next time: asgrep {canonical} …"
                    ));
                    if canonical == "search" {
                        search_via_recovery = true;
                    }
                    out.push(std::ffi::OsString::from(canonical));
                    continue;
                }
                if canonical != text && !vocab_recovery_is_read_only(canonical) {
                    // Cross-surface vocabulary naming a mutating command is
                    // NEVER guessed into execution: teach the canonical form
                    // and let the (read-only) bare-search path answer instead
                    // of silently running e.g. `index` (which writes the
                    // workspace).
                    warnings.push(format!(
                        "note: `{text}` names the MCP tool `{canonical}`; the CLI canonical form is \
                         `asgrep {canonical} …` and mutating commands are never guessed into \
                         execution. If intended: asgrep {canonical} --help"
                    ));
                }
            }
        }
        if word_prefix
            && !word_prefixed
            && !nonsearch_subcommand
            && !pattern_seen
            && query_looks_like_subcommand_typo(text).is_none()
        {
            // grep `-w WORD` habit: the boundary mode is the `word:` channel.
            word_prefixed = true;
            out.push(std::ffi::OsString::from(format!("word:{text}")));
            continue;
        }
        if search_via_recovery && pattern_seen && !pattern_root_seen {
            // `--pattern P ROOT` (ast-grep batch habit): with the query
            // carried by flags the first bare positional is the ROOT.
            pattern_root_seen = true;
            out.push(std::ffi::OsString::from("--root"));
            out.push(arg);
            continue;
        }
        out.push(arg);
    }
    (out, warnings)
}

/// grep/rg long-flag vocabulary with an exact asgrep equivalent:
/// `(canonical long flag, the canonical flag takes a value)`.
fn habit_long_flag(folded: &str) -> Option<(&'static str, bool)> {
    match folded {
        "include" | "glob" => Some(("file-filter", true)),
        "max-results" | "max-count" => Some(("limit", true)),
        "type" => Some(("lang", true)),
        "word-regexp" => None,   // mode gap: taught by augment, never aliased
        "fixed-strings" => None, // mode gap
        _ => None,
    }
}

/// grep/rg long flags whose semantics are already the asgrep default.
fn habit_noop_long_flag(folded: &str) -> Option<&'static str> {
    match folded {
        "ignore-case" | "smart-case" => Some("search is case-insensitive by default"),
        "line-number" | "line-numbers" | "line-num" => Some("every hit already carries file:line"),
        _ => None,
    }
}

/// Short-flag letters recognised as grep/rg habit vocabulary (2026-10-22
/// intent corpus). Letters without a semantic equivalent are taught by
/// `augment_clap_usage_message` instead of being aliased.
const HABIT_SHORT_LETTERS: &[char] = &['l', 'C', 'g', 't', 'e', 'i', 'n', 'w', 'F', 'v', 'c', 'm'];

/// Habit short flags with no equivalent mode in fused hybrid search.
const MODE_GAP_SHORTS: &[char] = &['v', 'm'];

/// Human names for the mode gaps, used in clap-error teaching.
const MODE_GAP_NAMES: &[(char, &str)] = &[('v', "invert"), ('m', "per-file match cap")];

/// Exact cross-surface vocabulary targets that are safe to recover into
/// without the info-command gate: unambiguous read-only search/status verbs.
fn vocab_recovery_is_read_only(canonical: &str) -> bool {
    matches!(
        canonical,
        "search" | "keyword" | "semantic" | "status" | "read"
    )
}

pub(crate) fn query_looks_like_subcommand_typo(query: &str) -> Option<&'static str> {
    let q = query.trim();
    if q.is_empty() || q.contains(' ') {
        return None;
    }
    let lower = q.to_ascii_lowercase();
    // Plausible search tokens that sit near a command name at edit-distance 2.
    const SEARCH_SAFE: &[&str] = &[
        "static",
        "string",
        "struct",
        "switch",
        "symbol",
        "sample",
        "searchable",
    ];
    if SEARCH_SAFE.contains(&lower.as_str()) {
        return None;
    }
    const ALIASES: &[(&str, &str)] = &[
        ("capability", "capabilities"),
        ("robot_docs", "robot-docs"),
        ("robotdocs", "robot-docs"),
        ("robot-doc", "robot-docs"),
        ("docs", "robot-docs"),
        ("handbook", "robot-docs"),
    ];
    if let Some((_, canonical)) = ALIASES.iter().find(|(alias, _)| *alias == lower) {
        return Some(*canonical);
    }
    // Cross-surface vocabulary (2026-10-22 intent corpus): exact MCP/codemode
    // tool names and abbreviations an agent transfers to the CLI. Exact
    // matches only — never fuzzy, so a search query word that happens to be
    // close never reroutes. Mutating targets still pass the read-only gate
    // in `positional_recovery_is_safe`.
    const EXACT_VOCAB: &[(&str, &str)] = &[
        ("sem", "semantic"),
        ("keyword_search", "keyword"),
        ("semantic_search", "semantic"),
        ("code_search", "keyword"),
        ("code_read", "read"),
        ("index_status", "status"),
        ("grep", "search"),
        ("run", "search"),
        ("scan", "search"),
        ("ast", "search"),
        ("structural", "search"),
        ("syntax", "search"),
        ("tree", "search"),
        ("index_repo", "index"),
    ];
    if let Some((_, canonical)) = EXACT_VOCAB.iter().find(|(alias, _)| *alias == lower) {
        // Targets must name canonical commands (checked against `Commands::name`
        // via `all_names` so the table cannot drift from the clap inventory).
        debug_assert!(
            crate::cli_args::Commands::all_names().contains(canonical),
            "EXACT_VOCAB target must be a canonical command"
        );
        return Some(*canonical);
    }
    // Canonical names come from `Commands::all_names` (single source next to
    // `Commands::name`); the search aliases come from
    // `Commands::search_aliases` so the pair cannot drift.
    crate::cli_args::Commands::all_names()
        .iter()
        .chain(crate::cli_args::Commands::search_aliases().iter())
        .map(|command| (*command, edit_distance(&lower, command)))
        .min_by_key(|(_, distance)| *distance)
        .filter(|(command, distance)| {
            is_adjacent_transposition(&lower, command)
                || *distance <= 1
                || (*distance == 2
                    && lower.len().abs_diff(command.len()) <= 1
                    && lower.len().min(command.len()) >= 5)
        })
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

/// Byte Levenshtein distance shared by flag, command, and format suggestions.
pub(crate) fn edit_distance(left: &str, right: &str) -> usize {
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
