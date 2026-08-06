import assert from "node:assert/strict";
import test from "node:test";
import { NativeSessionPool } from "../src/codemode/session-pool.js";
import type { StickyWorker } from "../src/codemode/dispatch.js";
import type { MachineEnvelope } from "../src/runtime.js";

function fakeWorker(log: string[]): StickyWorker {
  return {
    async call(tool) {
      log.push(`call:${tool}`);
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [] } as MachineEnvelope;
    },
    async batch(calls) {
      log.push(`batch:${calls.length}`);
      return {
        results: calls.map((c) => ({
          id: c.id,
          ok: true,
          value: { tool: "asgrep", schema_version: "1.0.0", ok: true },
        })),
      };
    },
    async end() {
      log.push("end");
    },
  };
}

test("session pool starts once per root and reuses the worker", async () => {
  const log: string[] = [];
  let starts = 0;
  const pool = new NativeSessionPool(async (opts) => {
    starts += 1;
    log.push(`start:${opts.cwd}`);
    return fakeWorker(log);
  });
  pool.configure({ binary: "/fake/asgrep" });

  const a = await pool.acquire("/project");
  const b = await pool.acquire("/project");
  assert.equal(starts, 1);
  assert.equal(a, b);

  await pool.call("/project", "search", { query: "auth" });
  await pool.call("/project", "defs", { symbol: "Foo" });
  assert.deepEqual(log.filter((x) => x.startsWith("call:")), ["call:search", "call:defs"]);

  const other = await pool.acquire("/other");
  assert.equal(starts, 2);
  assert.notEqual(other, a);

  await pool.shutdown();
  assert.ok(log.filter((x) => x === "end").length >= 2);
});

test("concurrent acquire shares one in-flight start", async () => {
  let starts = 0;
  let release!: () => void;
  const gate = new Promise<void>((r) => {
    release = r;
  });
  const pool = new NativeSessionPool(async () => {
    starts += 1;
    await gate;
    return fakeWorker([]);
  });
  pool.configure({ binary: "/fake/asgrep" });
  const p1 = pool.acquire("/p");
  const p2 = pool.acquire("/p");
  release();
  const [a, b] = await Promise.all([p1, p2]);
  assert.equal(starts, 1);
  assert.equal(a, b);
  await pool.shutdown();
});

test("invalidate drops worker so next acquire restarts", async () => {
  const log: string[] = [];
  let starts = 0;
  const pool = new NativeSessionPool(async () => {
    starts += 1;
    return fakeWorker(log);
  });
  pool.configure({ binary: "/fake/asgrep" });
  await pool.acquire("/p");
  await pool.invalidate("/p");
  assert.ok(log.includes("end"));
  await pool.acquire("/p");
  assert.equal(starts, 2);
  await pool.shutdown();
});

test("invalidating one root does not cancel another root's in-flight start", async () => {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  const pool = new NativeSessionPool(async () => {
    await gate;
    return fakeWorker([]);
  });
  pool.configure({ binary: "/fake/asgrep" });
  const other = pool.acquire("/other");
  await pool.invalidate("/project");
  release();
  assert.ok(await other);
  await pool.shutdown();
});

test("shutdown prevents an in-flight start from repopulating the pool", async () => {
  const log: string[] = [];
  let starts = 0;
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  const pool = new NativeSessionPool(async () => {
    starts += 1;
    if (starts === 1) await gate;
    return fakeWorker(log);
  });
  pool.configure({ binary: "/fake/asgrep" });
  const stale = pool.acquire("/project");
  await pool.shutdown();
  release();
  assert.equal(await stale, null);
  assert.ok(log.includes("end"), "stale worker must be closed");
  assert.ok(await pool.acquire("/project"));
  assert.equal(starts, 2);
  await pool.shutdown();
});
