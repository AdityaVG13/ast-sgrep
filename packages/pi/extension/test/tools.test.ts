import assert from "node:assert/strict";
import test from "node:test";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerAstSgrepTools } from "../src/index.js";
import { RuntimeError, type MachineEnvelope } from "../src/runtime.js";

type Tool = {
  name: string;
  parameters: { properties: Record<string, Record<string, unknown>>; additionalProperties?: boolean };
  execute(id: string, params: Record<string, unknown>, signal: AbortSignal, onUpdate: (value: unknown) => void, ctx: { cwd: string }): Promise<{ content: Array<{ text: string }>; details: Record<string, unknown> }>;
};

type Call = { args: readonly string[]; context: { cwd: string }; options: { signal?: AbortSignal } };

function fixture(response: MachineEnvelope = { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [] }) {
  const tools: Tool[] = [];
  const calls: Call[] = [];
  const handlers: Array<(event: Record<string, unknown>, ctx: { cwd: string }) => void> = [];
  const pi = {
    registerTool(tool: Tool) { tools.push(tool); },
    on(event: string, handler: (event: Record<string, unknown>, ctx: { cwd: string }) => void) { if (event === "tool_result") handlers.push(handler); },
  } as unknown as ExtensionAPI;
  const runtime = {
    async resolveRoot(context: { cwd: string }) { return context.cwd; },
    async run(args: readonly string[], context: { cwd: string }, options: { signal?: AbortSignal }) {
      calls.push({ args, context, options });
      return response;
    },
  };
  const dirtied: Array<{ path: string; cwd: string }> = [];
  const freshness = {
    async ensureFresh() {},
    markAffectedPath(path: string, cwd: string) { dirtied.push({ path, cwd }); },
  };
  registerAstSgrepTools(pi, runtime, freshness);
  return { tools, calls, handlers, dirtied, byName: (name: string) => tools.find((tool) => tool.name === name)! };
}

async function invoke(tool: Tool, params: Record<string, unknown> = {}, signal = new AbortController().signal) {
  const updates: unknown[] = [];
  const result = await tool.execute("call-1", params, signal, (value) => updates.push(value), { cwd: "/project" });
  return { result, updates, signal };
}

test("registers exact Pi tool names with bounded TypeBox schemas", () => {
  const { tools, byName } = fixture();
  assert.deepEqual(tools.map(({ name }) => name), ["asgrep_search", "asgrep_index", "asgrep_status"]);
  const search = byName("asgrep_search").parameters;
  assert.equal(search.additionalProperties, false);
  assert.equal(search.properties.query.minLength, 1);
  assert.equal(search.properties.query.maxLength, 4096);
  assert.equal(search.properties.limit.minimum, 1);
  assert.equal(search.properties.limit.maximum, 100);
  assert.equal(search.properties.limit.default, 8);
  assert.equal(search.properties.excerptLines.minimum, 0);
  assert.equal(search.properties.excerptLines.maximum, 100);
  assert.equal(search.properties.excerptLines.default, 0);
  assert.equal(byName("asgrep_index").parameters.properties.force.default, false);
  assert.equal(byName("asgrep_status").parameters.additionalProperties, false);
});

test("search defaults to a small zero-excerpt agent capsule", async () => {
  const f = fixture({ tool: "asgrep", schema_version: "1.0.0", ok: true, hits: new Array(500).fill({ preview: "x".repeat(500) }) });
  const { result } = await invoke(f.byName("asgrep_search"), { query: "where auth refreshes" });
  assert.deepEqual(f.calls[0]?.args, ["--json", "--format", "agent-capsule", "--limit", "8", "--excerpt-lines", "0", "where auth refreshes", "."]);
  assert.ok(result.content[0]!.text.length <= 1200);
  assert.equal((result.details.response as MachineEnvelope).hits instanceof Array, true);
});

test("maps every query mode and bounded output option to argv arrays", async () => {
  const cases: Array<[string, string[]]> = [
    ["natural", ["--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3", "needle", "."]],
    ["pattern", ["--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3", "pattern: needle", "."]],
    ["defs", ["--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3", "defs: needle", "."]],
    ["callers", ["--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3", "callers: needle", "."]],
    ["chain", ["chain", "needle", ".", "--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3"]],
    ["semantic", ["semantic", "needle", ".", "--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3"]],
  ];
  for (const [mode, expected] of cases) {
    const f = fixture();
    await invoke(f.byName("asgrep_search"), { query: "needle", mode, limit: 25, excerptLines: 3 });
    assert.deepEqual(f.calls[0]?.args, expected, mode);
  }
});

