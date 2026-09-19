import assert from "node:assert/strict";
import test from "node:test";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerAstSgrepTools } from "../../../packages/pi/extension/src/index.js";
import { hostProvidesFileTools } from "../../../packages/pi/extension/src/host/tools.js";
import { writesOffSession } from "../../../packages/pi/extension/src/host/tools.js";
import { argvFor } from "../../../packages/pi/extension/src/codemode/index.js";
import { errorDetails } from "../../../packages/pi/extension/src/host/results.js";
import { RESOLVED_ROOT } from "../../../packages/pi/extension/src/runtime/types.js";
import { RuntimeError, type MachineEnvelope } from "../../../packages/pi/extension/src/runtime/runtime.js";

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
  // asgrep_status is not a tool: the model reads status in Code Mode and
  // humans run /asgrep-status, so the schema does not carry it every request.
  assert.deepEqual(tools.map(({ name }) => name), ["asgrep", "asgrep_search", "asgrep_edit", "asgrep_read", "asgrep_index"]);
  assert.ok(byName("asgrep").promptSnippet);
  assert.match(byName("asgrep").promptSnippet!, /without being asked/);
  assert.ok((byName("asgrep").promptGuidelines ?? []).length >= 2);
  const search = byName("asgrep_search").parameters;
  assert.equal(search.additionalProperties, false);
  assert.equal(search.properties.query.maxLength, 4096);
  assert.equal(search.properties.mode.default, "natural");
  assert.deepEqual(search.properties.mode.enum, ["natural", "pattern", "defs", "callers", "chain", "semantic", "word", "literal", "regex", "imports"]);
  assert.equal(search.properties.limit.default, 8);
  assert.equal(search.properties.excerptLines.default, 0);
  assert.equal(byName("asgrep_index").parameters.properties.force.default, false);
  const codemode = byName("asgrep").parameters;
  assert.equal(codemode.additionalProperties, false);
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
  // Lean by design: the call args carry the query; the result is the payload.
  assert.equal(text.split("\n")[0], "search: 1 hit");
  assert.match(text, /src\/auth\.rs:42 refresh_token/);
  assert.equal(typeof result.details.activationMs, "number");
});

