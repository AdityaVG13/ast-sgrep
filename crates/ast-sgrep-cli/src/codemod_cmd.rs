//! Indexed structural codemod command.

use crate::cli_args::{usage_error, Cli, CodemodCmd};
use crate::index_cmd::{ensure_existing_root, index_options};
use crate::machine::print_machine_json;
use anyhow::{bail, Context};
use ast_sgrep_core::codemod::{apply_codemod, plan_codemod};
use ast_sgrep_core::Indexer;

pub(crate) fn run_codemod(cli: &Cli, command: &CodemodCmd) -> anyhow::Result<()> {
    if !command.dry_run && !cli.yes {
        return Err(usage_error(
            "refusing to apply a codemod without --yes. Plan first: asgrep codemod --dry-run --pattern 'legacy($ARG)' --rewrite 'modern($ARG)' .\nThen apply: asgrep codemod --yes --pattern 'legacy($ARG)' --rewrite 'modern($ARG)' .",
        ));
    }
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
    options.root = root.clone();
    let refresh = (|| -> anyhow::Result<_> {
        let mut indexer = Indexer::new(options).context("could not open the index")?;
        indexer
            .update_paths(&changed_paths)
            .context("incremental refresh did not complete")
    })();
    let refresh = match refresh {
        Ok(refresh) => refresh,
        Err(error) => bail!(
            "codemod source transaction committed, but index refresh failed: {error:#}. Source changes remain applied; recover by running `asgrep index` for project root {}",
            root.display()
        ),
    };

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
