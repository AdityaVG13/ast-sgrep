#![forbid(unsafe_code)]

mod agent;
pub mod bench;
mod cli_args;
mod codemod_cmd;
mod eval;
mod index_cmd;
mod install;
pub mod keep_gate;
mod machine;
mod outline_cmd;
mod search_cmd;
pub mod supervisor;
mod watch;

use anyhow::Context;
use clap::{CommandFactory, Parser};
use cli_args::{Cli, Commands, VersionArgs};
use index_cmd::{print_status_command, run_full_index, run_index_dry_run, run_targeted_index};
use machine::{
    print_machine_failure, print_machine_json, raw_command_name, raw_machine_output_requested,
    MACHINE_SCHEMA_VERSION,
};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub(crate) use cli_args::{usage_error, UsageError};
pub(crate) use index_cmd::{
    effective_root, ensure_existing_root, ensure_unambiguous_root, index_options,
    open_indexed_store, open_indexer, open_searcher, resolve_root_index, search_options,
};
pub(crate) use machine::print_machine_json_status;

pub fn main() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    if supervisor::guardian_requested() {
        supervisor::run_guardian();
    }
    #[cfg(not(unix))]
    {
        run_process()
    }
    #[cfg(unix)]
    {
        if supervisor::is_worker() {
            if supervisor::worker_authenticate() {
                supervisor::worker_start()?;
                run_process()
            } else {
                supervisor::supervise()
            }
        } else if supervisor::supervision_requested() {
            supervisor::supervise()
        } else {
            supervisor::run_unsupervised()
        }
    }
}

fn rewrite_search_path_alias(mut raw: Vec<OsString>) -> Vec<OsString> {
    if !raw
        .iter()
        .skip(1)
        .any(|arg| arg == "--path" || arg.to_str().is_some_and(|s| s.starts_with("--path=")))
    {
        return raw;
    }
    // Only the first positional can select a command. Option values and later
    // positionals (queries/roots) must not change what --path means.
    let parser = Cli::command();
    let mut active = &parser;
    let mut command_seen = false;
    let mut value_pending = false;
    let mut aliases = Vec::new();
    for (index, arg) in raw.iter().enumerate().skip(1) {
        if value_pending {
            value_pending = false;
            continue;
        }
        if arg == "--" {
            break;
        }
        let Some(s) = arg.to_str() else {
            command_seen = true;
            continue;
        };
        if let Some(option) = s.strip_prefix("--") {
            let (name, attached) = option
                .split_once('=')
                .map_or((option, false), |(name, _)| (name, true));
            if name == "path" {
                aliases.push(index);
            }
            value_pending = !attached
                && (name == "path"
                    || active
                        .get_arguments()
                        .chain(parser.get_arguments())
                        .any(|spec| {
                            spec.get_long() == Some(name) && spec.get_action().takes_values()
                        }));
        } else if (!s.starts_with('-') || s == "-") && !command_seen {
            command_seen = true;
            if let Some(command) = parser.find_subcommand(arg) {
                if !matches!(command.get_name(), "search" | "keyword" | "semantic") {
                    return raw;
                }
                active = command;
            }
        }
    }
    for index in aliases {
        if let Some(value) = raw[index].to_str().and_then(|s| s.strip_prefix("--path=")) {
            raw[index] = OsString::from(format!("--file-filter={value}"));
        } else {
            raw[index] = OsString::from("--file-filter");
        }
    }
    raw
}

fn run_process() -> ! {
    run_process_with(run_cli)
}

/// Shared parser and terminal error boundary for the CLI and exec helpers.
fn run_process_with(run: impl FnOnce(&Cli) -> anyhow::Result<()>) -> ! {
    let raw_args = rewrite_search_path_alias(std::env::args_os().collect());
    let (raw_args, typo_warnings) = agent::rewrite_typos(raw_args);
    for warning in &typo_warnings {
        eprintln!("{warning}");
    }
    let cli = match Cli::try_parse_from(&raw_args) {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = if error.use_stderr() { 1 } else { 0 };
            let mut msg = error.to_string();
            // Intent recovery: common agent mistakes clap does not map well.
            if msg.contains("'--colour'") || msg.contains("\"--colour\"") {
                msg.push_str(
                    "\nTip: asgrep has no --colour; use NO_COLOR=1 or default monochrome. Machine data: asgrep --json …",
                );
            }
            if msg.contains("'--color'") || msg.contains("\"--color\"") {
                msg.push_str(
                    "\nTip: asgrep has no --color flag; set NO_COLOR=1 to force plain text. Machine data: asgrep --json …",
                );
            }
            let command = raw_command_name(&raw_args);
            msg = agent::augment_clap_usage_message(&msg, &command);
            let augmented = msg != error.to_string();
            if exit_code == 1 && raw_machine_output_requested(&raw_args) {
                print_machine_failure(&command, "usage", exit_code, &msg);
            } else if exit_code == 1 {
                // Always teach: triad footer on every usage error (not only when we rewrote the body).
                if augmented {
                    eprint!("{msg}");
                    if !msg.ends_with('\n') {
                        eprintln!();
                    }
                } else {
                    let _ = error.print();
                }
                agent::print_agent_help_footer();
            } else {
                let _ = error.print();
            }
            std::process::exit(exit_code);
        }
    };
    match run(&cli) {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            let usage = error.downcast_ref::<UsageError>().is_some();
            let exit_code = if usage { 1 } else { 2 };
            if cli.machine_output_requested() {
                print_machine_failure(
                    cli.command_name(),
                    if usage { "usage" } else { "operational" },
                    exit_code,
                    &format!("{error:#}"),
                );
            } else {
                eprintln!("{error:#}");
                agent::print_agent_help_footer();
            }
            std::process::exit(exit_code);
        }
    }
}

