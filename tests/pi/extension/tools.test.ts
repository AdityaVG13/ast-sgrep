import assert from "node:assert/strict";
import test from "node:test";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerAstSgrepTools } from "../../../packages/pi/extension/src/index.js";
import { RuntimeError, type MachineEnvelope } from "../../../packages/pi/extension/src/runtime.js";

type Tool = {
  name: string;
  promptSnippet?: string;
  promptGuidelines?: string[];
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

test("registers Code Mode first with auto-use prompt snippet", () => {
  const { tools, byName } = fixture();
  assert.deepEqual(tools.map(({ name }) => name), ["asgrep", "asgrep_search", "asgrep_index", "asgrep_status"]);
  assert.ok(byName("asgrep").promptSnippet);
  assert.match(byName("asgrep").promptSnippet!, /without being asked/);
  assert.ok((byName("asgrep").promptGuidelines ?? []).length >= 2);
  const search = byName("asgrep_search").parameters;
  assert.equal(search.additionalProperties, false);
  assert.equal(search.properties.query.minLength, 1);
  assert.equal(search.properties.query.maxLength, 4096);
  assert.equal(search.properties.mode.default, "natural");
  assert.equal(search.properties.limit.minimum, 1);
  assert.equal(search.properties.limit.maximum, 100);
  assert.equal(search.properties.limit.default, 8);
  assert.equal(search.properties.excerptLines.minimum, 0);
  assert.equal(search.properties.excerptLines.maximum, 100);
  assert.equal(search.properties.excerptLines.default, 0);
  assert.equal(byName("asgrep_index").parameters.properties.force.default, false);
  assert.equal(byName("asgrep_status").parameters.additionalProperties, false);
  const codemode = byName("asgrep").parameters;
  assert.equal(codemode.additionalProperties, false);
  assert.equal(codemode.properties.code.minLength, 1);
  assert.equal(codemode.properties.code.maxLength, 32000);
});

test("asgrep runs JS against the connector and returns a shaped result", async () => {
  const f = fixture({
    tool: "asgrep",
    schema_version: "1.0.0",
    ok: true,
    hits: [{ file: "src/a.ts", symbol: "auth_refresh", kind: "embed", score: 2 }],
  });
  const { result } = await invoke(f.byName("asgrep"), {
    code: `async () => {
      const seed = await asgrep.search({ query: "auth", limit: 3 });
      return { symbol: seed.hits[0].symbol, n: seed.hits.length };
    }`,
  });
  assert.equal(result.details.ok, true);
  assert.deepEqual(result.details.result, { symbol: "auth_refresh", n: 1 });
  assert.match(result.content[0]!.text, /auth_refresh/);
  assert.ok(f.calls.some((call) => call.args.includes("agent-capsule")));
  assert.ok(result.details.stats);
  assert.ok(typeof result.details.wallMs === "number");
});

test("search result content names the call and lists hits", async () => {
  const f = fixture({
    tool: "asgrep",
    schema_version: "1.0.0",
    ok: true,
    hits: [{ file: "src/auth.rs", start_line: 42, symbol: "refresh_token", kind: "function" }],
  });
  const { result } = await invoke(f.byName("asgrep_search"), { query: "auth refresh", mode: "natural" });
  assert.deepEqual(f.calls[0]?.args, ["--json", "--format", "agent-capsule", "--limit", "8", "--excerpt-lines", "0", "auth refresh", "."]);
  const text = result.content[0]!.text;
  assert.match(text, /asgrep/);
  assert.match(text, /search/);
  assert.match(text, /auth refresh/);
  assert.match(text, /src\/auth\.rs:42/);
  assert.match(text, /refresh_token/);
  assert.equal(typeof result.details.activationMs, "number");
});

test("maps every query mode and bounded output option to argv arrays", async () => {
  const cases: Array<[string, string[]]> = [
    ["natural", ["--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3", "needle", "."]],
    ["pattern", ["--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3", "pattern: needle", "."]],
    ["defs", ["--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3", "defs: needle", "."]],
    ["callers", ["--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3", "callers: needle", "."]],
    ["chain", ["chain", "needle", ".", "--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3"]],
    ["semantic", ["semantic", "needle", ".", "--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3"]],
    ["word", ["--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3", "word: needle", "."]],
    ["literal", ["--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3", "literal: needle", "."]],
    ["regex", ["--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3", "regex: needle", "."]],
    ["imports", ["--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3", "imports: needle", "."]],
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
    tool: "asgrep", schema_version: "1.0.0", ok: true, command: "status", version: "1.4.0",
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
      if (args[0] === "status") return status;
      if (args[0] === "index") {
        return { tool: "asgrep" as const, schema_version: "1.0.0", ok: true, files_failed: 0, walk_errors: false };
      }
      return { tool: "asgrep" as const, schema_version: "1.0.0", ok: true, hits: [] };
    },
  };
  registerAstSgrepTools(pi, runtime);
  const search = tools.find((tool) => tool.name === "asgrep_search")!;
  await invoke(search, { query: "first" });
  assert.deepEqual(calls, ["status", "index", "--json"]);

  handlers[0]!({ toolName: "edit", input: { path: "src/a.ts" }, isError: false }, { cwd: "/project" });
  status = { tool: "asgrep", schema_version: "1.0.0", ok: true };
  const { result } = await invoke(search, { query: "blocked" });
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
  const search = tools.find((tool) => tool.name === "asgrep_search")!;
  const { result } = await invoke(search, { query: "x" });
  assert.equal(result.content[0]!.text, "search failed [CANCELLED]: execution cancelled");
  assert.deepEqual(result.details, {
    ok: false,
    command: "search",
    error: { code: "CANCELLED", message: "execution cancelled", details: { source: "signal" } },
  });
});

