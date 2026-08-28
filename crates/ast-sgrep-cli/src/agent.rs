use crate::Cli;
use anyhow::Context;
use ast_sgrep_core::semantic_ann::should_use_ann;
use ast_sgrep_core::{IndexStore, StoreError, INDEX_SCHEMA_VERSION};
use clap::{CommandFactory, Parser, Subcommand};
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
pub(crate) fn capabilities_json(_cli: &Cli) -> anyhow::Result<Value> {
    let command = crate::Cli::command();
    let (commands, global_flags, search_tuning_flags) = clap_catalog(&command);
    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "description": command.get_about().map(|s| s.to_string()).unwrap_or_else(|| "Polyglot hybrid code search".into()),
        "agent_contract": {"stdout": "one data payload in machine/default-agent modes", "stderr": "empty in machine modes; human diagnostics otherwise", "deterministic": "stable JSON key ordering via serde_json; disable color with NO_COLOR=1"},
        "commands": commands,
        "global_flags": global_flags,
        "search_tuning_flags": search_tuning_flags,
        "root_specification": {
            "canonical": "positional ROOT on the subcommand (or bare-search ROOT)",
            "alias": "--root ROOT",
            "precedence": "conflicting --root and positional ROOT is a usage error; effective_root prefers --root when set",
            "bin_aliases": ["asgrep", "ast-sgrep"]
        },
        "environment": ["ASGREP_LIMIT", "ASGREP_INDEX_PATH", "ASGREP_DURABILITY", "ASGREP_NO_EMBED", "ASGREP_NO_AUTO_INDEX", "ASGREP_AUTO_INDEX", "ASGREP_NEURAL_EMBED", "ASGREP_NEURAL_FALLBACK", "ASGREP_SEMANTIC_ONLY", "ASGREP_TANTIVY", "ASGREP_ANN_THRESHOLD", "ASGREP_ANN_PROBES", "ASGREP_RERANK", "ASGREP_RERANK_TOP_K", "ASGREP_ALLOW_AST_GREP", "ASGREP_ALLOW_EXTERNAL_INDEX", "ASGREP_AST_GREP", "ASGREP_LEDGER_PATH", "ASGREP_USE_CACHE", "XDG_CACHE_HOME", "NO_COLOR", "CI"],
        "environment_bool_values": ["1", "0", "true", "false", "yes", "no", "on", "off"],
        "sibling_binaries": [
            {"name":"asgrep-mcp","purpose":"MCP stdio server","launch":"asgrep-mcp (stdio JSON-RPC)"},
            {"name":"asgrep-lsp","purpose":"Language Server Protocol server","launch":"asgrep-lsp"}
        ],
        "integrations": {
            "mcp": {"binary": "asgrep-mcp", "transport": "stdio"},
            "lsp": {"binary": "asgrep-lsp", "transport": "stdio"}
        },
        "indexed_source": {
            "policy": "Do not spawn rg on indexed source.",
            "exact_text": "Use literal:<term> for exact substring presence in indexed languages.",
            "freshness": "CLI: search is read-only (no auto-index/refresh) unless --auto-index; run asgrep index / reindex / watch to write; Pi and Code Mode refresh before search with a 30-second default correctness lease; LSP applies document open/change/save/close before the next request.",
            "outside_contract": "Use ripgrep only for logs and unindexed or unsupported files."
        },
        "aliases": ["ast-sgrep"],
        "query_prefixes": ["callers:", "defs:", "imports:", "pattern:", "literal:", "regex:", "word:", "semantic:"],
        "output_limits": {
            "max_results": ast_sgrep_core::MAX_OUTPUT_RESULTS,
            "max_excerpt_lines": ast_sgrep_core::MAX_EXCERPT_LINES,
            "default_snippet_tokens": 96,
            "default_response_snippet_tokens": 768,
            "max_snippet_tokens": 4096,
            "max_response_snippet_tokens": 65536,
            "max_error_message_chars": 4096
        },
        "machine_schema": {
            "schema_version": "1.0.0",
            "ok_field": "boolean",
            "exit_code_field": "integer",
            "notes": "ok:true only on successful operations; doctor uses ok:false when healthy:false; operational faults use exit_code 2"
        },
        "search_formats": ["native", "agent", "agent-capsule", "compact", "github", "gitlab"],
        "exit_codes": [
            {"code": 0, "meaning": "success"},
            {"code": 1, "meaning": "usage error (missing required args, unknown flags, invalid --format, conflicting roots)"},
            {"code": 2, "meaning": "operational failure (index/search/IO) or doctor healthy:false"}
        ],
        "canonical_tasks": ["asgrep capabilities --json", "asgrep robot-docs guide", "asgrep --robot-triage", "asgrep --json --format compact \"where is auth refreshed\" ."],
        "notes": {
            "default_search": "Bare QUERY without a subcommand runs hybrid search; the word 'search' is not a required verb — use the `search`/`find`/`query` subcommand only when you want an explicit search command.",
            "format_implies_json": true,
            "safe_mutating": "index refreshes incrementally with transactional writes. reindex forces a full transactional rewrite -- prefer `asgrep reindex --dry-run <ROOT> --json` before a full reindex. codemod dry-run always emits a JSON edit plan; apply commits one source transaction before a separate incremental index transaction. A source-apply failure rolls back source files; an index-refresh failure leaves the source edits applied and reports `asgrep index` as recovery."
        }
    }))
}

