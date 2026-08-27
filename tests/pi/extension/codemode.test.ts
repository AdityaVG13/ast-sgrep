import assert from "node:assert/strict";
import { getEventListeners } from "node:events";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { createAsgrepConnector } from "../../../packages/pi/extension/src/codemode/connector.js";
import { createCodemodeDispatcher, argvFor, asEnvelope } from "../../../packages/pi/extension/src/codemode/dispatch.js";
import { normalizeCode, resetCodemodeSandboxForTests, runCodemode, warmCodemodeSandbox } from "../../../packages/pi/extension/src/codemode/runner.js";
import { runBatchViaStdin, startStickyWorker } from "../../../packages/pi/extension/src/codemode/worker.js";
import type { MachineEnvelope } from "../../../packages/pi/extension/src/runtime.js";

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
  assert.equal(outcome.ok, true, outcome.ok ? undefined : outcome.error);
  assert.deepEqual(outcome.result, { n: 3 });
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
  assert.equal(outcome.ok, true, outcome.ok ? undefined : outcome.error);
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
  assert.equal(outcome.ok, true, outcome.ok ? undefined : outcome.error);
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
  assert.equal(outcome.ok, true, outcome.ok ? undefined : outcome.error);
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
  const outcome = await runCodemode(
    `async () => {
      const [a, b] = await Promise.all([asgrep.search({ query: "a" }), asgrep.search({ query: "b" })]);
      return a.ok && b.ok;
    }`,
    bundle.asgrep,
    { stats: bundle.stats },
  );
  assert.equal(outcome.ok, true, outcome.ok ? undefined : outcome.error);
  assert.equal(outcome.result, true);
  assert.equal(runCalls.length, 2);
  assert.ok(Math.max(...runCalls) - Math.min(...runCalls) < 25, "fallback calls should overlap");
  assert.equal(bundle.stats().parallelSpawnCalls, 2);
});

test("dispatcher does not retry an aborted sticky batch", async () => {
  const controller = new AbortController();
  let spawnCalls = 0;
  const sticky = {
    async call() { throw new Error("not used"); },
    async batch(_calls: unknown, options?: { signal?: AbortSignal }) {
      assert.equal(options?.signal, controller.signal);
      controller.abort();
      throw Object.assign(new Error("aborted"), { name: "AbortError" });
    },
    async end() {},
  };
  const dispatcher = createCodemodeDispatcher({
    sticky,
    async run() {
      spawnCalls += 1;
      return asEnvelope({ hits: [] });
    },
  });
  const options = { signal: controller.signal };
  const calls = [
    dispatcher.host.call("search", { query: "a" }, { cwd: "/p" }, options),
    dispatcher.host.call("search", { query: "b" }, { cwd: "/p" }, options),
  ];
  const results = await Promise.allSettled(calls);
  assert.deepEqual(results.map(({ status }) => status), ["rejected", "rejected"]);
  assert.equal(spawnCalls, 0);
});

test("dispatcher rejects pre-aborted calls without starting a backend", async () => {
  const controller = new AbortController();
  controller.abort();
  let backendCalls = 0;
  const dispatcher = createCodemodeDispatcher({
    async run() {
      backendCalls += 1;
      return asEnvelope({ hits: [] });
    },
  });

  await assert.rejects(
    dispatcher.host.call("search", { query: "cancelled" }, { cwd: "/p" }, { signal: controller.signal }),
    { name: "AbortError" },
  );
  await new Promise<void>((resolve) => queueMicrotask(resolve));
  assert.equal(backendCalls, 0);
  assert.equal(getEventListeners(controller.signal, "abort").length, 0);
});

