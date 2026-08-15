//! Indexed structural codemod command.

use crate::cli_args::{Cli, CodemodCmd};
use crate::index_cmd::{ensure_existing_root, index_options};
use crate::machine::print_machine_json;
use anyhow::Context;
use ast_sgrep_core::codemod::{apply_codemod, plan_codemod};
use ast_sgrep_core::Indexer;

pub(crate) fn run_codemod(cli: &Cli, command: &CodemodCmd) -> anyhow::Result<()> {
    let root = ensure_existing_root(&command.root.root, cli)?;
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve project root: {}", root.display()))?;
    let plan = plan_codemod(
        &root,
        cli.index_path.as_deref(),
        &command.pattern,
        &command.rewrite,
    )?;

    if command.dry_run {
        return print_machine_json(
            "codemod",
            serde_json::json!({"dry_run": true, "plan": plan}),
        );
    }

    let changed_paths = plan.changed_paths();
    let result = apply_codemod(&plan)?;
    let mut options = index_options(&root, cli);
    options.root = root;
    let mut indexer = Indexer::new(options).context(
        "source files changed successfully, but the index could not be opened for refresh",
    )?;
    let refresh = indexer
        .update_paths(&changed_paths)
        .context("source files changed successfully, but the index refresh did not complete")?;

    if cli.json {
        print_machine_json(
            "codemod",
            serde_json::json!({
                "dry_run": false,
                "files_changed": result.files_changed,
                "edits_applied": result.edits_applied,
                "index_refresh": refresh,
            }),
        )
    } else {
        println!(
            "Applied {} edit(s) across {} file(s)",
            result.edits_applied, result.files_changed
        );
        Ok(())
    }
}