test("missing CLI backend surfaces BACKEND_UNAVAILABLE from search", async () => {
  const tools: Tool[] = [];
  const pi = {
    registerTool(tool: Tool) { tools.push(tool); },
    on() {},
  } as unknown as ExtensionAPI;
  const runtime = {
    async resolveRoot(context: { cwd: string }) { return context.cwd; },
    resolveBinaryPath() { throw new RuntimeError("BINARY_RESOLUTION_FAILED", "Unable to resolve an ast-sgrep binary for this platform"); },
    nativeEnv() { return { NO_COLOR: "1" }; },
    async run() { throw new Error("should not reach run"); },
  };
  const freshness = { async ensureFresh() {}, markAffectedPath() {} };
  registerAstSgrepTools(pi, runtime as never, freshness as never);
  const search = tools.find((t) => t.name === "asgrep_search")!;
  const out = await search.execute("c1", { query: "x" }, new AbortController().signal, () => {}, { cwd: "/project" });
  assert.equal(out.details.ok, false);
  assert.equal(out.details.error.code, "BACKEND_UNAVAILABLE");
  assert.equal(out.details.error.details.backend, "unavailable");
  assert.equal(out.details.error.details.napi, false);
  assert.equal(out.details.error.details.cli, false);
  assert.match(String(out.details.error.details.hint), /@ast-sgrep\//);
  assert.match(out.content[0].text, /BACKEND_UNAVAILABLE/);
});

test("missing backend surfaces BACKEND_UNAVAILABLE from asgrep ensureFresh path", async () => {
  const tools: Tool[] = [];
  const pi = {
    registerTool(tool: Tool) { tools.push(tool); },
    on() {},
  } as unknown as ExtensionAPI;
  const runtime = {
    async resolveRoot(context: { cwd: string }) { return context.cwd; },
    resolveBinaryPath() { throw new RuntimeError("BINARY_RESOLUTION_FAILED", "Unable to resolve an ast-sgrep binary for this platform"); },
    nativeEnv() { return { NO_COLOR: "1" }; },
    async run() { throw new Error("should not reach run"); },
  };
  // Default FreshnessCoordinator — ensureFresh → nativeCall → BACKEND_UNAVAILABLE.
  registerAstSgrepTools(pi, runtime as never);
  const codemode = tools.find((t) => t.name === "asgrep")!;
  const out = await codemode.execute("c1", { code: "async () => 1" }, new AbortController().signal, () => {}, { cwd: "/project" });
  assert.equal(out.details.ok, false);
  assert.equal((out.details.error as { code: string }).code, "BACKEND_UNAVAILABLE");
});