test("dispatcher cancels one batched call without cancelling its siblings", async () => {
  const firstController = new AbortController();
  const secondController = new AbortController();
  const started = Promise.withResolvers<void>();
  const response = Promise.withResolvers<{
    results: Array<{ id: string; ok: boolean; value: MachineEnvelope }>;
  }>();
  const dispatcher = createCodemodeDispatcher({
    async run() { throw new Error("spawn fallback should not run"); },
    async runBatch(calls, _context, options) {
      assert.equal(options, undefined, "distinct call signals must not own batch transport cancellation");
      started.resolve();
      return response.promise.then(() => ({
        results: calls.map(({ id, tool }) => ({ id, ok: true, value: asEnvelope({ hits: [tool] }) })),
      }));
    },
  });

  const first = dispatcher.host.call(
    "search",
    { query: "first" },
    { cwd: "/p" },
    { signal: firstController.signal },
  );
  const second = dispatcher.host.call(
    "defs",
    { symbol: "Second" },
    { cwd: "/p" },
    { signal: secondController.signal },
  );
  await started.promise;
  secondController.abort();
  await assert.rejects(second, { name: "AbortError" });
  response.resolve({ results: [] });
  assert.equal((await first).ok, true);
  assert.equal(getEventListeners(firstController.signal, "abort").length, 0);
  assert.equal(getEventListeners(secondController.signal, "abort").length, 0);
});

test("dispatcher removes per-call abort listeners after a successful batch", async () => {
  const controllers = [new AbortController(), new AbortController()];
  const dispatcher = createCodemodeDispatcher({
    async run() { throw new Error("spawn fallback should not run"); },
    async runBatch(calls) {
      return {
        results: calls.map(({ id, tool }) => ({ id, ok: true, value: asEnvelope({ hits: [tool] }) })),
      };
    },
  });

  await Promise.all(controllers.map((controller, index) => dispatcher.host.call(
    "search",
    { query: String(index) },
    { cwd: "/p" },
    { signal: controller.signal },
  )));
  for (const controller of controllers) {
    assert.equal(getEventListeners(controller.signal, "abort").length, 0);
  }
});

