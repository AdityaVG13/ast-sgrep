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
        cli.lang.as_deref(),
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

    // Reference-agreed refusal class for read-only targets. The reference's
    // update-all applies the writable files, skips the read-only ones
    // (`Cannot rewrite file … Permission denied`, `Skip to next file`), and
    // exits 6. Mirror that surface: the writable edits above ARE applied and
    // the index refreshed; the run then fails loudly (exit 2, this CLI's
    // operational refusal class) naming every refused file — never a silent
    // ok:true. The refusal is previewed in the dry-run plan envelope
    // (`read_only_refused`), so preview and apply stay in agreement.
    if !plan.read_only_refused.is_empty() {
        bail!(
            "codemod refused to rewrite read-only target file(s): {}; applied \
             {} edit(s) across {} file(s); chmod u+w on the listed file(s) \
             and re-run to include them",
            plan.read_only_refused.join(", "),
            result.edits_applied,
            result.files_changed
        );
    }

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