pub fn run() -> anyhow::Result<()> {
    run_cli(&Cli::parse())
}

fn run_cli(cli: &Cli) -> anyhow::Result<()> {
    if cli.robot_help {
        return agent::emit_robot_guide(cli);
    }
    validate_cli(cli)?;
    match cli.command.as_ref() {
        Some(c) => run_command(cli, c),
        None => run_default_search(cli),
    }
}

fn validate_cli(cli: &Cli) -> anyhow::Result<()> {
    // Discovery commands must work in any environment: a stale retired env
    // var in the shell cannot be allowed to break `capabilities`.
    if matches!(
        cli.command.as_ref(),
        Some(Commands::Capabilities(_) | Commands::RobotDocs(_) | Commands::Version(_))
    ) {
        return Ok(());
    }
    // Retired flags fail loudly instead of silently no-op'ing: the static
    // engine is the only embedding backend and reranking left with the
    // neural stack.
    let tuning = cli.active_tuning();
    if tuning.neural_embed {
        anyhow::bail!(
            "--neural-embed / ASGREP_NEURAL_EMBED was removed: asgrep has one embedding engine (compiled-in static); unset it and re-run `asgrep index` once to migrate old indexes"
        );
    }
    if tuning.rerank || tuning.rerank_top_k != 20 {
        anyhow::bail!(
            "--rerank / ASGREP_RERANK was removed with the neural backends; reranking is not available in this build"
        );
    }
    if let Ok(backend) = std::env::var("ASGREP_EMBED_BACKEND") {
        let backend = backend.trim();
        if !backend.is_empty() && !backend.eq_ignore_ascii_case("static") {
            anyhow::bail!(
                "ASGREP_EMBED_BACKEND={backend} was removed: asgrep has one embedding engine (compiled-in static); unset it and re-run `asgrep index` once to migrate old indexes"
            );
        }
    }
    // Search-only flags must fail on commands that cannot apply them.
    let search_command = matches!(
        cli.command.as_ref(),
        None | Some(Commands::Search(_) | Commands::Keyword(_) | Commands::Semantic(_))
    );
    if tuning.semantic_only
        && matches!(cli.command.as_ref(), Some(Commands::Search(q)) if !q.pattern.is_empty())
    {
        return Err(usage_error(
            "--pattern selects structural search and cannot be combined with --semantic-only; \
             remove the flag or unset ASGREP_SEMANTIC_ONLY",
        ));
    }
    if tuning.format.is_some() && !search_command {
        return Err(usage_error(
            "--format applies only to search, keyword, or semantic commands",
        ));
    }
    if tuning.file_filter.is_some() && !search_command {
        return Err(usage_error(
            "--file-filter applies only to search, keyword, or semantic commands",
        ));
    }
    // An unknown --lang label must fail closed as a usage error instead of
    // lowercasing into a silent filter-to-nothing (the reference exits 2 on
    // unknown languages). Language::parse accepts every stored id, indexed
    // extension, and alias (py, rs, ts, hpp, Title Case, golang, c#, c++).
    if let Some(raw) = cli.lang.as_deref() {
        if ast_sgrep_core::Language::parse(raw).is_none() {
            return Err(usage_error(format!(
                "unknown --lang '{raw}' (expected a stored id, file extension, or alias such as py, rs, ts, hpp)"
            )));
        }
    }
    Ok(())
}

