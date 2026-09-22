import assert from "node:assert/strict";
import test from "node:test";
import { isUncachedSearchError, NativeSessionPool } from "../../../packages/pi/extension/src/codemode/session-pool.js";
import type { StickyWorker } from "../../../packages/pi/extension/src/codemode/dispatch.js";
import type { MachineEnvelope } from "../../../packages/pi/extension/src/runtime/runtime.js";

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

test("a crash-looping backend backs off and surfaces the real start error", async () => {
  let starts = 0;
  const pool = new NativeSessionPool(async () => {
    starts += 1;
    throw new Error("index schema 16 is newer than supported version 15");
  });
  pool.configure({ binary: "/fake/asgrep" });

  assert.equal(await pool.acquire("/p"), null);
  assert.equal(starts, 1);
  // Inside the backoff window the pool does not pay another doomed spawn.
  assert.equal(await pool.acquire("/p"), null);
  assert.equal(starts, 1);
  assert.match(pool.lastStartError("/p") ?? "", /newer than supported/u);

  // pool.call on a dead transport surfaces the real start error, not a stale
  // "is closed" that hides the schema refusal.
  const pool2 = new NativeSessionPool(async () => {
    throw new Error("spawn ENOENT codemode-serve");
  });
  pool2.configure({ binary: "/fake/asgrep" });
  await assert.rejects(
    pool2.call("/p", "search", {}),
    /codemode backend unavailable: spawn ENOENT codemode-serve/u,
  );
});

test("a freshly spawned worker that is already closed counts as a start failure", async () => {
  let starts = 0;
  const deadOnArrival = (): StickyWorker => ({
    closed: () => true,
    async call() { throw new Error("codemode-serve is closed"); },
    async batch() { throw new Error("codemode-serve is closed"); },
    async end() {},
  });
  const pool = new NativeSessionPool(async () => {
    starts += 1;
    return deadOnArrival();
  });
  pool.configure({ binary: "/fake/asgrep" });
  assert.equal(await pool.acquire("/p"), null);
  assert.equal(starts, 1);
  // Second acquire inside backoff must not spawn again.
  assert.equal(await pool.acquire("/p"), null);
  assert.equal(starts, 1);
  assert.match(pool.lastStartError("/p") ?? "", /exited during startup/u);
});

test("a closed worker is replaced on the next call and recovers automatically", async () => {
  const log: string[] = [];
  let closed = false;
  const flaky = (): StickyWorker => ({
    closed: () => closed,
    async call(tool) {
      if (closed) throw new Error("codemode-serve is closed");
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [] } as MachineEnvelope;
    },
    async batch() { return { results: [] }; },
    async end() { closed = true; },
  });
  let starts = 0;
  const pool = new NativeSessionPool(async () => {
    starts += 1;
    closed = false;
    return flaky();
  });
  pool.configure({ binary: "/fake/asgrep" });

  const w1 = await pool.acquire("/p");
  assert.ok(w1);
  // Kill the transport out-of-band (serve crash): next call retries once and
  // the restarted worker serves normally.
  closed = true;
  const response = await pool.call("/p", "search", {});
  assert.equal(response.ok, true);
  assert.equal(starts, 2);
  await pool.shutdown();
});

test("pre-aborted calls reject before starting a backend", async () => {
  let starts = 0;
  const pool = new NativeSessionPool(async () => {
    starts += 1;
    return fakeWorker([]);
  });
  pool.configure({ binary: "/fake/asgrep" });
  const controller = new AbortController();
  controller.abort();

  await assert.rejects(pool.call("/p", "search", {}, { signal: controller.signal }), {
    name: "AbortError",
  });
  assert.equal(starts, 0);
});