test("asgrep_search injects in: into the query and keeps doctor off the tool list", async () => {
  const f = fixture();
  await invoke(f.byName("asgrep_search"), { query: "auth", in: "src", lang: "rs" });
  assert.ok(f.calls.some((call) => call.args.includes("in:src auth")));
  assert.ok(f.calls.some((call) => call.args.includes("--lang") && call.args.includes("rs")));
  assert.equal(f.tools.some((tool) => tool.name === "asgrep_doctor"), false);
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
  // Refresh gates the query (status -> index); a zero-hit answer then spends one
  // index-coverage probe so an empty index is never reported as a plain no-match.
  assert.deepEqual(calls, ["status", "index", "--json", "status"]);

  handlers[0]!({ toolName: "edit", input: { path: "src/a.ts" }, isError: false }, { cwd: "/project" });
  status = { tool: "asgrep", schema_version: "1.0.0", ok: true };
  const { result } = await invoke(search, { query: "blocked" });
  assert.equal((result.details.error as { code: string }).code, "INDEX_STATUS_UNKNOWN");
  assert.deepEqual(calls, ["status", "index", "--json", "status", "status"]);
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

test("asgrep_search freshness timeout still searches", async () => {
  const tools: Tool[] = [];
  const calls: Call[] = [];
  const pi = {
    registerTool(tool: Tool) { tools.push(tool); },
    on() {},
  } as unknown as ExtensionAPI;
  const runtime = {
    async resolveRoot(context: { cwd: string }) { return context.cwd; },
    async run(args: readonly string[], context: { cwd: string }, options: { signal?: AbortSignal }) {
      calls.push({ args, context, options });
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [{ path: "src/train.py" }] };
    },
  };
  const freshness = {
    async ensureFresh() { throw new Error("codemode call timed out after 30000ms"); },
    markAffectedPath() {},
  };
  registerAstSgrepTools(pi, runtime as never, freshness as never);
  const search = tools.find((t) => t.name === "asgrep_search")!;
  const out = await search.execute("c1", { query: "model training" }, new AbortController().signal, () => {}, { cwd: "/project" });
  assert.equal(out.details.ok, true, JSON.stringify(out.details));
  assert.ok(calls.some((call) => call.args.includes("model training") || call.args.some((arg) => String(arg).includes("model"))));
});

test("asgrep_search in:path indexes that directory instead of a full refresh", async () => {
  const tools: Tool[] = [];
  const calls: Call[] = [];
  const pi = {
    registerTool(tool: Tool) { tools.push(tool); },
    on() {},
  } as unknown as ExtensionAPI;
  const runtime = {
    async resolveRoot(context: { cwd: string }) { return context.cwd; },
    async run(args: readonly string[], context: { cwd: string }, options: { signal?: AbortSignal }) {
      calls.push({ args, context, options });
      return {
        tool: "asgrep",
        schema_version: "1.0.0",
        ok: true,
        hits: [],
        stats: { files_indexed: 1, files_skipped: 0, files_removed: 0, files_failed: 0, walk_errors: false },
      };
    },
  };
  const freshness = {
    async ensureFresh() { throw new Error("ensureFresh must not run for in: queries"); },
    markAffectedPath() {},
  };
  registerAstSgrepTools(pi, runtime as never, freshness as never);
  const search = tools.find((t) => t.name === "asgrep_search")!;
  const out = await search.execute("c1", { query: "in:ARCHANA-3/src model training" }, new AbortController().signal, () => {}, { cwd: "/project" });
  assert.equal(out.details.ok, true, JSON.stringify(out.details));
  assert.ok(
    calls.some((call) => call.args.includes("index") && call.args.includes("--path") && call.args.includes("ARCHANA-3/src")),
    `expected targeted index of ARCHANA-3/src, got ${JSON.stringify(calls)}`,
  );
});

test("closed sticky-session errors are SESSION_CLOSED not UNEXPECTED_ERROR", async () => {
  const tools: Tool[] = [];
  const pi = {
    registerTool(tool: Tool) { tools.push(tool); },
    on() {},
  } as unknown as ExtensionAPI;
  const runtime = {
    async resolveRoot(context: { cwd: string }) { return context.cwd; },
    async run() { throw new Error("codemode-serve is closed"); },
  };
  const freshness = {
    async ensureFresh() { throw new Error("codemode-serve is closed"); },
    markAffectedPath() {},
  };
  registerAstSgrepTools(pi, runtime as never, freshness as never);
  const search = tools.find((t) => t.name === "asgrep_search")!;
  const out = await search.execute("c1", { query: "auth" }, new AbortController().signal, () => {}, { cwd: "/project" });
  assert.equal(out.details.ok, false);
  assert.equal((out.details.error as { code: string }).code, "SESSION_CLOSED");
});

test("generic workspace events invalidate paths or roots without naming a producer", () => {
  const listeners = new Map<string, (event: unknown) => void>();
  const changed: unknown[] = [];
  const pi = {
    registerTool() {}, on() {},
    events: { on(name: string, listener: (event: unknown) => void) {
      listeners.set(name, listener);
      return () => listeners.delete(name);
    } },
  } as unknown as ExtensionAPI;
  const runtime = {
    async resolveRoot(context: { cwd: string }) { return context.cwd; },
    async run(): Promise<MachineEnvelope> { throw Error("notification must not run an index"); },
  };
  registerAstSgrepTools(pi, runtime, {
    async ensureFresh() { return "/project"; },
    markAffectedPath(file, cwd) { changed.push([file, cwd]); },
    markRootDirty(root) { changed.push(root); },
  });
  const notify = listeners.get("workspace:changed");
  assert.ok(notify);
  notify({ version: 1, cwd: "/project", paths: ["/project/a.ts"] });
  notify({ version: 1, cwd: "/project", paths: null });
  notify({ version: 2, cwd: "/project", paths: ["/project/no.ts"] });
  notify({ version: 1, cwd: "/project", paths: [null] });
  notify({ version: 1, cwd: "relative", paths: null });
  notify(null);
  assert.deepEqual(changed, [["/project/a.ts", "/project"], "/project"]);
});

test("a freshness timeout serves the search from the current index with a staleness note", async () => {
  const tools: Tool[] = [];
  const pi = {
    registerTool(tool: Tool) { tools.push(tool); },
    on() {},
  } as unknown as ExtensionAPI;
  const runtime = {
    async resolveRoot(context: { cwd: string }) { return context.cwd; },
    async run(): Promise<MachineEnvelope> {
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [{ file: "src/a.ts", start_line: 1, symbol: "auth" }] };
    },
  };
  const freshness = {
    async ensureFresh() {
      throw new RuntimeError("TIMEOUT", "ast-sgrep freshness wait exceeded 10000ms; serving the current index");
    },
    markAffectedPath() {},
  };
  registerAstSgrepTools(pi, runtime as never, freshness as never);
  const search = tools.find((t) => t.name === "asgrep_search")!;
  const out = await search.execute("c1", { query: "auth" }, new AbortController().signal, () => {}, { cwd: "/project" });
  assert.equal(out.details.ok, true, JSON.stringify(out.details));
  assert.equal(out.details.freshness, "stale");
  assert.match(out.content[0]!.text, /refresh/i);
});

