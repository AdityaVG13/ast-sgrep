import assert from "node:assert/strict";
import test from "node:test";
import { isUncachedSearchError, NativeSessionPool, normalizeNativeValue } from "../../../packages/pi/extension/src/codemode/session-pool.js";
import type { StickyWorker } from "../../../packages/pi/extension/src/codemode/dispatch.js";
import type { MachineEnvelope } from "../../../packages/pi/extension/src/runtime/runtime.js";

test("native BigInts retain exact JSON values outside the safe range", () => {
  const result = normalizeNativeValue({ generation: 9007199254740993n, small: 3n, values: [-9007199254740993n] });
  assert.deepEqual(result, { generation: "9007199254740993", small: 3, values: ["-9007199254740993"] });
  assert.doesNotThrow(() => JSON.stringify(result));
});

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

test("invalidated startup failures cannot poison a replacement generation", async () => {
  const staleStart = Promise.withResolvers<StickyWorker>();
  let starts = 0;
  const pool = new NativeSessionPool(async () => {
    if (++starts === 1) return staleStart.promise;
    return fakeWorker([]);
  });
  pool.configure({ binary: "/fake/asgrep" });
  const stale = pool.acquire("/project");
  const invalidated = pool.invalidate("/project");
  staleStart.reject(new Error("old startup failed after invalidation"));
  await invalidated;
  assert.equal(await stale, null);
  try {
    assert.equal(pool.lastStartError("/project"), undefined, "obsolete errors must not impose restart backoff");
    assert.ok(await pool.acquire("/project"), "replacement must be available immediately after invalidation");
  } finally {
    await pool.shutdown();
  }
});

test("a stale closed call cannot retire the replacement worker", async () => {
  const failedCall = Promise.withResolvers<MachineEnvelope>();
  const callStarted = Promise.withResolvers<void>();
  let starts = 0;
  let replacementClosed = false;
  const replacement: StickyWorker = {
    ...fakeWorker([]),
    closed: () => replacementClosed,
    async end() { replacementClosed = true; },
  };
  const pool = new NativeSessionPool(async () => {
    if (++starts === 1) return {
      ...fakeWorker([]),
      call() { callStarted.resolve(); return failedCall.promise; },
    };
    return starts === 2 ? replacement : fakeWorker([]);
  });
  pool.configure({ binary: "/fake/asgrep" });
  const pending = pool.call("/project", "search");
  await callStarted.promise;
  try {
    await pool.invalidate("/project");
    assert.equal(await pool.acquire("/project"), replacement);
    failedCall.reject(new Error("codemode-serve is closed"));
    assert.equal((await pending).ok, true);
    assert.equal(replacementClosed, false, "a retired worker has no authority over its replacement");
    assert.equal(await pool.acquire("/project"), replacement);
  } finally {
    failedCall.reject(new Error("codemode-serve is closed"));
    await pending.catch(() => undefined);
    await pool.shutdown();
  }
});

test("a retired session's closed call cannot restart after shutdown", async () => {
  const failedCall = Promise.withResolvers<MachineEnvelope>();
  const callStarted = Promise.withResolvers<void>();
  const pool = new NativeSessionPool(async () => ({
    ...fakeWorker([]),
    call() { callStarted.resolve(); return failedCall.promise; },
  }));
  pool.configure({ binary: "/fake/asgrep" });
  const pending = pool.call("/project", "search");
  await callStarted.promise;
  const rejected = assert.rejects(pending, /codemode-serve is closed/);
  try {
    await pool.shutdown();
    failedCall.reject(new Error("codemode-serve is closed"));
    await rejected;
    assert.equal(pool.backend(), "none", "retired calls must not restart the backend");
    assert.ok(await pool.acquire("/project"), "an explicit new call may reuse the pool after shutdown");
  } finally {
    failedCall.reject(new Error("codemode-serve is closed"));
    await rejected.catch(() => undefined);
    await pool.shutdown();
  }
});

test("shutdown drains workers already removed by invalidation", async () => {
  const retirementStarted = Promise.withResolvers<void>();
  const releaseRetirement = Promise.withResolvers<void>();
  const pool = new NativeSessionPool(async () => ({
    ...fakeWorker([]),
    async end() { retirementStarted.resolve(); await releaseRetirement.promise; },
  }));
  pool.configure({ binary: "/fake/asgrep" });
  await pool.acquire("/project");
  const invalidated = pool.invalidate("/project");
  await retirementStarted.promise;
  let shutdownComplete = false;
  const shutdown = pool.shutdown().then(() => { shutdownComplete = true; });
  try {
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(shutdownComplete, false, "shutdown must drain previously admitted retirement");
  } finally {
    releaseRetirement.resolve();
    await Promise.all([invalidated, shutdown]);
  }
});

test("a closed acquire cannot resume into a retired session", async () => {
  const retirementStarted = Promise.withResolvers<void>();
  const releaseRetirement = Promise.withResolvers<void>();
  let closed = false;
  let starts = 0;
  const pool = new NativeSessionPool(async () => {
    if (++starts !== 1) return fakeWorker([]);
    return {
      ...fakeWorker([]),
      closed: () => closed,
      async end() { retirementStarted.resolve(); await releaseRetirement.promise; },
    };
  });
  pool.configure({ binary: "/fake/asgrep" });
  await pool.acquire("/project");
  closed = true;
  const pending = pool.acquire("/project");
  await retirementStarted.promise;
  const shutdown = pool.shutdown();
  try {
    releaseRetirement.resolve();
    assert.equal(await pending, null, "an acquire admitted before shutdown must not create a new worker");
    await shutdown;
    assert.ok(await pool.acquire("/project"), "new callers can reuse the completed pool");
  } finally {
    releaseRetirement.resolve();
    await Promise.all([pending, shutdown]);
    await pool.shutdown();
  }
});

test("uncached unique search is deferred off the JS thread", () => {
  assert.equal(
    isUncachedSearchError(new Error("callNow is only for bounded metadata/symbol lookups; use call() for search/index/semantic/chain")),
    true,
  );
  assert.equal(isUncachedSearchError(new Error("session is busy")), false);
  assert.equal(isUncachedSearchError(new Error("native session is closed")), false);
});