test("aborting an in-flight pool call unblocks the next caller", async () => {
  const abortErr = () => Object.assign(new Error("native call aborted"), { name: "AbortError" });
  let started = 0;
  const pool = new NativeSessionPool(async () => ({
    async call(_tool, _args, options) {
      started += 1;
      if (!options?.signal) {
        return { tool: "asgrep", schema_version: "1.0.0", ok: true } as MachineEnvelope;
      }
      if (options.signal.aborted) throw abortErr();
      await new Promise<void>((_resolve, reject) => {
        options.signal.addEventListener("abort", () => reject(abortErr()), { once: true });
      });
      return { tool: "asgrep", schema_version: "1.0.0", ok: true } as MachineEnvelope;
    },
    async batch() {
      return { results: [] };
    },
    async end() {},
  }));
  pool.configure({ binary: "/fake/asgrep" });
  const controller = new AbortController();
  const pending = pool.call("/p", "search", {}, { signal: controller.signal });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(started, 1);
  controller.abort();
  await assert.rejects(pending, { name: "AbortError" });
  const startedAt = Date.now();
  await pool.call("/p", "index_status");
  assert.ok(Date.now() - startedAt < 500, "next caller must not wait on aborted in-flight work");
  await pool.shutdown();
});

test("acquire restarts a closed sticky worker", async () => {
  let log: string[] = [];
  let starts = 0;
  let pool = new NativeSessionPool(async () => {
    starts += 1;
    let closed = false;
    return {
      closed: () => closed,
      async call(tool) {
        if (closed) throw new Error("codemode-serve is closed");
        log.push(`call:${tool}`);
        return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [] } as MachineEnvelope;
      },
      async batch() {
        return { results: [] };
      },
      async end() {
        closed = true;
        log.push("end");
      },
    };
  });
  pool.configure({ binary: "/fake/asgrep" });
  const first = await pool.acquire("/p");
  await first!.end();
  await pool.call("/p", "search", { query: "x" });
  assert.equal(starts, 2);
  assert.ok(log.includes("call:search"));
  await pool.shutdown();
});

test("mutating pool calls never replay after an ambiguous transport failure", async () => {
  for (const tool of ["edit", "index_repo"]) {
    let calls = 0;
    const pool = new NativeSessionPool(async () => ({
      ...fakeWorker([]),
      async call() { calls++; throw new Error("codemode-serve is closed"); },
    }));
    pool.configure({ binary: "/fake/asgrep" });
    await assert.rejects(pool.call("/p", tool), /closed/);
    assert.equal(calls, 1, "the first call may have committed before transport failure");
    await pool.shutdown();
  }
});

test("call retries once after codemode-serve is closed", async () => {
  let starts = 0;
  const pool = new NativeSessionPool(async () => {
    starts += 1;
    return {
      async call() {
        if (starts === 1) throw new Error("codemode-serve is closed");
        return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [] } as MachineEnvelope;
      },
      async batch() {
        return { results: [] };
      },
      async end() {},
    };
  });
  pool.configure({ binary: "/fake/asgrep" });
  const result = await pool.call("/p", "search", { query: "x" });
  assert.equal(result.ok, true);
  assert.equal(starts, 2);
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
  let shutdownComplete = false;
  const shutdown = pool.shutdown().then(() => { shutdownComplete = true; });
  await Promise.resolve();
  assert.equal(shutdownComplete, false, "shutdown must wait for in-flight starts");
  assert.equal(
    await pool.acquire("/project"),
    null,
    "an acquire concurrent with shutdown must not start a replacement worker",
  );
  assert.equal(starts, 1);
  release();
  await shutdown;
  assert.equal(await stale, null);
  assert.ok(log.includes("end"), "stale worker must be closed");
  assert.ok(await pool.acquire("/project"));
  assert.equal(starts, 2);
  await pool.shutdown();
});

test("uncached unique search is deferred off the JS thread", () => {
  assert.equal(
    isUncachedSearchError(new Error("callNow is only for bounded metadata/symbol lookups; use call() for search/index/semantic/chain")),
    true,
  );
  assert.equal(isUncachedSearchError(new Error("session is busy")), false);
  assert.equal(isUncachedSearchError(new Error("native session is closed")), false);
});