test("a zero-hit search on an empty index reports the repository as unindexed", async () => {
  const tools: Tool[] = [];
  const calls: string[] = [];
  const pi = {
    registerTool(tool: Tool) { tools.push(tool); },
    on() {},
  } as unknown as ExtensionAPI;
  const runtime = {
    async resolveRoot(context: { cwd: string }) { return context.cwd; },
    async run(args: readonly string[]): Promise<MachineEnvelope> {
      calls.push(args[0]!);
      if (args[0] === "status") {
        return { tool: "asgrep", schema_version: "1.0.0", ok: true, index_path: "./.asgrep/index.db", file_count: 0, symbol_count: 0, line_count: 0 };
      }
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [] };
    },
  };
  const freshness = {
    async ensureFresh() { return "/project"; },
    markAffectedPath() {},
  };
  registerAstSgrepTools(pi, runtime as never, freshness as never);
  const search = tools.find((t) => t.name === "asgrep_search")!;
  const out = await search.execute("c1", { query: "sample_controlled" }, new AbortController().signal, () => {}, { cwd: "/project" });
  assert.equal(out.details.ok, true);
  assert.equal(out.details.indexState, "empty");
  assert.match(out.content[0]!.text, /not indexed|0 files/i);
  assert.ok(calls.includes("status"), "expected an index status probe, saw " + JSON.stringify(calls));
});

test("tool rows are one self-shelled card with no separate call line", () => {
  const { tools } = fixture();
  type Shellable = {
    name: string;
    renderShell?: string;
    renderCall?(args: Record<string, unknown>, theme: unknown, context: unknown): { render(width: number): string[] },
  };
  for (const tool of tools as unknown as Shellable[]) {
    assert.equal(tool.renderShell, "self", tool.name + " must own its frame (no host Box background band)");
    const call = tool.renderCall?.({ query: "auth", path: "src/a.ts", code: "async () => 1" }, {}, {});
    assert.ok(call, tool.name + " must expose a call slot");
    assert.deepEqual(call.render(80), [], tool.name + " must not paint a call line above the card");
  }
});


/**
 * Every agent-visible failure must arrive with a code that says what to do.
 * The corpus is the real message text each layer produces: the native session
 * (NAPI carries `err.to_string()`, so only text survives), the CLI sticky
 * worker, the runtime envelope parser, and aborts.
 */