fn run_command(cli: &Cli, command: &Commands) -> anyhow::Result<()> {
    match command {
        Commands::Index(c) => {
            if !c.paths.is_empty() {
                if c.dry_run {
                    return Err(usage_error("--dry-run and --path are mutually exclusive"));
                }
                return run_targeted_index(&c.root.root, cli, &c.paths, c.scip.as_deref());
            }
            if c.dry_run {
                return run_index_dry_run("index", &c.root.root, cli);
            }
            run_full_index("index", &c.root.root, cli, false, c.scip.as_deref())
        }
        Commands::Status(r) => print_status_command(cli, &r.root),
        Commands::Outline(o) => outline_cmd::run_outline(cli, &o.root.root, &o.path),
        Commands::Reindex(c) => {
            if c.dry_run {
                return run_index_dry_run("reindex", &c.root.root, cli);
            }
            run_full_index("reindex", &c.root.root, cli, true, c.scip.as_deref())
        }
        Commands::Codemod(c) => codemod_cmd::run_codemod(cli, c),
        Commands::Search(q) => {
            if q.pattern.is_empty() {
                let query = q.query.as_deref().unwrap_or_default();
                search_cmd::run_search(&q.root.root, cli, query, false)
            } else if q.query.as_deref().is_some_and(|s| !s.is_empty()) {
                Err(usage_error(
                    "pass either a QUERY positional or --pattern flags, not both \
                     (batch usage: asgrep search --pattern P1 --pattern P2 --root ROOT)",
                ))
            } else {
                search_cmd::run_multi_pattern_search(&q.root.root, cli, &q.pattern)
            }
        }
        Commands::Bench {
            root,
            query,
            iterations,
            suite,
            fixture,
            queries_file,
            skip_index,
            ..
        } => bench::run_bench_command(
            &root.root,
            cli,
            query,
            *iterations,
            suite.as_deref(),
            fixture,
            queries_file.as_deref(),
            *skip_index,
        ),
        Commands::Watch { root, .. } => {
            ensure_unambiguous_root(&root.root, cli)?;
            watch::run_watch_via_helper()
        }
        Commands::Keyword(q) => search_cmd::run_keyword_search(&q.query.root, cli, &q.query.query),
        Commands::Semantic(q) => search_cmd::run_search(&q.query.root, cli, &q.query.query, true),
        Commands::Chain(q) => search_cmd::run_chain(
            &q.query.root,
            cli,
            &q.query.query,
            usize::try_from(q.max_work).expect("bounded chain work fits usize"),
        ),
        Commands::CallPath(args) => search_cmd::run_call_path(args, cli),
        Commands::Capabilities(args) => agent::run_capabilities(cli, args),
        Commands::Version(args) => run_version(cli, args),
        Commands::RobotDocs(args) => agent::run_robot_docs(cli, args),
        Commands::Doctor { root, args } => agent::run_doctor(cli, &root.root, args),
        Commands::Eval(args) => eval::run_eval(cli, args),
        Commands::CodemodeBatch { requests, .. } => run_codemode_batch(cli, requests),
        Commands::CodemodeServe { .. } => run_codemode_serve(cli),
        Commands::Install(args) => install::run_install(args),
    }
}

/// Run the file-watcher callback with the CLI's parsing, validation, error
/// envelopes and exit classes, including failures after the shim has exec'd.
/// The callback keeps the `notify` dependency out of the main CLI binary.
pub fn run_watch_process(
    watch: impl FnOnce(ast_sgrep_core::IndexOptions, u64) -> anyhow::Result<()>,
) -> ! {
    run_process_with(|cli| {
        validate_cli(cli)?;
        let Some(Commands::Watch { root, debounce_ms }) = &cli.command else {
            return Err(usage_error(
                "asgrep-watch only runs `watch`; use asgrep for other commands",
            ));
        };
        let root = ensure_existing_root(&root.root, cli)?;
        watch(index_options(&root, cli), *debounce_ms)
    })
}

fn codemode_session_config(cli: &Cli, root: PathBuf) -> ast_sgrep_codemode::SessionConfig {
    ast_sgrep_codemode::SessionConfig {
        root,
        index_path: cli.index_path.clone(),
        limit: ast_sgrep_core::clamp_output_limit(
            cli.limit,
            ast_sgrep_core::SearchOptions::default_limit(),
        ),
        use_embed: !cli.active_tuning().no_embed,
        ..ast_sgrep_codemode::SessionConfig::default()
    }
}