test("one-shot batch transport kills output that exceeds its configured cap", async () => {
  const dir = await mkdtemp(join(tmpdir(), "asgrep-batch-output-"));
  try {
    await writeFile(
      join(dir, "codemode-batch"),
      "process.stdout.write('x'.repeat(8192));\n",
      "utf8",
    );
    await assert.rejects(
      runBatchViaStdin({
        binary: process.execPath,
        cwd: dir,
        body: "{}",
        maxOutputBytes: 1024,
      }),
      /output exceeded 1024 bytes/u,
    );
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("sticky transport kills an oversized NDJSON response", {
  skip: process.platform === "win32" ? "executable script fixture is POSIX-only" : false,
}, async () => {
  const dir = await mkdtemp(join(tmpdir(), "asgrep-sticky-output-"));
  let worker: Awaited<ReturnType<typeof startStickyWorker>> | undefined;
  try {
    const binary = join(dir, "fake-asgrep");
    await writeFile(
      binary,
      `#!/usr/bin/env node
process.stdin.once("data", () => process.stdout.write("x".repeat(8192) + "\\n"));
setInterval(() => {}, 1000);
`,
      { encoding: "utf8", mode: 0o755 },
    );
    worker = await startStickyWorker({
      binary,
      cwd: dir,
      maxOutputBytes: 1024,
    });
    await assert.rejects(
      worker.call("search", { query: "x" }),
      /output exceeded 1024 bytes/u,
    );
  } finally {
    await worker?.end();
    await rm(dir, { recursive: true, force: true });
  }
});

test("ending a sticky transport rejects pending calls", {
  skip: process.platform === "win32" ? "executable script fixture is POSIX-only" : false,
}, async () => {
  const dir = await mkdtemp(join(tmpdir(), "asgrep-sticky-end-"));
  let worker: Awaited<ReturnType<typeof startStickyWorker>> | undefined;
  try {
    const binary = join(dir, "fake-asgrep");
    await writeFile(
      binary,
      `#!/usr/bin/env node
process.stdin.resume();
setInterval(() => {}, 1000);
`,
      { encoding: "utf8", mode: 0o755 },
    );
    worker = await startStickyWorker({ binary, cwd: dir });
    const rejected = assert.rejects(
      worker.call("search", { query: "x" }),
      /codemode-serve ended/u,
    );
    await worker.end();
    await rejected;
  } finally {
    await worker?.end();
    await rm(dir, { recursive: true, force: true });
  }
});

test("sticky stdin write failure terminates the transport", {
  skip: process.platform === "win32" ? "executable script fixture is POSIX-only" : false,
}, async () => {
  const dir = await mkdtemp(join(tmpdir(), "asgrep-sticky-stdin-"));
  let worker: Awaited<ReturnType<typeof startStickyWorker>> | undefined;
  try {
    const binary = join(dir, "fake-asgrep");
    await writeFile(
      binary,
      `#!/usr/bin/env node
require("node:fs").closeSync(0);
setInterval(() => {}, 1000);
`,
      { encoding: "utf8", mode: 0o755 },
    );
    worker = await startStickyWorker({ binary, cwd: dir, timeoutMs: 1_000 });
    await new Promise((resolve) => setTimeout(resolve, 100));
    const started = Date.now();
    await assert.rejects(worker.call("search", { query: "x" }));
    assert.ok(Date.now() - started < 500, "write failure must reject before the request timeout");
    await assert.rejects(worker.call("search", { query: "y" }), /closed/u);
  } finally {
    await worker?.end();
    await rm(dir, { recursive: true, force: true });
  }
});

test("dispatcher never replays a mutation after an ambiguous native failure", async () => {
  let batchFallbacks = 0;
  let spawnFallbacks = 0;
  const transportFailure = new Error("native transport closed after dispatch");
  const dispatcher = createCodemodeDispatcher({
    sticky: {
      async call() { throw new Error("not used"); },
      async batch() { throw transportFailure; },
      async end() {},
    },
    async runBatch() {
      batchFallbacks += 1;
      return { results: [] };
    },
    async run() {
      spawnFallbacks += 1;
      return asEnvelope({ hits: [] });
    },
  });

  const results = await Promise.allSettled([
    dispatcher.host.call("index_repo", { force: false }, { cwd: "/p" }),
    dispatcher.host.call("search", { query: "auth" }, { cwd: "/p" }),
  ]);
  assert.deepEqual(results.map(({ status }) => status), ["rejected", "rejected"]);
  assert.ok(results.every((result) => result.status === "rejected" && result.reason === transportFailure));
  assert.equal(batchFallbacks, 0);
  assert.equal(spawnFallbacks, 0);
});

test("runner binds asgrep and console through the isolated bridge", async () => {
  const bundle = createAsgrepConnector({
    async run(): Promise<MachineEnvelope> {
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [{ path: "a.ts" }] };
    },
  }, { cwd: "/project" });
  const outcome = await runCodemode(
    `console.log("hi"); const r = await asgrep.search({ query: "x" }); return r.hits?.length ?? 0;`,
    bundle.asgrep,
  );
  assert.equal(outcome.ok, true, outcome.ok ? undefined : outcome.error);
  assert.equal(outcome.result, 1);
  assert.deepEqual(outcome.logs, ["hi"]);
});

test("runner does not expose ambient Node authority", async () => {
  const bundle = createAsgrepConnector({
    async run(): Promise<MachineEnvelope> {
      return { tool: "asgrep", schema_version: "1.0.0", ok: true };
    },
  }, { cwd: "/project" });
  for (const code of [
    "return typeof process",
    "return typeof require",
    "return typeof ArrayBuffer",
    "return typeof WebAssembly",
    "return globalThis.constructor.constructor('return process')()",
  ]) {
    const outcome = await runCodemode(code, bundle.asgrep);
    if (code.includes("constructor")) {
      assert.equal(outcome.ok, false, `constructor escape unexpectedly succeeded: ${JSON.stringify(outcome)}`);
    } else {
      assert.equal(outcome.ok, true, outcome.ok ? undefined : outcome.error);
      assert.equal(outcome.result, "undefined");
    }
  }
});

test("runner interrupts synchronous infinite loops", async () => {
  const bundle = createAsgrepConnector({
    async run(): Promise<MachineEnvelope> {
      return { tool: "asgrep", schema_version: "1.0.0", ok: true };
    },
  }, { cwd: "/project" });
  const outcome = await runCodemode("while (true) {}", bundle.asgrep, { timeoutMs: 20 });
  assert.equal(outcome.ok, false);
  if (!outcome.ok) assert.match(outcome.error, /timed out|timeout/iu);
});

test("runner timeout rejects a hanging await without a Worker", async () => {
  const bundle = createAsgrepConnector({
    async run(): Promise<MachineEnvelope> {
      return { tool: "asgrep", schema_version: "1.0.0", ok: true };
    },
  }, { cwd: "/project" });
  const started = Date.now();
  const outcome = await runCodemode(`return await new Promise(() => {});`, bundle.asgrep, { timeoutMs: 20 });
  assert.equal(outcome.ok, false);
  if (!outcome.ok) assert.match(outcome.error, /timed out|timeout/iu);
  assert.ok(Date.now() - started < 2_000, "in-process timeout should remain bounded");
});

test("runner serializes result getters inside the VM timeout", async () => {
  const bundle = createAsgrepConnector({
    async run(): Promise<MachineEnvelope> {
      return { tool: "asgrep", schema_version: "1.0.0", ok: true };
    },
  }, { cwd: "/project" });
  const outcome = await runCodemode(
    `return Object.defineProperty({}, "value", {
      enumerable: true,
      get() { while (true) {} },
    });`,
    bundle.asgrep,
    { timeoutMs: 20 },
  );
  assert.equal(outcome.ok, false);
  if (!outcome.ok) assert.match(outcome.error, /timed out|timeout/iu);
});

test("runner bounds call arguments, logs, and serialized results before returning to the host", async () => {
  let hostCalls = 0;
  const bundle = createAsgrepConnector({
    async run(): Promise<MachineEnvelope> {
      hostCalls += 1;
      return { tool: "asgrep", schema_version: "1.0.0", ok: true };
    },
  }, { cwd: "/project" });

  const oversizedCall = await runCodemode(
    `return await asgrep.search({ query: "x".repeat(70_000) });`,
    bundle.asgrep,
  );
  assert.equal(oversizedCall.ok, false);
  if (!oversizedCall.ok) assert.match(oversizedCall.error, /call arguments exceed/iu);
  assert.equal(hostCalls, 0, "oversized arguments must be rejected before dispatch");

  const oversizedResult = await runCodemode(`return "x".repeat(1_100_000);`, bundle.asgrep);
  assert.equal(oversizedResult.ok, false);
  if (!oversizedResult.ok) assert.match(oversizedResult.error, /result exceeds/iu);

  const boundedLogs = await runCodemode(
    `for (let i = 0; i < 1_000; i += 1) console.log("x".repeat(10_000)); return true;`,
    bundle.asgrep,
  );
  assert.equal(boundedLogs.ok, true, boundedLogs.ok ? undefined : boundedLogs.error);
  assert.ok(boundedLogs.logs.length <= 100);
  assert.ok(boundedLogs.logs.every((line) => line.length <= 4_096));
  assert.ok(boundedLogs.logs.reduce((total, line) => total + line.length, 0) <= 64_000);

  const oversizedError = await runCodemode(`throw new Error("x".repeat(100_000));`, bundle.asgrep);
  assert.equal(oversizedError.ok, false);
  if (!oversizedError.ok) assert.ok(oversizedError.error.length <= 8_192);
});

test("runner bounds total bridge fan-out", async () => {
  let hostCalls = 0;
  const bundle = createAsgrepConnector({
    async run(): Promise<MachineEnvelope> {
      hostCalls += 1;
      return { tool: "asgrep", schema_version: "1.0.0", ok: true };
    },
  }, { cwd: "/project" });
  const outcome = await runCodemode(
    `for (let i = 0; i < 257; i += 1) await asgrep.indexStatus(); return true;`,
    bundle.asgrep,
  );
  assert.equal(outcome.ok, false);
  if (!outcome.ok) assert.match(outcome.error, /exceeds 256 host calls/iu);
  assert.equal(hostCalls, 256);
});

test("runner observes cancellation while awaiting asynchronous code", async () => {
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
  if (outcome.ok) {
    assert.fail(`expected abort failure, got ${JSON.stringify(outcome.result)}`);
  } else {
    assert.match(outcome.error, /aborted/iu);
  }
});

test("runner timeout cancels in-flight host work and stops later bridge calls", async () => {
  let hostCalls = 0;
  let hostAborted = false;
  let hostStarted!: () => void;
  const started = new Promise<void>((resolve) => { hostStarted = resolve; });
  const bundle = createAsgrepConnector({
    async run(_args, _context, options): Promise<MachineEnvelope> {
      hostCalls += 1;
      hostStarted();
      return new Promise((_resolve, reject) => {
        const onAbort = () => {
          hostAborted = true;
          reject(Object.assign(new Error("host call aborted"), { name: "AbortError" }));
        };
        if (options?.signal?.aborted) {
          onAbort();
          return;
        }
        options?.signal?.addEventListener("abort", onAbort, { once: true });
      });
    },
  }, { cwd: "/project" });
  const pending = runCodemode(
    `await asgrep.search({ query: "one" });
     await asgrep.search({ query: "two" });
     return true;`,
    bundle.asgrep,
    { timeoutMs: 250 },
  );
  await started;
  const outcome = await pending;
  assert.equal(outcome.ok, false);
  if (!outcome.ok) assert.match(outcome.error, /timeout/iu);
  assert.equal(hostAborted, true, "soft timeout must abort the in-flight host call");
  const callsAtTimeout = hostCalls;
  await new Promise((resolve) => setTimeout(resolve, 80));
  assert.equal(hostCalls, callsAtTimeout, "timed-out program must not keep dispatching host calls");
  assert.ok(hostCalls <= 2, `orphaned AsyncFunction kept calling the session: ${hostCalls}`);
});

test("runner cancellation cancels an in-flight host call", async () => {
  let hostStarted!: () => void;
  const started = new Promise<void>((resolve) => { hostStarted = resolve; });
  let hostAborted!: () => void;
  const aborted = new Promise<void>((resolve) => { hostAborted = resolve; });
  const bundle = createAsgrepConnector({
    async run(_args, _context, options): Promise<MachineEnvelope> {
      hostStarted();
      return new Promise((_resolve, reject) => {
        const onAbort = () => {
          hostAborted();
          reject(Object.assign(new Error("host call aborted"), { name: "AbortError" }));
        };
        options?.signal?.addEventListener("abort", onAbort, { once: true });
      });
    },
  }, { cwd: "/project" });
  const controller = new AbortController();
  const run = runCodemode(
    `return await asgrep.search({ query: "never completes" });`,
    bundle.asgrep,
    { timeoutMs: 5_000, signal: controller.signal },
  );
  await started;
  controller.abort();
  const outcome = await run;
  assert.equal(outcome.ok, false);
  await aborted;
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
  assert.throws(
    () => argvFor("catalog_search", { query: "search" }),
    /no direct CLI fallback/,
  );
  assert.deepEqual(argvFor("find", { query: "hello", limit: 8, excerpt_lines: 0 }), [
    "--json", "--format", "agent-capsule", "--limit", "8", "--excerpt-lines", "0", "word:hello", ".",
  ]);
  assert.deepEqual(argvFor("find", { query: "defs:Foo", limit: 4 }), [
    "--json", "--format", "agent-capsule", "--limit", "4", "--excerpt-lines", "0", "defs:Foo", ".",
  ]);
  assert.deepEqual(argvFor("find", { query: "blast:Foo", limit: 4 }), [
    "--json", "--format", "agent-capsule", "--limit", "4", "--excerpt-lines", "0", "callers:Foo", ".",
  ]);
  assert.deepEqual(argvFor("find", { query: "blast:src/auth.ts", limit: 4 }), [
    "--json", "--format", "agent-capsule", "--limit", "4", "--excerpt-lines", "0", "imports:src/auth.ts", ".",
  ]);
  assert.throws(() => argvFor("read", { path: "a.ts" }), /no direct CLI fallback/);
  assert.throws(() => argvFor("edit", { path: "a.ts", oldText: "a", newText: "b" }), /no direct CLI fallback/);
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
    host.call("search", { query: "a", limit: 8, format: "capsule" }, { cwd: "/p" }),
    host.call("search", { query: "b", limit: 8, format: "capsule" }, { cwd: "/p" }),
  ]);
  assert.equal(stats().waves, 1);
  assert.equal(stats().calls, 2);
  assert.equal(stats().parallelSpawnCalls, 2);
});

