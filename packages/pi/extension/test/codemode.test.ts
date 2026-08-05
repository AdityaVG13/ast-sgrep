import assert from "node:assert/strict";
import test from "node:test";
import { createAsgrepConnector } from "../src/codemode/connector.js";
import { createCodemodeDispatcher, argvFor, asEnvelope } from "../src/codemode/dispatch.js";
import { normalizeCode, runCodemode } from "../src/codemode/sandbox.js";
import type { MachineEnvelope } from "../src/runtime.js";

test("normalizeCode wraps bare bodies and strips fences", () => {
  assert.match(normalizeCode("return 1"), /async \(\) =>/);
  assert.match(normalizeCode("```js\nreturn 2\n```"), /return 2/);
  assert.match(normalizeCode("async () => 3"), /^\(async \(\) => 3\)\(\)$/);
});

test("Promise.all overlaps host calls (Amdahl parallel fraction)", async () => {
  const starts: number[] = [];
  const host = {
    async run(args: readonly string[]): Promise<MachineEnvelope> {
      starts.push(Date.now());
      await new Promise((r) => setTimeout(r, 60));
      return {
        tool: "asgrep",
        schema_version: "1.0.0",
        ok: true,
        hits: [{ file: "src/a.ts", symbol: "S", kind: "embed", score: 1 }],
        argv0: args[0],
      };
    },
  };
  const bundle = createAsgrepConnector(host, { cwd: "/project" });
  const wall0 = Date.now();
  const outcome = await runCodemode(
    `async () => {
      const [a, b, c] = await Promise.all([
        asgrep.search({ query: "one" }),
        asgrep.defs({ symbol: "Foo" }),
        asgrep.callers({ symbol: "Foo" }),
      ]);
      return { n: [a, b, c].filter((x) => x.ok).length };
    }`,
    bundle.asgrep,
    { stats: bundle.stats },
  );
  const wall = Date.now() - wall0;
  assert.equal(outcome.ok, true, outcome.error);
  assert.deepEqual(outcome.result, { n: 3 });
  assert.ok(wall < 140, `expected overlapped wall < 140ms, got ${wall}ms`);
  assert.equal(starts.length, 3);
  assert.ok(Math.max(...starts) - Math.min(...starts) < 25, "calls should start in the same wave");
  assert.ok(bundle.stats().calls >= 3);
  assert.ok(bundle.stats().waves >= 1);
});

test("dispatcher coalesces same-tick calls into one batch wave", async () => {
  const runCalls: string[][] = [];
  let batchCalls = 0;
  const host = {
    async run(args: readonly string[]): Promise<MachineEnvelope> {
      runCalls.push([...args]);
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [] };
    },
    async runBatch(calls: Array<{ id: string; tool: string; args: Record<string, unknown> }>) {
      batchCalls += 1;
      return {
        results: calls.map((c) => ({
          id: c.id,
          ok: true,
          value: { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [{ symbol: c.tool }], batched: true },
        })),
        mode: "serial",
      };
    },
  };
  const bundle = createAsgrepConnector(host, { cwd: "/project" });
  const outcome = await runCodemode(
    `async () => {
      const [a, b] = await Promise.all([
        asgrep.search({ query: "auth" }),
        asgrep.defs({ symbol: "Auth" }),
      ]);
      return { a: a.hits[0].symbol, b: b.hits[0].symbol, batched: a.batched && b.batched };
    }`,
    bundle.asgrep,
    { stats: bundle.stats },
  );
  assert.equal(outcome.ok, true, outcome.error);
  assert.deepEqual(outcome.result, { a: "search", b: "defs", batched: true });
  assert.equal(batchCalls, 1);
  assert.equal(runCalls.length, 0);
  assert.equal(bundle.stats().batchedCalls, 2);
});

