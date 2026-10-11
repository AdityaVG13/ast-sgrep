//! Config-only agent MCP install (Wave 2).
//!
//! Writes stdio `asgrep-mcp` entries for supported hosts. Does **not** start a
//! daemon, bind a port, or download models.

use anyhow::{bail, Context};
use clap::Parser;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug, Clone)]
pub(crate) struct InstallArgs {
    /// Agent target(s): cursor|claude|codex|opencode|all (repeatable).
    #[arg(long = "target", value_name = "NAME", num_args = 1.., required = true)]
    pub(crate) targets: Vec<String>,
    /// Skip confirmation prompts.
    #[arg(long)]
    pub(crate) yes: bool,
    /// Replace an existing unmanaged `asgrep` / `ast-sgrep` MCP entry.
    #[arg(long)]
    pub(crate) force: bool,
}

#[derive(Clone, Copy)]
enum Target {
    Cursor,
    Claude,
    Codex,
    OpenCode,
}

impl Target {
    fn parse(raw: &str) -> anyhow::Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "cursor" => Ok(Self::Cursor),
            "claude" | "claude-code" | "cc" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            "opencode" => Ok(Self::OpenCode),
            "all" => bail!("expand 'all' before parse"),
            other => bail!(
                "unknown install target '{other}' (expected cursor|claude|codex|opencode|all)"
            ),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Cursor => "cursor",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }
}

pub(crate) fn run_install(args: &InstallArgs) -> anyhow::Result<()> {
    if !args.yes {
        bail!("pass --yes to write agent MCP config (non-interactive; no daemon is started)");
    }
    let command = resolve_mcp_command()?;
    let mut targets = Vec::new();
    for raw in &args.targets {
        if raw.eq_ignore_ascii_case("all") {
            targets.extend([
                Target::Cursor,
                Target::Claude,
                Target::Codex,
                Target::OpenCode,
            ]);
            continue;
        }
        targets.push(Target::parse(raw)?);
    }
    targets.sort_by_key(|t| t.label());
    targets.dedup_by_key(|t| t.label());

    let mut written = Vec::new();
    for target in targets {
        let path = config_path(target)?;
        install_target(target, &path, &command, args.force)?;
        written.push(format!("{} → {}", target.label(), path.display()));
    }
    for line in written {
        println!("installed: {line}");
    }
    println!(
        "stdio MCP only — no daemon started. Restart the agent (or open a new session) to load asgrep-mcp."
    );
    Ok(())
}

fn resolve_mcp_command() -> anyhow::Result<String> {
    if let Ok(path) = std::env::var("ASGREP_MCP_BIN") {
        let trimmed = path.trim();
        anyhow::ensure!(!trimmed.is_empty(), "ASGREP_MCP_BIN is empty");
        return Ok(trimmed.to_owned());
    }
    if which("asgrep-mcp").is_some() {
        return Ok("asgrep-mcp".into());
    }
    if which("asgrep_mcp").is_some() {
        return Ok("asgrep_mcp".into());
    }
    bail!(
        "asgrep-mcp not found on PATH; install the MCP binary or set ASGREP_MCP_BIN to its absolute path"
    )
}

pub(crate) fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{name}.exe"));
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

fn config_path(target: Target) -> anyhow::Result<PathBuf> {
    let home = dirs_home()?;
    Ok(match target {
        Target::Cursor => {
            let base = std::env::var_os("CURSOR_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".cursor"));
            base.join("mcp.json")
        }
        Target::Claude => {
            let base = std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude"));
            base.join("settings.json")
        }
        Target::Codex => {
            let base = std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"));
            base.join("config.toml")
        }
        Target::OpenCode => {
            let base = std::env::var_os("OPENCODE_CONFIG")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config").join("opencode").join("opencode.json"));
            if base.extension().and_then(|e| e.to_str()) == Some("json") {
                base
            } else {
                base.join("opencode.json")
            }
        }
    })
}

fn dirs_home() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("HOME / USERPROFILE is unset")
}

fn install_target(target: Target, path: &Path, command: &str, force: bool) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    match target {
        Target::Codex => install_codex_toml(path, command, force),
        Target::Cursor | Target::Claude | Target::OpenCode => {
            install_json_mcp(path, command, force, target)
        }
    }
}

fn mcp_server_entry(command: &str) -> Value {
    json!({
        "command": command,
        "args": [],
        "env": {}
    })
}

fn install_json_mcp(path: &Path, command: &str, force: bool, target: Target) -> anyhow::Result<()> {
    let mut root = read_json_object(path)?;
    let servers = match target {
        Target::Claude => {
            // Claude Code settings use mcpServers at top level (same shape as Cursor).
            ensure_object(&mut root, "mcpServers")?
        }
        Target::Cursor | Target::OpenCode => ensure_object(&mut root, "mcpServers")?,
        Target::Codex => unreachable!(),
    };
    let replaced_names = ["asgrep", "ast-sgrep", "zvec_grep"];
    for key in replaced_names {
        if servers.contains_key(key) && !force {
            bail!(
                "{} already has mcpServers.{key}; pass --force to replace (path {})",
                target.label(),
                path.display()
            );
        }
    }
    // Admission and cleanup must agree on every name this install supersedes.
    for key in replaced_names {
        servers.remove(key);
    }
    servers.insert("asgrep".to_owned(), mcp_server_entry(command));
    write_json(path, &Value::Object(root))?;
    Ok(())
}

fn install_codex_toml(path: &Path, command: &str, force: bool) -> anyhow::Result<()> {
    let existing = if path.exists() {
        fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?
    } else {
        String::new()
    };
    let mut doc = existing
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("parse {}", path.display()))?;
    if !doc.contains_key("mcp_servers") {
        let mut table = toml_edit::Table::new();
        table.set_implicit(true);
        doc["mcp_servers"] = toml_edit::Item::Table(table);
    }
    let inline = doc["mcp_servers"].is_inline_table();
    let servers = doc["mcp_servers"]
        .as_table_like_mut()
        .context("mcp_servers must be a TOML table")?;
    if !force && (servers.contains_key("asgrep") || servers.contains_key("ast-sgrep")) {
        bail!(
            "codex config already has an asgrep MCP server; pass --force to replace ({})",
            path.display()
        );
    }
    // Replace the whole server subtree, not marker-looking text in authored data.
    servers.remove("asgrep");
    servers.remove("ast-sgrep");
    let mut server = toml_edit::Table::new();
    server["command"] = toml_edit::value(command);
    server["args"] = toml_edit::value(toml_edit::Array::new());
    let entry = if inline {
        toml_edit::Item::Value(toml_edit::Value::InlineTable(server.into_inline_table()))
    } else {
        toml_edit::Item::Table(server)
    };
    servers.insert("asgrep", entry);
    fs::write(path, doc.to_string()).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn read_json_object(path: &Path) -> anyhow::Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    let value: Value =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => bail!("{} must contain a JSON object", path.display()),
    }
}

fn ensure_object<'a>(
    root: &'a mut Map<String, Value>,
    key: &str,
) -> anyhow::Result<&'a mut Map<String, Value>> {
    if !root.contains_key(key) {
        root.insert(key.to_owned(), Value::Object(Map::new()));
    }
    root.get_mut(key)
        .and_then(Value::as_object_mut)
        .context("mcpServers must be a JSON object")
}

fn write_json(path: &Path, value: &Value) -> anyhow::Result<()> {
    let body = serde_json::to_string_pretty(value)?;
    fs::write(path, format!("{body}\n")).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}
