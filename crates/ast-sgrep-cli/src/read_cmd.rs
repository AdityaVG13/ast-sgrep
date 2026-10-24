//! `asgrep read` — bounded source windows for file refs, the CLI parity
//! surface for MCP `code_read` and the codemode `read` tool.
//!
//! One shared contract: refs (`path`, `path#L12`, `path#L12-L40`) parse,
//! jail, and window exactly like the codemode session reader
//! (`ast_sgrep_codemode::io`), so all three surfaces answer identically.
//! Read is an INDEX-COUPLED reader like outline: opening the store inherits
//! the unindexed teaching error; windows fall back to a bounded disk scan
//! for paths the index does not cover. Fail-closed: jail escapes, inverted
//! ranges, and ref floods are usage errors before any file is opened.
use crate::cli_args::ReadArgs;
use crate::Cli;
use anyhow::Context;
use ast_sgrep_codemode::io::{
    parse_ref_str, read_one_window, ReadSpec, MAX_READ_CHARS, MAX_READ_REFS,
};
use serde_json::json;
use std::path::Path;

pub(crate) fn run_read(cli: &Cli, args: &ReadArgs) -> anyhow::Result<()> {
    // No positional ROOT: multiple REF positionals would be ambiguous, so the
    // root comes from the global --root (or cwd) exactly like --index-path.
    let root = crate::index_cmd::ensure_existing_root(Path::new("."), cli)?;
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve project root: {}", root.display()))?;
    if args.refs.len() > MAX_READ_REFS {
        return Err(crate::cli_args::usage_error(format!(
            "read exceeds max {MAX_READ_REFS} windows — pass fewer refs (batch by file instead)"
        )));
    }
    // Ref syntax and the '..' jail pre-check are usage errors; keep the deep
    // TOCTOU jail inside the shared reader.
    let specs: Vec<ReadSpec> = args
        .refs
        .iter()
        .map(|raw| {
            let spec = parse_ref_str(raw)
                .map_err(|e| crate::cli_args::usage_error(format!("read ref `{raw}`: {e}")))?;
            if Path::new(&spec.path)
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(crate::cli_args::usage_error(format!(
                    "read ref `{raw}` must not contain '..' — refs are relative to the project root"
                )));
            }
            Ok(spec)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let context_lines = args
        .context_lines
        .min(ast_sgrep_core::MAX_EXCERPT_LINES);
    let max_chars = args.max_chars.clamp(1, MAX_READ_CHARS);
    let store = crate::index_cmd::open_readonly_store(&root, cli)?;
    let mut windows = Vec::with_capacity(specs.len());
    for spec in &specs {
        windows.push(read_one_window(&store, &root, spec, context_lines, max_chars)?);
    }
    let count = windows.len();
    if cli.json {
        crate::machine::print_machine_json("read", &json!({ "count": count, "windows": windows }))
    } else {
        for window in &windows {
            let header = window["ref"].as_str().unwrap_or("");
            let truncated = window["truncated"].as_bool().unwrap_or(false);
            if truncated {
                println!("==> {header} (truncated)");
            } else {
                println!("==> {header}");
            }
            let text = window["text"].as_str().unwrap_or("");
            if !text.is_empty() {
                println!("{text}");
            }
        }
        Ok(())
    }
}