test("find/read/edit ride the same Promise.all wave", async () => {
  const tools: string[] = [];
  const host = {
    async run(): Promise<MachineEnvelope> {
      throw new Error("run should not be used");
    },
    sticky: {
      async call(tool: string) {
        tools.push(tool);
        return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [{ symbol: tool }] };
      },
      async batch(calls: Array<{ id: string; tool: string }>) {
        for (const c of calls) tools.push(c.tool);
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
      const [a, b, c] = await Promise.all([
        asgrep.search({ query: "one" }),
        asgrep.find({ query: "Foo" }),
        asgrep.read({ path: "a.ts", start: 1, end: 2 }),
      ]);
      return { a: a.hits[0].symbol, b: b.hits[0].symbol, c: c.hits[0].symbol };
    }`,
    bundle.asgrep,
    { stats: bundle.stats },
  );
  assert.equal(outcome.ok, true, outcome.ok ? undefined : outcome.error);
  assert.deepEqual(outcome.result, { a: "search", b: "find", c: "read" });
  assert.equal(bundle.stats().waves, 1);
  assert.deepEqual(tools.sort(), ["find", "read", "search"]);
});

test("edit is a mutating tool and does not spawn-replay after sticky failure", async () => {
  const transportFailure = new Error("sticky died");
  let spawnFallbacks = 0;
  const dispatcher = createCodemodeDispatcher({
    sticky: {
      async call() { throw new Error("not used"); },
      async batch() { throw transportFailure; },
      async end() {},
    },
    async run() {
      spawnFallbacks += 1;
      return asEnvelope({ hits: [] });
    },
  });
  const results = await Promise.allSettled([
    dispatcher.host.call("edit", { path: "a.ts", oldText: "a", newText: "b" }, { cwd: "/p" }),
    dispatcher.host.call("search", { query: "auth" }, { cwd: "/p" }),
  ]);
  assert.deepEqual(results.map(({ status }) => status), ["rejected", "rejected"]);
  assert.equal(spawnFallbacks, 0);
});

test("in-process Code Mode activation stays off Worker spawn", async () => {
  const bundle = createAsgrepConnector({
    async run(): Promise<MachineEnvelope> {
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [] };
    },
  }, { cwd: "/p" });
  await resetCodemodeSandboxForTests();
  await warmCodemodeSandbox();
  const first = await runCodemode("return 1", bundle.asgrep);
  const second = await runCodemode("return 2", bundle.asgrep);
  assert.equal(first.ok, true, first.ok ? undefined : first.error);
  assert.equal(second.ok, true, second.ok ? undefined : second.error);
  assert.equal(second.result, 2);
  assert.ok(second.wallMs < 20, `in-process activation ${second.wallMs}ms`);
});