fn clap_catalog(command: &clap::Command) -> (Vec<Value>, Vec<String>, Vec<String>) {
    const SEARCH_TUNING: &[&str] = &[
        "--no-embed",
        "--neural-embed",
        "--semantic-only",
        "--tantivy",
        "--ann-threshold",
        "--ann-probes",
        "--rerank",
        "--rerank-top-k",
        "--format",
        "--excerpt-lines",
        "--snippet-tokens",
        "--response-snippet-tokens",
        "--dry-run",
        // m38g: whole-response token budget that picks per-result detail.
        "--budget-tokens",
    ];
    let mut global_flags = Vec::new();
    let mut search_tuning_flags = Vec::new();
    for arg in command.get_arguments() {
        let Some(long) = arg.get_long() else { continue };
        let flag = format!("--{long}");
        if arg.is_global_set() {
            global_flags.push(flag);
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
        if matches!(name.as_str(), "search" | "keyword" | "semantic") {
            entry["robot_output"] = json!("--format implies --json; formats: native|agent|agent-capsule|compact|github|gitlab");
            entry["example"] = json!(match name.as_str() {
                "keyword" => r#"asgrep keyword --json "auth refresh" ."#,
                "semantic" => r#"asgrep semantic --json "where is auth refreshed" ."#,
                _ => r#"asgrep search --json --format compact "auth refresh" ."#,
            });
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
        commands.push(entry);
    }
    commands.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    (commands, global_flags, search_tuning_flags)
}
/// Doctor issue when FastUnsafe durability is active (R-OPS-DOCS-FOOTGUNS).
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
                    let msg = format!("{e:#}");
                    if let Some((on_disk, supported)) = StoreError::parse_schema_mismatch(&msg) {
                        issues.push(json!({
                            "kind": "schema_mismatch",
                            "on_disk": on_disk,
                            "supported": supported,
                            "message": msg
                        }));
                        if on_disk > supported {
                            next.push("asgrep version --json".to_string());
                        } else {
                            next.push(format!("asgrep reindex {} --json", root.display()));
                        }
                    } else {
                        issues.push(json!({"kind": "index_open", "message": msg}));
                    }
                    None
                }
            },
            Err(e) => {
                let msg = e.to_string();
                if let Some((on_disk, supported)) = StoreError::parse_schema_mismatch(&msg) {
                    issues.push(json!({
                        "kind": "schema_mismatch",
                        "on_disk": on_disk,
                        "supported": supported,
                        "message": msg
                    }));
                    if on_disk > supported {
                        next.push("asgrep version --json".to_string());
                    } else {
                        next.push(format!("asgrep reindex {} --json", root.display()));
                    }
                } else {
                    issues.push(json!({"kind": "index_open", "message": msg}));
                }
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
    Ok(
        json!({"robot_triage": true, "root": root, "index_path": cli.index_path, "status": status, "issues": issues, "suggested_commands": next, "healthy": issues.is_empty(), "tty": io::stdout().is_terminal()}),
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
- `--format` implies `--json`. Prefer `--format compact` for bounded LLM consumption.
- Machine mode emits one JSON value on stdout and no duplicate stderr diagnostics.
## Index cancel / dry-run
- `asgrep index --dry-run` / `asgrep reindex --dry-run` report planned work without mutating the index.
- `asgrep codemod --pattern 'legacy($ARG)' --rewrite 'modern($ARG)' --dry-run .` emits a JSON edit plan without writing; omit `--dry-run` to apply all planned source files transactionally, followed by a separate transactional index refresh. If refresh fails, source edits remain applied and the command reports `asgrep index` as recovery.
- Index writes are transactional; an interrupted uncommitted write is rolled back when SQLite recovers.
## Exit codes
- 0 success · 1 usage · 2 index/search failure
## Environment
See `capabilities --json` → `environment`. Common: `ASGREP_INDEX_PATH`, `ASGREP_LIMIT`, `ASGREP_NO_EMBED`, `ASGREP_NO_AUTO_INDEX`, `ASGREP_DURABILITY`, `NO_COLOR`, `CI`.
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
    "excerpt-lines",
    "snippet-tokens",
    "response-snippet-tokens",
    "file-filter",
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
    let has_doctor = raw.iter().any(|arg| arg == "doctor");
    let mut has_triage = false;
    let mut rewritten = Vec::with_capacity(raw.len() + 1);
    for arg in raw {
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

/// Recover Levenshtein-1 / transposition flag and subcommand typos before clap.
pub(crate) fn rewrite_typos(
    raw: Vec<std::ffi::OsString>,
) -> (Vec<std::ffi::OsString>, Vec<String>) {
    let mut warnings = Vec::new();
    let raw = inject_robot_triage(raw, &mut warnings);
    let mut out = Vec::with_capacity(raw.len());
    let mut passthrough = false;
    let mut saw_positional = false;
    for (index, arg) in raw.into_iter().enumerate() {
        if index == 0 || passthrough {
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
            if let Some(canonical) = closest_long_flag(&folded) {
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
            out.push(arg);
            continue;
        }
        if text.starts_with('-') {
            out.push(arg);
            continue;
        }
        if !saw_positional {
            saw_positional = true;
            if let Some(canonical) = query_looks_like_subcommand_typo(text) {
                if canonical != text {
                    warnings.push(format!(
                        "note: recovered `{text}` as `{canonical}`. Next time: asgrep {canonical} …"
                    ));
                    out.push(std::ffi::OsString::from(canonical));
                    continue;
                }
            }
        }
        out.push(arg);
    }
    (out, warnings)
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
        "call-path",
        "bench",
        "watch",
        "capabilities",
        "version",
        "robot-docs",
        "doctor",
        "eval",
        "codemod",
        "codemode-batch",
        "codemode-serve",
        "find",
        "query",
    ];
    COMMANDS
        .iter()
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

/// Teach missing-QUERY and other clap usage gaps with an exact recoverable command.
pub(crate) fn augment_clap_usage_message(msg: &str, command: &str) -> String {
    let mut msg = msg.to_string();
    let missing_query = msg.contains("required arguments were not provided")
        && (msg.contains("<QUERY>") || msg.contains("QUERY"));
    if missing_query {
        let example = match command {
            "keyword" => r#"Example: asgrep keyword --json "auth refresh" ."#,
            "semantic" => r#"Example: asgrep semantic --json "where is auth refreshed" ."#,
            "search" => r#"Example: asgrep search --json --format compact "auth refresh" ."#,
            "chain" => r#"Example: asgrep chain "callers:process_request" ."#,
            "call-path" => r#"Example: asgrep call-path main validate_input ."#,
            _ => r#"Example: asgrep --json --format compact "auth refresh" ."#,
        };
        msg.push('\n');
        msg.push_str(example);
        msg.push_str("\nTip: QUERY is required; optional ROOT defaults to `.`.");
    }
    msg
}

pub(crate) fn print_agent_help_footer() {
    eprintln!("\nAgent surfaces: {TOOL} capabilities --json | {TOOL} robot-docs guide | {TOOL} doctor --robot-triage");
    eprintln!(
        "Exit codes: 0=ok, 1=usage, 2=operation failed. Use --json for machine-readable stdout."
    );
}
