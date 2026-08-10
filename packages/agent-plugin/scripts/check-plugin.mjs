import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const plugin = JSON.parse(readFileSync(join(root, "plugin.json"), "utf8"));
const mcp = JSON.parse(readFileSync(join(root, "mcp.json"), "utf8"));
const skill = join(root, "skills/ast-sgrep/SKILL.md");
readFileSync(skill, "utf8");

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
console.log("agent-plugin ok:", plugin.name, Object.keys(mcp.mcpServers).join(","));