test("classifies every failure family the layers can raise", () => {
  const abort = (message: string) => Object.assign(new Error(message), { name: "AbortError" });
  const cases: Array<[string, unknown, string]> = [
    ["runtime error passes its own code", new RuntimeError("INDEX_REBUILD_FAILED", "Incompatible index rebuild failed: x"), "INDEX_REBUILD_FAILED"],
    ["missing backend", new RuntimeError("BACKEND_UNAVAILABLE", "ast-sgrep backend unavailable"), "BACKEND_UNAVAILABLE"],
    ["sticky worker died", new Error("codemode-serve is closed"), "SESSION_CLOSED"],
    ["sticky worker died with cause", new Error("codemode-serve is closed (codemode-serve exited code=1 signal=null stderr=boom)"), "SESSION_CLOSED"],
    ["native session died", new Error("native session is closed"), "SESSION_CLOSED"],
    ["sticky call deadline", new Error("codemode call timed out after 30000ms"), "TIMEOUT"],
    ["batch deadline", new Error("codemode-batch timed out after 30000ms"), "TIMEOUT"],
    ["cli deadline", new Error("ast-sgrep exceeded 30000ms"), "TIMEOUT"],
    ["aborted native call", abort("native call aborted"), "CANCELLED"],
    ["cancelled by native cancel", new Error("operation cancelled"), "CANCELLED"],
    ["write lock held by a concurrent index build", new Error("database is locked"), "OPERATIONAL_ERROR"],
    ["unindexed root", new Error("index is empty for /x: no index file at /x/.asgrep/index.db; run: asgrep index /x --json"), "OPERATIONAL_ERROR"],
    ["stale index schema", new Error("index schema version 12 is older than supported version 16; its rows predate a schema migration"), "OPERATIONAL_ERROR"],
    ["index open failure", new Error("failed to open index at ./.asgrep/index.db (root .): no index file"), "OPERATIONAL_ERROR"],
    ["index changed mid-query", new Error("index changed while preparing lexical sidecar; retry the rebuild"), "OPERATIONAL_ERROR"],
    ["corrupt index file", new Error("database disk image is malformed"), "OPERATIONAL_ERROR"],
    ["unknown failure stays unexpected", new Error("something entirely new"), "UNEXPECTED_ERROR"],
  ];
  for (const [label, cause, code] of cases) {
    assert.equal(errorDetails(cause).code, code, label + " -> " + errorDetails(cause).code);
  }
  // Actionable families carry a hint the model can act on.
  assert.match(String(errorDetails(new Error("database is locked")).details.hint), /retry/i);
  assert.match(String(errorDetails(new Error("database disk image is malformed")).details.hint), /reindex/i);
  // A cancelled signal wins over whatever the underlying failure was.
  const controller = new AbortController();
  controller.abort();
  assert.equal(errorDetails(new Error("database is locked"), controller.signal).code, "CANCELLED");
});


/**
 * One index per checkout: a session whose cwd sits inside an already-indexed
 * checkout must be served by that checkout's index, scoped to the cwd — never
 * a second `.asgrep` growing inside the tree.
 */
test("an indexed checkout anchors subdirectory sessions to its own index", async () => {
  const { mkdtemp, mkdir, writeFile } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const project = await mkdtemp(join(tmpdir(), "asgrep-anchor-"));
  const subdir = join(project, "packages", "coding-agent", "src");
  await mkdir(subdir, { recursive: true });
  await mkdir(join(project, ".asgrep"), { recursive: true });
  await writeFile(join(project, ".asgrep", "index.db"), "");

  const tools: Tool[] = [];
  const pi = { registerTool(tool: Tool) { tools.push(tool); }, on() {} } as unknown as ExtensionAPI;
  const calls: Array<{ args: readonly string[]; cwd: string }> = [];
  const indexed: string[] = [];
  const runtime = {
    async resolveRoot(context: { cwd: string }) { return context.cwd; },
    resolveIndexPath(root: string) { return join(root, ".asgrep", "index.db"); },
    nativeEnv() { return {} as NodeJS.ProcessEnv; },
    config: { timeoutMs: 1_000, maxOutputBytes: 1_000_000, refreshIntervalMs: 30_000 },
    async run(args: readonly string[], context: { cwd: string }) {
      calls.push({ args: [...args], cwd: context.cwd });
      return { tool: "asgrep" as const, schema_version: "1.0.0", ok: true, hits: [] };
    },
  };
  const freshness = {
    async ensureFresh(_runtime: unknown, context: { cwd: string }) {
      indexed.push(context.cwd);
      return context.cwd;
    },
    markAffectedPath() {},
  };
  registerAstSgrepTools(pi, runtime as never, freshness as never);

  const search = tools.find((tool) => tool.name === "asgrep_search")!;
  await search.execute("c1", { query: "auth" }, new AbortController().signal, () => {}, { cwd: subdir });
  assert.deepEqual(indexed, [project], "the checkout's index serves the subdirectory");
  const searchCall = calls.find((call) => call.args.includes("agent-capsule"));
  assert.ok(searchCall, "expected a search argv, saw " + JSON.stringify(calls));
  assert.ok(
    searchCall.args.includes("in:packages/coding-agent/src auth"),
    "search scope follows the anchor: " + JSON.stringify(searchCall.args),
  );
  assert.equal(searchCall.cwd, project, "searches run against the checkout root");
});