test("partial batch failure does not re-run successful siblings via spawn", async () => {
  const runCalls: string[][] = [];
  const host = {
    async run(args: readonly string[]): Promise<MachineEnvelope> {
      runCalls.push([...args]);
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [] };
    },
    async runBatch(calls: Array<{ id: string; tool: string; args: Record<string, unknown> }>) {
      return {
        all_ok: false,
        results: calls.map((c, i) =>
          i === 0
            ? { id: c.id, ok: true, value: { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [{ symbol: "ok" }] } }
            : { id: c.id, ok: false, error: "symbol is required" },
        ),
      };
    },
  };
  const bundle = createAsgrepConnector(host, { cwd: "/p" });
  const outcome = await runCodemode(
    `async () => {
      try {
        await Promise.all([
          asgrep.search({ query: "a" }),
          asgrep.defs({ symbol: "" }),
        ]);
        return "should-not";
      } catch (e) {
        return String(e.message || e);
      }
    }`,
    bundle.asgrep,
    { stats: bundle.stats },
  );
  assert.equal(outcome.ok, true, outcome.error);
  assert.match(String(outcome.result), /symbol|failed/i);
  assert.equal(runCalls.length, 0, "must not fall back to spawn on per-call failure");
  assert.equal(bundle.stats().batchedCalls, 2);
  assert.equal(bundle.stats().parallelSpawnCalls, 0);
});

test("sticky worker handles multi-wave program without batch/spawn", async () => {
  const stickyCalls: string[] = [];
  const host = {
    async run(): Promise<MachineEnvelope> {
      throw new Error("run should not be used");
    },
    sticky: {
      async call(tool: string) {
        stickyCalls.push(tool);
        return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [{ symbol: tool }] };
      },
      async batch(calls: Array<{ id: string; tool: string }>) {
        for (const c of calls) stickyCalls.push(c.tool);
        return {
          results: calls.map((c) => ({
            id: c.id,
            ok: true,
            value: { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [{ symbol: c.tool }] },
          })),
        };
      },
      async end() {},
    },
  };
  const bundle = createAsgrepConnector(host, { cwd: "/p" });
  const outcome = await runCodemode(
    `async () => {
      const [a, b] = await Promise.all([
        asgrep.search({ query: "one" }),
        asgrep.defs({ symbol: "Foo" }),
      ]);
      const c = await asgrep.chain({ query: "Foo" });
      return { a: a.hits[0].symbol, b: b.hits[0].symbol, c: c.hits[0].symbol };
    }`,
    bundle.asgrep,
    { stats: bundle.stats },
  );
  assert.equal(outcome.ok, true, outcome.error);
  assert.deepEqual(outcome.result, { a: "search", b: "defs", c: "chain" });
  assert.ok(bundle.stats().stickyCalls >= 3);
  assert.equal(bundle.stats().parallelSpawnCalls, 0);
  assert.deepEqual(stickyCalls.sort(), ["chain", "defs", "search"]);
});

test("dispatcher falls back to parallel spawn when batch fails", async () => {
  const runCalls: number[] = [];
  const host = {
    async run(): Promise<MachineEnvelope> {
      runCalls.push(Date.now());
      await new Promise((r) => setTimeout(r, 40));
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [{ symbol: "x" }] };
    },
    async runBatch() {
      throw new Error("codemode-batch not available");
    },
  };
  const bundle = createAsgrepConnector(host, { cwd: "/p" });
  const wall0 = Date.now();
  const outcome = await runCodemode(
    `async () => {
      const [a, b] = await Promise.all([asgrep.search({ query: "a" }), asgrep.search({ query: "b" })]);
      return a.ok && b.ok;
    }`,
    bundle.asgrep,
    { stats: bundle.stats },
  );
  const wall = Date.now() - wall0;
  assert.equal(outcome.ok, true, outcome.error);
  assert.equal(outcome.result, true);
  assert.equal(runCalls.length, 2);
  assert.ok(wall < 90, `fallback parallel spawn should overlap, wall=${wall}`);
  assert.equal(bundle.stats().parallelSpawnCalls, 2);
});

test("sandbox blocks require and process", async () => {
  const bundle = createAsgrepConnector({
    async run(): Promise<MachineEnvelope> {
      return { tool: "asgrep", schema_version: "1.0.0", ok: true };
    },
  }, { cwd: "/project" });
  const requireAttempt = await runCodemode(`return typeof require`, bundle.asgrep);
  assert.equal(requireAttempt.ok, true);
  assert.equal(requireAttempt.result, "undefined");
  const processAttempt = await runCodemode(`return typeof process`, bundle.asgrep);
  assert.equal(processAttempt.ok, true);
  assert.equal(processAttempt.result, "undefined");
});

