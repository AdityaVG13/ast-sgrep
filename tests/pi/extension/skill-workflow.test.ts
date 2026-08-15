import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerAstSgrepCommands, registerAstSgrepTools } from "../../../packages/pi/extension/src/index.js";
import { ASGREP_PROMPT_GUIDELINES, ASGREP_PROMPT_SNIPPET } from "../../../packages/pi/extension/src/present.js";
import type { MachineEnvelope } from "../../../packages/pi/extension/src/runtime.js";

test("tools auto-register so a deterministic agent can complete the workflow without a skill file", async () => {
  const packageRoot = new URL("../../../packages/pi/extension/", import.meta.url);
  const manifest = JSON.parse(await readFile(new URL("package.json", packageRoot), "utf8")) as {
    pi: { extensions: string[]; skills?: string[] };
    files: string[];
  };
  assert.deepEqual(manifest.pi.extensions, ["./dist/index.js"]);
  assert.equal(manifest.pi.skills, undefined);
  assert.equal(manifest.files.includes("skills"), false);

  type RegisteredCommand = { description: string; handler(args: string, context: unknown): Promise<void> };
  type RegisteredTool = {
    name: string;
    description: string;
    promptSnippet?: string;
    promptGuidelines?: string[];
    execute(id: string, params: Record<string, unknown>, signal: AbortSignal, update: undefined, context: { cwd: string }): Promise<{ content: Array<{ text: string }> }>;
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
  assert.match(search.description, /Prefer asgrep/i);
  const codemode = tools.get("asgrep")!;
  assert.equal(codemode.promptSnippet, ASGREP_PROMPT_SNIPPET);
  assert.deepEqual(codemode.promptGuidelines, [...ASGREP_PROMPT_GUIDELINES]);
  assert.match(codemode.description, /do not wait for the user to mention asgrep/i);
  assert.match(codemode.description, /Promise\.all/i);
  const signal = new AbortController().signal;
  const lookup = await search.execute("intent", { query: "refresh the index after edits", mode: "natural" }, signal, undefined, { cwd: "/fixture" });
  assert.match(lookup.content[0]!.text, /asgrep/);
  assert.match(lookup.content[0]!.text, /refresh the index after edits/);
  await search.execute("callers", { query: "ensureFresh", mode: "callers", limit: 8 }, signal, undefined, { cwd: "/fixture" });
  await codemode.execute("compose", {
    code: `async () => {
      const seed = await asgrep.search({ query: "ensureFresh", limit: 3 });
      return { n: seed.hits?.length ?? 0 };
    }`,
  }, signal, undefined, { cwd: "/fixture" });

  assert.equal(JSON.parse(notices[0]!).response.status, "healthy");
  assert.ok(argv.some((args) => args[0] === "doctor"));
  assert.ok(argv.some((args) => args.includes("agent-capsule")));
  assert.ok(argv.some((args) => args.includes("callers: ensureFresh") || args.some((a) => a.includes("callers:"))));
});
