# asgrep Agent Plugin

Portable [Agent Plugins](https://agent-plugins.org/) package for ast-sgrep.

## Contents

- `plugin.json` -- Agent Plugins 1.0.0 manifest
- `skills/ast-sgrep/` -- Agent Skill (same guidance as the Pi package)
- `mcp.json` -- stdio MCP server via `asgrep-mcp`

## Requirements

- `asgrep-mcp` on `PATH` (from a published `ast-sgrep-mcp` install / release binary), or replace `command` with an absolute path
- Optional: native `asgrep` CLI for non-MCP workflows

## Install (client-specific)

Clients that support Agent Plugins load this directory as a plugin root. Example layout after clone:

```text
packages/agent-plugin/
├── plugin.json
├── mcp.json
└── skills/ast-sgrep/SKILL.md
```

Pi users should prefer `pi install npm:pi-ast-sgrep` for the full Code Mode + tools surface. This plugin is the portable skills+MCP floor for other agent clients.

## Code Mode XOR MCP

This plugin is the **MCP** path for non-Pi clients.

If you use Pi, install `npm:pi-ast-sgrep` (Code Mode) instead and do **not** also load this plugin / `asgrep-mcp` in that same agent. Choose one surface.