test("sandbox blocks constructor escapes through globals, APIs, and returned values", async () => {
  const bundle = createAsgrepConnector({
    async run(): Promise<MachineEnvelope> {
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [] };
    },
  }, { cwd: "/project" });
  for (const code of [
    `return Object.constructor("return process")().pid`,
    `return asgrep.search.constructor("return process")().pid`,
    `const result = await asgrep.search({ query: "x" }); return result.constructor.constructor("return process")().pid`,
  ]) {
    const outcome = await runCodemode(code, bundle.asgrep);
    assert.equal(outcome.ok, false, code);
    assert.match(outcome.error ?? "", /code generation from strings disallowed/iu);
  }
});

test("sandbox interrupts synchronous infinite loops", async () => {
  const bundle = createAsgrepConnector({
    async run(): Promise<MachineEnvelope> {
      return { tool: "asgrep", schema_version: "1.0.0", ok: true };
    },
  }, { cwd: "/project" });
  const started = Date.now();
  const outcome = await runCodemode(`while (true) {}`, bundle.asgrep, { timeoutMs: 25 });
  assert.equal(outcome.ok, false);
  assert.match(outcome.error ?? "", /timed out/iu);
  assert.ok(Date.now() - started < 1_000, "infinite loop must be interrupted promptly");
});

test("sandbox observes cancellation while awaiting asynchronous code", async () => {
  const bundle = createAsgrepConnector({
    async run(): Promise<MachineEnvelope> {
      return { tool: "asgrep", schema_version: "1.0.0", ok: true };
    },
  }, { cwd: "/project" });
  const controller = new AbortController();
  const pending = runCodemode(`await new Promise(() => {})`, bundle.asgrep, {
    signal: controller.signal,
    timeoutMs: 5_000,
  });
  setTimeout(() => controller.abort(), 10);
  const outcome = await pending;
  assert.equal(outcome.ok, false);
  assert.match(outcome.error ?? "", /aborted/iu);
});

test("typed connector preserves defs vs search(query containing defs:)", async () => {
  const tools: string[] = [];
  const host = {
    async run(): Promise<MachineEnvelope> {
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [] };
    },
    async runBatch(calls: Array<{ id: string; tool: string; args: Record<string, unknown> }>) {
      for (const c of calls) tools.push(c.tool);
      return {
        results: calls.map((c) => ({
          id: c.id,
          ok: true,
          value: { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [], got: c.tool, args: c.args },
        })),
      };
    },
  };
  const bundle = createAsgrepConnector(host, { cwd: "/project" });
  await Promise.all([
    bundle.asgrep.search({ query: "defs: auth in login flow", limit: 4 }),
    bundle.asgrep.defs({ symbol: "Auth", limit: 4, excerptLines: 2 }),
  ]);
  assert.deepEqual(tools.sort(), ["defs", "search"]);
});

test("argvFor emits typed-equivalent CLI for spawn fallback", () => {
  assert.deepEqual(argvFor("defs", { symbol: "Foo", limit: 4, excerpt_lines: 2 }), [
    "--json", "--format", "agent-capsule", "--limit", "4", "--excerpt-lines", "2", "defs:Foo", ".",
  ]);
  assert.deepEqual(argvFor("chain", { query: "Foo", limit: 4 }), [
    "chain", "Foo", ".", "--json", "--limit", "4",
  ]);
});

test("asEnvelope does not let payload clobber ok/tool", () => {
  const env = asEnvelope({ tool: "evil", ok: false, schema_version: "9", hits: [1] });
  assert.equal(env.tool, "asgrep");
  assert.equal(env.ok, true);
  assert.equal(env.schema_version, "1.0.0");
  assert.deepEqual(env.hits, [1]);
});

test("createCodemodeDispatcher exposes wave stats", async () => {
  const { host, stats, resetStats } = createCodemodeDispatcher({
    async run(): Promise<MachineEnvelope> {
      return { tool: "asgrep", schema_version: "1.0.0", ok: true };
    },
  });
  resetStats();
  await Promise.all([
    host.run(["--json", "a", "."], { cwd: "/p" }),
    host.run(["--json", "b", "."], { cwd: "/p" }),
  ]);
  assert.equal(stats().waves, 1);
  assert.equal(stats().calls, 2);
  assert.equal(stats().parallelSpawnCalls, 2);
});