test("index force maps only to index or reindex", async () => {
  const normal = fixture();
  await invoke(normal.byName("asgrep_index"), {});
  assert.deepEqual(normal.calls[0]?.args, ["index", ".", "--json"]);
  const forced = fixture();
  await invoke(forced.byName("asgrep_index"), { force: true });
  assert.deepEqual(forced.calls[0]?.args, ["reindex", ".", "--json"]);
});

test("status preserves version, protocol, root, index, counts, backend, IVF and capabilities", async () => {
  const response: MachineEnvelope = {
    tool: "asgrep", schema_version: "1.0.0", ok: true, command: "status", version: "1.2.0-alpha",
    machine_schema_version: "1.0.0", root: "/project", index_path: "/project/.asgrep/index.db",
    counts: { files: 12, symbols: 34 }, backend: "fastembed", ivf: { clusters: 4, probes: 2 }, capabilities: ["semantic", "chain"],
  };
  const f = fixture(response);
  const { result } = await invoke(f.byName("asgrep_status"));
  assert.deepEqual(f.calls[0]?.args, ["status", ".", "--json"]);
  assert.deepEqual(result.details.response, response);
});

test("forwards progress, project cwd, and cancellation signal", async () => {
  const f = fixture();
  const controller = new AbortController();
  controller.abort();
  const { updates } = await invoke(f.byName("asgrep_search"), { query: "x" }, controller.signal);
  assert.equal(f.calls[0]?.context.cwd, "/project");
  assert.equal(f.calls[0]?.options.signal, controller.signal);
  assert.deepEqual(updates, [
    { content: [{ type: "text", text: "search started" }], details: { command: "search", phase: "started" } },
    { content: [{ type: "text", text: "search completed" }], details: { command: "search", phase: "completed" } },
  ]);
});

test("marks successful official write and edit tool results dirty", () => {
  const f = fixture();
  const emit = f.handlers[0]!;
  emit({ toolName: "write", input: { path: "src/new.ts" }, isError: false }, { cwd: "/project" });
  emit({ toolName: "edit", input: { path: "/project/src/existing.ts" }, isError: false }, { cwd: "/project" });
  emit({ toolName: "write", input: { path: "ignored.ts" }, isError: true }, { cwd: "/project" });
  emit({ toolName: "bash", input: { command: "touch hidden" }, isError: false }, { cwd: "/project" });
  assert.deepEqual(f.dirtied, [
    { path: "src/new.ts", cwd: "/project" },
    { path: "/project/src/existing.ts", cwd: "/project" },
  ]);
});

test("search refreshes before querying and refuses unknown index health", async () => {
  const tools: Tool[] = [];
  const handlers: Array<(event: Record<string, unknown>, ctx: { cwd: string }) => void> = [];
  const pi = {
    registerTool(tool: Tool) { tools.push(tool); },
    on(_event: string, handler: (event: Record<string, unknown>, ctx: { cwd: string }) => void) { handlers.push(handler); },
  } as unknown as ExtensionAPI;
  const calls: string[] = [];
  let status: MachineEnvelope = { tool: "asgrep", schema_version: "1.0.0", ok: true, index: { exists: false, compatible: true, status: "missing" } };
  const runtime = {
    async resolveRoot(context: { cwd: string }) { return context.cwd; },
    async run(args: readonly string[]) {
      calls.push(args[0]!);
      return args[0] === "status" ? status : { tool: "asgrep" as const, schema_version: "1.0.0", ok: true, hits: [] };
    },
  };
  registerAstSgrepTools(pi, runtime);
  await invoke(tools[0]!, { query: "first" });
  assert.deepEqual(calls, ["status", "index", "--json"]);

  handlers[0]!({ toolName: "edit", input: { path: "src/a.ts" }, isError: false }, { cwd: "/project" });
  status = { tool: "asgrep", schema_version: "1.0.0", ok: true };
  const { result } = await invoke(tools[0]!, { query: "blocked" });
  assert.equal((result.details.error as { code: string }).code, "INDEX_STATUS_UNKNOWN");
  assert.deepEqual(calls, ["status", "index", "--json", "status"]);
});

test("maps runtime failures to concise structured tool errors", async () => {
  const tools: Tool[] = [];
  const pi = { registerTool(tool: Tool) { tools.push(tool); }, on() {} } as unknown as ExtensionAPI;
  const runtime = {
    async resolveRoot(context: { cwd: string }) { return context.cwd; },
    async run() { throw new RuntimeError("CANCELLED", "execution cancelled", { source: "signal" }); },
  };
  registerAstSgrepTools(pi, runtime);
  const { result } = await invoke(tools[0]!, { query: "x" });
  assert.equal(result.content[0]!.text, "search failed [CANCELLED]: execution cancelled");
  assert.deepEqual(result.details, {
    ok: false,
    command: "search",
    error: { code: "CANCELLED", message: "execution cancelled", details: { source: "signal" } },
  });
});
