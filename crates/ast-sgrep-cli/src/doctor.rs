use crate::Cli;
use anyhow::Context;
use ast_sgrep_core::semantic_ann::should_use_ann;
use ast_sgrep_core::{IndexStore, StoreError, INDEX_SCHEMA_VERSION};
use clap::Parser;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Parser, Clone)]
pub(crate) struct DoctorArgs {
    #[arg(long)]
    pub(crate) json: bool,
    #[arg(long = "robot-triage")]
    pub(crate) robot_triage: bool,
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
