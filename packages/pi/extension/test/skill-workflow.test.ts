import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerAstSgrepCommands, registerAstSgrepTools } from "../src/index.js";
import type { MachineEnvelope } from "../src/runtime.js";

test("a deterministic agent can discover and complete the documented fixture workflow", async () => {
  const packageRoot = new URL("..", import.meta.url);
  const manifest = JSON.parse(await readFile(new URL("package.json", packageRoot), "utf8")) as {
    pi: { skills: string[] };
  };
  assert.deepEqual(manifest.pi.skills, ["./skills"]);
  const skill = await readFile(new URL("skills/ast-sgrep/SKILL.md", packageRoot), "utf8");
  for (const instruction of ["exact-text search", "`natural`:", "`defs`:", "`callers`:", "/asgrep-doctor", "/asgrep-index"]) {
    assert.match(skill, new RegExp(instruction.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  }

  type RegisteredCommand = { description: string; handler(args: string, context: unknown): Promise<void> };
  type RegisteredTool = {
    name: string;
    description: string;
    execute(id: string, params: Record<string, unknown>, signal: AbortSignal, update: undefined, context: { cwd: string }): Promise<unknown>;
  };
  const commands = new Map<string, RegisteredCommand>();
  const tools = new Map<string, RegisteredTool>();
  const argv: readonly string[][] = [];
  let indexed = false;
  const runtime = {
    async resolveRoot(context: { cwd: string }) { return context.cwd; },
    async run(args: readonly string[]): Promise<MachineEnvelope> {
      argv.push([...args]);
      if (args[0] === "index") indexed = true;
      if (args[0] === "status") {
        return { tool: "asgrep", schema_version: "1.0.0", ok: true, status: indexed ? "ready" : "missing", index: { exists: indexed } };
      }
      if (args[0] === "doctor") return { tool: "asgrep", schema_version: "1.0.0", ok: true, status: "healthy" };
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [{ path: "src/fixture.ts", symbol: "ensureFresh" }] };
    },
  };
  const pi = {
    registerCommand(name: string, command: RegisteredCommand) { commands.set(name, command); },
    registerTool(tool: RegisteredTool) { tools.set(tool.name, tool); },
    on() {},
  } as unknown as ExtensionAPI;
  registerAstSgrepCommands(pi, runtime);
  registerAstSgrepTools(pi, runtime, { async ensureFresh() {}, markAffectedPath() {} });

  const notices: string[] = [];
  const commandContext = { cwd: "/fixture", hasUI: false, ui: { notify(message: string) { notices.push(message); } } };
  await commands.get("asgrep-doctor")!.handler("", commandContext);
  await commands.get("asgrep-status")!.handler("", commandContext);
  await commands.get("asgrep-index")!.handler("", commandContext);
  const search = tools.get("asgrep_search")!;
  assert.match(search.description, /natural language.*symbol relationships/i);
  const signal = new AbortController().signal;
  await search.execute("intent", { query: "refresh the index after edits", mode: "natural" }, signal, undefined, { cwd: "/fixture" });
  await search.execute("callers", { query: "ensureFresh", mode: "callers", limit: 8 }, signal, undefined, { cwd: "/fixture" });

  assert.equal(JSON.parse(notices[0]!).response.status, "healthy");
  assert.deepEqual(argv, [
    ["doctor", ".", "--json"],
    ["status", ".", "--json"],
    ["index", ".", "--json"],
    ["--json", "--format", "agent-capsule", "--limit", "8", "--excerpt-lines", "0", "refresh the index after edits", "."],
    ["--json", "--format", "agent-capsule", "--limit", "8", "--excerpt-lines", "0", "callers: ensureFresh", "."],
  ]);
});
