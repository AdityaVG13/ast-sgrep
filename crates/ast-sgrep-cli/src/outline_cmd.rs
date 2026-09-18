//! File-scoped structure listing over the index.
//!
//! Beyond-reference surface by deliberate design: the reference's outline
//! contract (--items structure/exports/imports, --view
//! names/signatures/digest/expanded, per-language --outline-rules) is NOT
//! cloned — a half-clone would claim parity it cannot score. The subject
//! schema is the index's own symbol rows: name, kind, line span, byte span,
//! ordered by line_start. Outline is an INDEX READER (like status/chain):
//! a path with no indexed symbols refuses loudly instead of answering
//! silent-empty, pointing at `asgrep index`.
use crate::Cli;
use anyhow::Context;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(crate) fn run_outline(cli: &Cli, root: &Path, path: &PathBuf) -> anyhow::Result<()> {
    let root = crate::index_cmd::ensure_existing_root(root, cli)?;
    let rel = if path.is_absolute() {
        path.strip_prefix(&root).map_err(|_| {
            crate::cli_args::usage_error(format!(
                "outline path '{}' is outside root '{}'",
                path.display(),
                root.display()
            ))
        })?
    } else {
        path
    };
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let symbols = crate::index_cmd::open_readonly_store(&root, cli)?
        .symbols_in_file(&rel_str)
        .context("outline failed")?;
    if symbols.is_empty() {
        return Err(anyhow::anyhow!(
            "outline: no indexed symbols for '{rel_str}' — the file is missing, unsupported, or unindexed; run `asgrep index` first"
        ));
    }
    let payload: Vec<Value> = symbols
        .iter()
        .map(|sym| {
            json!({
                "name": sym.name,
                "kind": sym.kind,
                "line_start": sym.line_start,
                "line_end": sym.line_end,
                "byte_start": sym.byte_start,
                "byte_end": sym.byte_end,
            })
        })
        .collect();
    let count = payload.len();
    let envelope = json!({"file": rel_str, "count": count, "symbols": payload});
    if cli.json {
        crate::print_machine_json("outline", &envelope)
    } else {
        for sym in &payload {
            println!(
                "{:<12} {:<28} ({}-{})",
                sym["kind"].as_str().unwrap_or(""),
                sym["name"].as_str().unwrap_or(""),
                sym["line_start"],
                sym["line_end"]
            );
        }
        Ok(())
    }
}
