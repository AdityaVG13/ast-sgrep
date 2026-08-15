import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const pluginRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = join(pluginRoot, "../..");
const plugin = JSON.parse(readFileSync(join(pluginRoot, "plugin.json"), "utf8"));
const mcp = JSON.parse(readFileSync(join(pluginRoot, "mcp.json"), "utf8"));
const pkg = JSON.parse(readFileSync(join(pluginRoot, "package.json"), "utf8"));
const workspace = JSON.parse(readFileSync(join(repoRoot, "package.json"), "utf8"));
const skill = readFileSync(join(pluginRoot, "skills/ast-sgrep/SKILL.md"), "utf8");

const mcpTools = [
  "keyword_search",
  "ast_search",
  "semantic_search",
  "code_read",
  "index_status",
  "index_repo",
  "code_search",
];

if (plugin.$schema !== "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json") {
  throw new Error("plugin.$schema mismatch");
}
if (!plugin.name || plugin.name.length > 64) throw new Error("plugin.name invalid");
if (mcp.$schema !== "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json") {
  throw new Error("mcp.$schema mismatch");
}
if (!mcp.mcpServers?.asgrep || mcp.mcpServers.asgrep.type !== "stdio") {
  throw new Error("mcpServers.asgrep stdio required");
}
if (mcp.mcpServers.asgrep.command !== "asgrep-mcp") {
  throw new Error("expected asgrep-mcp command");
}
if (typeof plugin.version !== "string" || !plugin.version) {
  throw new Error("plugin.json version required");
}
if (plugin.version !== pkg.version) {
  throw new Error(`plugin.json version ${plugin.version} != package.json ${pkg.version}`);
}
if (plugin.version !== workspace.version) {
  throw new Error(
    `agent-plugin version ${plugin.version} drifts from workspace ${workspace.version}`,
  );
}
for (const tool of mcpTools) {
  // Require the MCP tools inventory bullet (`- \`name\`: ...`), not a casual later mention.
  if (!skill.includes("- `" + tool + "`:")) {
    throw new Error(`skills/ast-sgrep/SKILL.md missing MCP tool inventory bullet: ${tool}`);
  }
}
console.log("agent-plugin ok:", plugin.name, plugin.version, Object.keys(mcp.mcpServers).join(","));
