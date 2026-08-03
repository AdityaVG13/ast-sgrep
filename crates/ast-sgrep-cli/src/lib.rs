#![forbid(unsafe_code)]

mod agent;
mod bench;
mod cli_args;
mod eval;
mod index_cmd;
mod machine;
mod search_cmd;
pub mod supervisor;
mod watch;

use anyhow::Context;
use clap::Parser;
use cli_args::{Cli, Commands, VersionArgs};
use machine::{
    print_machine_failure, print_machine_json, raw_command_name, raw_machine_output_requested,
    MACHINE_SCHEMA_VERSION,
};
use index_cmd::{
    print_index_stats, run_index_dry_run, print_status_command, with_index,
};

pub(crate) use cli_args::{UsageError, usage_error};
pub(crate) use index_cmd::{
    ensure_existing_root, ensure_nonempty_index, ensure_unambiguous_root, effective_root,
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
            if exit_code == 1 && raw_machine_output_requested(&raw_args) {
                print_machine_failure(
                    raw_command_name(&raw_args),
                    "usage",
                    exit_code,
                    &error.to_string(),
                );
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

fn run_cli(cli: &Cli) -> anyhow::Result<()> {
    if cli.robot_help {
        agent::print_robot_guide();
        return Ok(());
    }
    if cli.active_tuning().format.is_some()
        && !matches!(
            cli.command.as_ref(),
            None
                | Some(
                    Commands::Search(_)
                        | Commands::Keyword(_)
                        | Commands::Semantic(_)
                        | Commands::Index(_)
                        | Commands::Reindex(_)
                        | Commands::Bench { .. }
                )
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
            if c.dry_run {
                return run_index_dry_run("index", &c.root.root, cli);
            }
            with_index(
                "index",
                &c.root.root,
                cli,
                |i| {
                    if !cli.json {
                        eprintln!("asgrep: indexing {} ...", c.root.root.display());
                    }
                    i.index_all().context("indexing failed")
                },
                print_index_stats,
            )
        }
        Commands::Status(r) => print_status_command(cli, &r.root),
        Commands::Reindex(c) => {
            if c.dry_run {
                return run_index_dry_run("reindex", &c.root.root, cli);
            }
            with_index(
                "reindex",
                &c.root.root,
                cli,
                |i| {
                    if !cli.json {
                        eprintln!("asgrep: reindexing {} ...", c.root.root.display());
                    }
                    i.reindex_all().context("reindex failed")
                },
                print_index_stats,
            )
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
        Commands::Keyword(q) => {
            search_cmd::run_keyword_search(&q.query.root, cli, &q.query.query)
        }
        Commands::Semantic(q) => search_cmd::run_search(&q.query.root, cli, &q.query.query, true),
        Commands::Chain(q) => search_cmd::run_chain(&q.root, cli, &q.query),
        Commands::Capabilities(args) => agent::run_capabilities(cli, args),
        Commands::Version(args) => run_version(cli, args),
        Commands::RobotDocs(args) => agent::run_robot_docs(cli, args),
        Commands::Doctor { root, args } => agent::run_doctor(cli, &root.root, args),
        Commands::Eval(args) => eval::run_eval(cli, args),
    }
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
