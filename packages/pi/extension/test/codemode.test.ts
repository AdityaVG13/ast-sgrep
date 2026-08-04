import assert from "node:assert/strict";
import test from "node:test";
import { createAsgrepConnector } from "../src/codemode/connector.js";
import { createCodemodeDispatcher } from "../src/codemode/dispatch.js";
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
        mode: "parallel",
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

test("connector maps defs/callers/chain/semantic argv", async () => {
  const calls: string[][] = [];
  const bundle = createAsgrepConnector({
    async run(args: readonly string[]): Promise<MachineEnvelope> {
      calls.push([...args]);
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [] };
    },
  }, { cwd: "/project" });
  await bundle.asgrep.defs({ symbol: "Foo", limit: 4 });
  await bundle.asgrep.callers({ symbol: "Foo", limit: 4 });
  await bundle.asgrep.chain({ query: "Foo", limit: 4 });
  await bundle.asgrep.semantic({ query: "credential renewal", limit: 4 });
  // Each call is its own wave when awaited sequentially.
  assert.deepEqual(calls[0], ["--json", "--format", "agent-capsule", "--limit", "4", "--excerpt-lines", "0", "defs: Foo", "."]);
  assert.deepEqual(calls[1], ["--json", "--format", "agent-capsule", "--limit", "4", "--excerpt-lines", "0", "callers: Foo", "."]);
  assert.deepEqual(calls[2], ["chain", "Foo", ".", "--json", "--format", "agent-capsule", "--limit", "4", "--excerpt-lines", "0"]);
  assert.deepEqual(calls[3], ["semantic", "credential renewal", ".", "--json", "--format", "agent-capsule", "--limit", "4", "--excerpt-lines", "0"]);
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
