#![forbid(unsafe_code)]

mod agent;
mod bench;
mod cli_args;
mod eval;
mod index_cmd;
mod keep_gate;
mod machine;
mod search_cmd;
pub mod supervisor;
mod watch;

use anyhow::Context;
use clap::Parser;
use cli_args::{Cli, Commands, VersionArgs};
use index_cmd::{print_status_command, run_full_index, run_index_dry_run, run_targeted_index};
use machine::{
    print_machine_failure, print_machine_json, raw_command_name, raw_machine_output_requested,
    MACHINE_SCHEMA_VERSION,
};
use std::path::{Path, PathBuf};

pub(crate) use cli_args::{usage_error, UsageError};
pub(crate) use index_cmd::{
    effective_root, ensure_existing_root, ensure_nonempty_index, ensure_unambiguous_root,
    index_options, open_indexer, open_searcher, resolve_root_index, search_options,
};
pub(crate) use machine::print_machine_json_status;

pub fn main() -> anyhow::Result<()> {
    #[cfg(not(unix))]
    {
        run_process()
    }
    #[cfg(unix)]
    {
        if supervisor::is_worker() {
            if supervisor::worker_authenticate() {
                supervisor::worker_start();
                run_process()
            } else {
                supervisor::supervise()
            }
        } else {
            supervisor::supervise()
        }
    }
}

fn run_process() -> ! {
    let raw_args: Vec<_> = std::env::args_os().collect();
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
            msg = agent::augment_clap_usage_message(&msg, command);
            let augmented = msg != error.to_string();
            if exit_code == 1 && raw_machine_output_requested(&raw_args) {
                print_machine_failure(command, "usage", exit_code, &msg);
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
    match run_cli(&cli) {
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
    // --format is search-only (implies machine JSON for search envelopes).
    // Index/reindex/bench accept --json for machine output; do not accept and
    // silently ignore --format (d2a1.12).
    if cli.active_tuning().format.is_some()
        && !matches!(
            cli.command.as_ref(),
            None | Some(Commands::Search(_) | Commands::Keyword(_) | Commands::Semantic(_))
        )
    {
        return Err(usage_error(
            "--format applies only to search, keyword, or semantic commands",
        ));
    }
    match cli.command.as_ref() {
        Some(c) => run_command(cli, c),
        None => run_default_search(cli),
    }
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
        Commands::Reindex(c) => {
            if c.dry_run {
                return run_index_dry_run("reindex", &c.root.root, cli);
            }
            run_full_index("reindex", &c.root.root, cli, true, c.scip.as_deref())
        }
        Commands::Search(q) => search_cmd::run_search(&q.query.root, cli, &q.query.query, false),
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
        Commands::Watch { root, debounce_ms } => watch::run_watch(&root.root, cli, *debounce_ms),
        Commands::Keyword(q) => search_cmd::run_keyword_search(&q.query.root, cli, &q.query.query),
        Commands::Semantic(q) => search_cmd::run_search(&q.query.root, cli, &q.query.query, true),
        Commands::Chain(q) => search_cmd::run_chain(&q.root, cli, &q.query),
        Commands::Capabilities(args) => agent::run_capabilities(cli, args),
        Commands::Version(args) => run_version(cli, args),
        Commands::RobotDocs(args) => agent::run_robot_docs(cli, args),
        Commands::Doctor { root, args } => agent::run_doctor(cli, &root.root, args),
        Commands::Eval(args) => eval::run_eval(cli, args),
        Commands::CodemodeBatch { requests } => run_codemode_batch(cli, requests),
        Commands::CodemodeServe => run_codemode_serve(cli),
    }
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
    // Broken-pipe safe: agents often pipe batch JSON through head/jq (d2a1.9).
    machine::write_stdout_line(&serde_json::to_string(&envelope)?)?;
    Ok(())
}

/// Cap batch payload (file *and* stdin) so a huge pipe cannot OOM the process (d2a1.9).
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
    if cli.json || args.json {
        print_machine_json(
            "version",
            serde_json::json!({"version": env!("CARGO_PKG_VERSION"), "machine_schema_version": MACHINE_SCHEMA_VERSION}),
        )
    } else {
        println!("asgrep {}", env!("CARGO_PKG_VERSION"));
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