fn run_codemode_batch(cli: &Cli, requests: &Path) -> anyhow::Result<()> {
    let raw = load_batch_raw(requests)?;
    let mut request: ast_sgrep_codemode::BatchRequest =
        serde_json::from_str(&raw).context("parse batch requests JSON")?;
    apply_cli_batch_defaults(&mut request, cli);
    let config = ast_sgrep_codemode::SessionConfig {
        root: request.root.clone().unwrap_or_else(|| PathBuf::from(".")),
        index_path: request.index_path.clone(),
        limit: ast_sgrep_core::clamp_output_limit(
            request.limit,
            ast_sgrep_core::SearchOptions::default_limit(),
        ),
        use_embed: request.use_embed.unwrap_or(true),
        ..ast_sgrep_codemode::SessionConfig::default()
    };
    let response = ast_sgrep_codemode::run_batch(config, &request)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    // Transport ok=true even when some calls fail — per-call status lives in results[].
    // (ok:false would make Pi parseEnvelope throw and re-run the whole wave.)
    let envelope = serde_json::json!({
        "schema_version": MACHINE_SCHEMA_VERSION,
        "tool": "asgrep",
        "command": "codemode-batch",
        "ok": true,
        "all_ok": response.all_ok,
        "version": env!("CARGO_PKG_VERSION"),
        "machine_schema_version": MACHINE_SCHEMA_VERSION,
        "call_count": response.call_count,
        "wall_ms": response.wall_ms,
        "mode": response.mode,
        "results": response.results,
    });
    // Compact JSON: Code Mode waves are hot; pretty-print is pure serial waste.
    // Broken-pipe safe: agents often pipe batch JSON through head/jq.
    machine::write_stdout_line(&serde_json::to_string(&envelope)?)?;
    Ok(())
}

/// Cap batch payload (file *and* stdin) so a huge pipe cannot OOM the process.
fn load_batch_raw(requests: &Path) -> anyhow::Result<String> {
    use machine::{read_utf8_capped, MAX_BATCH_REQUEST_BYTES};
    if requests.as_os_str() == "-" {
        return read_utf8_capped(std::io::stdin().lock(), MAX_BATCH_REQUEST_BYTES)
            .context("read batch requests from stdin (payload exceeds max or I/O error)");
    }
    let meta = std::fs::metadata(requests)
        .with_context(|| format!("stat batch requests {}", requests.display()))?;
    anyhow::ensure!(
        meta.len() <= MAX_BATCH_REQUEST_BYTES,
        "batch requests file exceeds max {} bytes",
        MAX_BATCH_REQUEST_BYTES
    );
    // Re-cap on the read path: file may grow between stat and open (same as io_bounds).
    let file = std::fs::File::open(requests)
        .with_context(|| format!("open batch requests {}", requests.display()))?;
    read_utf8_capped(file, MAX_BATCH_REQUEST_BYTES).with_context(|| {
        format!(
            "read batch requests {} (payload exceeds max or I/O error)",
            requests.display()
        )
    })
}

/// Fill unset batch request fields from CLI flags (root, index, embed, limit).
fn apply_cli_batch_defaults(request: &mut ast_sgrep_codemode::BatchRequest, cli: &Cli) {
    if request.root.is_none() {
        request.root = Some(cli.root.clone().unwrap_or_else(|| PathBuf::from(".")));
    }
    if request.index_path.is_none() {
        request.index_path = cli.index_path.clone();
    }
    if request.use_embed.is_none() {
        request.use_embed = Some(!cli.active_tuning().no_embed);
    }
    if request.limit.is_none() {
        request.limit = cli.limit;
    }
}

fn run_codemode_serve(cli: &Cli) -> anyhow::Result<()> {
    let root = cli
        .root
        .clone()
        .or_else(|| Some(cli.search_root.clone()))
        .unwrap_or_else(|| PathBuf::from("."));
    let config = codemode_session_config(cli, root);
    let stdin = std::io::BufReader::new(std::io::stdin());
    let stdout = std::io::stdout();
    ast_sgrep_codemode::run_serve(config, stdin, stdout.lock())
        .map_err(|e| anyhow::anyhow!(e.to_string()))
}

fn run_version(cli: &Cli, args: &VersionArgs) -> anyhow::Result<()> {
    let index_schema = ast_sgrep_core::INDEX_SCHEMA_VERSION;
    if cli.json || args.json {
        print_machine_json(
            "version",
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "index_schema_version": index_schema,
                "machine_schema_version": MACHINE_SCHEMA_VERSION
            }),
        )
    } else {
        println!("asgrep {}", env!("CARGO_PKG_VERSION"));
        println!("index_schema {index_schema}");
        println!("machine_schema {MACHINE_SCHEMA_VERSION}");
        Ok(())
    }
}

fn run_default_search(cli: &Cli) -> anyhow::Result<()> {
    let query = cli.query.as_deref().ok_or_else(|| usage_error("search query required (e.g. asgrep \"auth refresh\") or use a subcommand: asgrep capabilities --json"))?;
    if let Some(sub) = agent::query_looks_like_subcommand_typo(query) {
        return Err(usage_error(format!("unknown subcommand '{query}'; did you mean: asgrep {sub} ... ? Try: asgrep capabilities --json")));
    }
    search_cmd::run_search(&cli.search_root, cli, query, false)
}