/**
 * The live failure this pins: with a configured project root, re-resolving the
 * anchored root applied the config again and dropped the call back into the
 * un-indexed subdirectory (the CLI then refused to build a second index).
 * Anchored calls must arrive already-resolved.
 */
test("an anchored root is not re-resolved into the configured subdirectory", async () => {
  const { mkdtemp, mkdir, writeFile } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");

  const checkout = await mkdtemp(join(tmpdir(), "asgrep-configured-"));
  const configured = join(checkout, "packages", "coding-agent", "src");
  await mkdir(configured, { recursive: true });
  await mkdir(join(checkout, ".asgrep"), { recursive: true });
  await writeFile(join(checkout, ".asgrep", "index.db"), "");

  const tools: Tool[] = [];
  const pi = { registerTool(tool: Tool) { tools.push(tool); }, on() {} } as unknown as ExtensionAPI;
  const contexts: Array<{ cwd: string; resolved: boolean }> = [];
  const runtime = {
    // AstSgrepRuntime: a context marked RESOLVED_ROOT is taken as final,
    // anything else gets the configured subdirectory appended.
    async resolveRoot(context: { cwd: string; [RESOLVED_ROOT]?: true }) {
      return context[RESOLVED_ROOT] === true ? context.cwd : configured;
    },
    resolveIndexPath(root: string) { return join(root, ".asgrep", "index.db"); },
    nativeEnv() { return {} as NodeJS.ProcessEnv; },
    config: { timeoutMs: 1_000, maxOutputBytes: 1_000_000, refreshIntervalMs: 30_000 },
    async run(args: readonly string[], context: { cwd: string; [RESOLVED_ROOT]?: true }) {
      contexts.push({ cwd: context.cwd, resolved: context[RESOLVED_ROOT] === true });
      return { tool: "asgrep" as const, schema_version: "1.0.0", ok: true, hits: [] };
    },
  };
  const indexed: string[] = [];
  const freshness = {
    async ensureFresh(_runtime: unknown, context: { cwd: string; [RESOLVED_ROOT]?: true }) {
      indexed.push(context[RESOLVED_ROOT] === true ? context.cwd : "UNRESOLVED:" + context.cwd);
      return context.cwd;
    },
    markAffectedPath() {},
  };
  registerAstSgrepTools(pi, runtime as never, freshness as never);

  const search = tools.find((tool) => tool.name === "asgrep_search")!;
  await search.execute("c1", { query: "auth" }, new AbortController().signal, () => {}, { cwd: checkout });
  assert.deepEqual(indexed, [checkout], "the anchor must reach the refresh already resolved");
  const call = contexts.find((entry) => entry.resolved);
  assert.ok(call, "follow-up calls must carry the resolved marker: " + JSON.stringify(contexts));
  assert.equal(call.cwd, checkout);
});


/**
 * A checkout with no index yet puts it at the work tree root, and an index that
 * merely lives above the work tree is not this project's.
 */
test("an unindexed checkout anchors at its work tree root, not the session subdirectory", async () => {
  const { mkdtemp, mkdir, writeFile } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const outer = await mkdtemp(join(tmpdir(), "asgrep-worktree-"));
  await mkdir(join(outer, ".asgrep"), { recursive: true });
  await writeFile(join(outer, ".asgrep", "index.db"), ""); // unrelated parent index
  const repo = join(outer, "fresh-repo");
  const subdir = join(repo, "packages", "x", "src");
  await mkdir(join(repo, ".git"), { recursive: true });
  await mkdir(subdir, { recursive: true });

  const tools: Tool[] = [];
  const pi = { registerTool(tool: Tool) { tools.push(tool); }, on() {} } as unknown as ExtensionAPI;
  const indexed: string[] = [];
  const runtime = {
    async resolveRoot(context: { cwd: string }) { return context.cwd; },
    resolveIndexPath(root: string) { return join(root, ".asgrep", "index.db"); },
    nativeEnv() { return {} as NodeJS.ProcessEnv; },
    config: { timeoutMs: 1_000, maxOutputBytes: 1_000_000, refreshIntervalMs: 30_000 },
    async run() { return { tool: "asgrep" as const, schema_version: "1.0.0", ok: true, hits: [] }; },
  };
  const freshness = {
    async ensureFresh(_runtime: unknown, context: { cwd: string }) { indexed.push(context.cwd); return context.cwd; },
    markAffectedPath() {},
  };
  registerAstSgrepTools(pi, runtime as never, freshness as never);

  const search = tools.find((tool) => tool.name === "asgrep_search")!;
  await search.execute("c1", { query: "auth" }, new AbortController().signal, () => {}, { cwd: subdir });
  assert.deepEqual(indexed, [repo], "the index belongs at the work tree root");
});


