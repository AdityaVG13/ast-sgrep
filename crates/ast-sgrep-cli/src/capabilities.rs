use crate::Cli;
use clap::CommandFactory;
use serde_json::{json, Value};

/// Static capabilities skeleton (was one 60-line `json!` literal: 26KiB of
/// .text for static data). Dynamic holes — version, description, the
/// clap-derived catalog, and the two `MAX_*` consts — are inserted below so
/// they can never drift from their sources. Regenerate from a trusted binary:
/// `capabilities --json` minus the envelope and the 7 dynamic keys.
const CAPABILITIES_SKELETON: &str = include_str!("capabilities_data.json");

pub(crate) fn capabilities_json(_cli: &Cli) -> anyhow::Result<Value> {
    let command = crate::Cli::command();
    let (commands, global_flags, search_tuning_flags) = clap_catalog(&command);
    let mut caps: Value = serde_json::from_str(CAPABILITIES_SKELETON)
        .map_err(|e| anyhow::anyhow!("checked-in capabilities skeleton must parse: {e}"))?;
    let obj = caps
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("capabilities skeleton must be an object"))?;
    obj.insert(
        "version".to_string(),
        Value::String(env!("CARGO_PKG_VERSION").to_string()),
    );
    obj.insert(
        "description".to_string(),
        Value::String(
            command
                .get_about()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "Polyglot hybrid code search".into()),
        ),
    );
    obj.insert("commands".to_string(), Value::Array(commands));
    obj.insert(
        "global_flags".to_string(),
        Value::Array(global_flags.into_iter().map(Value::String).collect()),
    );
    obj.insert(
        "search_tuning_flags".to_string(),
        Value::Array(search_tuning_flags.into_iter().map(Value::String).collect()),
    );
    let limits = obj
        .get_mut("output_limits")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow::anyhow!("capabilities skeleton must carry output_limits"))?;
    limits.insert(
        "max_results".to_string(),
        json!(ast_sgrep_core::MAX_OUTPUT_RESULTS),
    );
    limits.insert(
        "max_excerpt_lines".to_string(),
        json!(ast_sgrep_core::MAX_EXCERPT_LINES),
    );
    Ok(caps)
}

/// Key for the bare-default row in [`COMMAND_EXAMPLES`] (no subcommand).
pub(crate) const BARE_DEFAULT_EXAMPLE: &str = "bare";

/// Per-command example command lines for the clap catalog and clap usage
/// teaching. Single source — to annotate a command, add a row here, not a
/// new literal at the use site.
const COMMAND_EXAMPLES: &[(&str, &str)] = &[
    (
        "search",
        r#"asgrep search --json --format compact "auth refresh" ."#,
    ),
    ("keyword", r#"asgrep keyword --json "auth refresh" ."#),
    (
        "semantic",
        r#"asgrep semantic --json "where is auth refreshed" ."#,
    ),
    (
        "chain",
        r#"asgrep chain "callers:process_request" ."#,
    ),
    (
        "call-path",
        r#"asgrep call-path main validate_input ."#,
    ),
    (
        "codemod",
        r#"asgrep codemod --dry-run --pattern 'legacy($ARG)' --rewrite 'modern($ARG)' ."#,
    ),
    // Bare default (no subcommand): the fallback for commands without a row.
    (
        BARE_DEFAULT_EXAMPLE,
        r#"asgrep --json --format compact "auth refresh" ."#,
    ),
];

/// Example command line for `name` (`None` when the command has no example row).
pub(crate) fn command_example(name: &str) -> Option<&'static str> {
    COMMAND_EXAMPLES
        .iter()
        .find(|(row, _)| *row == name)
        .map(|(_, example)| *example)
}

fn clap_catalog(command: &clap::Command) -> (Vec<Value>, Vec<String>, Vec<String>) {
    const SEARCH_TUNING: &[&str] = &[
        "--no-embed",
        "--semantic-only",
        "--tantivy",
        "--ann-threshold",
        "--ann-probes",
        "--format",
        "--excerpt-lines",
        "--snippet-tokens",
        "--response-snippet-tokens",
        // m38g: whole-response token budget that picks per-result detail.
        "--budget-tokens",
    ];
    let mut global_flags = Vec::new();
    let mut search_tuning_flags = Vec::new();
    for arg in command.get_arguments() {
        // Hidden args are retired stubs (parse-only, rejected later); never
        // advertise them to agents.
        if arg.is_hide_set() {
            continue;
        }
        let Some(long) = arg.get_long() else { continue };
        let flag = format!("--{long}");
        if arg.is_global_set() {
            global_flags.push(flag);
            if let Some(short) = arg.get_short() {
                global_flags.push(format!("-{short}"));
            }
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
            if arg.is_hide_set() {
                continue;
            }
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
        if crate::is_search_family(name.as_str()) {
            entry["robot_output"] = json!("--format implies --json; formats: native|agent|agent-capsule|compact|github|gitlab");
            if let Some(example) = command_example(&name) {
                entry["example"] = json!(example);
            }
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
        if name == "codemod" {
            entry["safe_mutating"] = json!({
                "kind": "source_rewrite",
                "prefer_first": "asgrep codemod --dry-run --pattern '<pattern>' --rewrite '<rewrite>' <ROOT>",
                "apply": "asgrep codemod --yes --pattern '<pattern>' --rewrite '<rewrite>' <ROOT>",
                "note": "apply requires --yes (alias --force); --dry-run emits a JSON plan without writing source"
            });
            if let Some(example) = command_example("codemod") {
                entry["example"] = json!(example);
            }
        }
        commands.push(entry);
    }
    commands.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    (commands, global_flags, search_tuning_flags)
}