/**
 * Package-only hosts: pi ships read/edit built in, so paying for schema tokens
 * on our duplicates is waste. They stay registered (MCP-style hosts, sessions
 * started with --no-builtin-tools, and hosts without the tool-set API still get
 * them) but leave the active set when the host already provides read+edit.
 */
const fakeHost = (activeTools: string[]): boolean =>
  hostProvidesFileTools({ getActiveTools: () => activeTools } as unknown as ExtensionAPI, {} as NodeJS.ProcessEnv);

test("one-shot file tools stay registered but leave the active set on builtin hosts", () => {
  const tools: Tool[] = [];
  const handlers = new Map<string, (event: unknown, ctx: { cwd: string }) => void>();
  let active = ["read", "edit", "bash"];
  const pi = {
    registerTool(tool: Tool) { tools.push(tool); active.push(tool.name); },
    on(event: string, handler: (event: unknown, ctx: { cwd: string }) => void) { handlers.set(event, handler); },
    getActiveTools: () => [...active],
    setActiveTools: (names: string[]) => { active = [...names]; },
  } as unknown as ExtensionAPI;
  const runtime = {
    async resolveRoot(context: { cwd: string }) { return context.cwd; },
    async run() { return { tool: "asgrep" as const, schema_version: "1.0.0", ok: true }; },
  };
  registerAstSgrepTools(pi, runtime as never, { async ensureFresh() { return "/p"; }, markAffectedPath() {} } as never);

  // Registered for every host, exactly as before.
  assert.deepEqual(
    tools.map((tool) => tool.name),
    ["asgrep", "asgrep_search", "asgrep_edit", "asgrep_read", "asgrep_index"],
    "the capability surface must not shrink",
  );

  handlers.get("session_start")!({}, { cwd: "/p" });
  assert.ok(!active.includes("asgrep_read"), "duplicate file tools must not ride in the schema");
  assert.ok(!active.includes("asgrep_edit"));
  for (const kept of ["asgrep", "asgrep_search", "asgrep_index"]) assert.ok(active.includes(kept), kept + " must stay active");

  // Any other active reader/editor counts (built-in, wrapped, or extension).
  assert.equal(fakeHost(["read", "edit"]), true);
  assert.equal(fakeHost(["read"]), false, "half a pair is not coverage");
  assert.equal(fakeHost([]), false, "no host reader: ours must stay active");
  // Escape hatch, and the safe default for hosts that cannot manage tool sets.
  assert.equal(hostProvidesFileTools(pi, { ASGREP_KEEP_FILE_TOOLS: "1" } as NodeJS.ProcessEnv), false);
  assert.equal(hostProvidesFileTools({} as ExtensionAPI, {} as NodeJS.ProcessEnv), false);
  assert.equal(hostProvidesFileTools(pi, {} as NodeJS.ProcessEnv), true);
});


/**
 * p100 contract: the warm session serializes its calls, so an index running
 * there blocks every read queued behind it (measured: a search waited 9.2s for
 * a background reindex). Mutating tools must route out of process, and the CLI
 * argv must carry the embeddings policy the caller chose.
 */
test("index work never rides the warm session, and its argv carries the embed policy", () => {
  assert.equal(writesOffSession("index_repo"), true, "writes must go out of process");
  for (const read of ["search", "find", "read", "edit", "defs", "callers", "semantic", "index_status"]) {
    assert.equal(writesOffSession(read), false, read + " is a read: keep the warm session");
  }
  // Implicit (freshness) refreshes are lexical/AST only; explicit index keeps vectors.
  assert.deepEqual(argvFor("index_repo", { force: false, use_embed: false }), ["index", ".", "--json", "--no-embed"]);
  assert.deepEqual(argvFor("index_repo", { force: true }), ["reindex", ".", "--json"]);
  assert.deepEqual(
    argvFor("index_repo", { paths: ["src/a.ts"], use_embed: false }),
    ["index", ".", "--json", "--no-embed", "--path", "src/a.ts"],
  );
});

