import { Worker } from "node:worker_threads";
import type { AsgrepConnector } from "./connector.js";
import type { DispatchStats } from "./dispatch.js";
import { coerceHostArgs, normalizeCode, packGuestCall, resolveHostMethod, timeoutHint, unknownMethodError } from "./guest-api.js";
import { CODEMODE_HOST_METHODS, type CodemodeHostMethod } from "./types.js";

export { normalizeCode };

/** Closed sum: success|failure — `ok:true` with `error` (or `ok:false` without) is unrepresentable. */
export type CodemodeRunSuccess = {
  ok: true;
  result: unknown;
  logs: string[];
  code: string;
  stats?: DispatchStats;
  wallMs: number;
};

export type CodemodeRunFailure = {
  ok: false;
  result: null;
  error: string;
  logs: string[];
  code: string;
  stats?: DispatchStats;
  wallMs: number;
};

export type CodemodeRunResult = CodemodeRunSuccess | CodemodeRunFailure;

const DEFAULT_TIMEOUT_MS = 30_000;
const MAX_CODE_CHARS = 32_000;
const MAX_BRIDGE_CALLS = 256;
const MAX_BRIDGE_REQUEST_CHARS = 64_000;
const MAX_ERROR_CHARS = 8_192;
const MAX_LOG_LINES = 100;
const MAX_LOG_CHARS = 64_000;
const MAX_LOG_LINE_CHARS = 4_096;
const MAX_RESULT_JSON_CHARS = 1_000_000;
const RESULT_SERIALIZE_TIMEOUT_MS = 1_000;
const MAX_TIMER_MS = 2_147_483_647;
/** Hard ceiling on guest heap; an OOM guest kills its isolate, never the host. */
const WORKER_MAX_OLD_SPACE_MB = 512;
const WORKER_MAX_YOUNG_SPACE_MB = 64;

/** NAPI u64/i64 fields cross as BigInt; keep safe ints numeric, exact-string the rest. */
const jsonSafe = (_key: string, item: unknown): unknown =>
  typeof item === "bigint"
    ? item >= -9007199254740991n && item <= 9007199254740991n
      ? Number(item)
      : item.toString()
    : item;

type HostMethod = CodemodeHostMethod;

const BLOCKED_GLOBALS = [
  "ArrayBuffer",
  "SharedArrayBuffer",
  "DataView",
  "Atomics",
  "WebAssembly",
  "eval",
  "Function",
  "AsyncFunction",
  "GeneratorFunction",
  "Int8Array",
  "Uint8Array",
  "Uint8ClampedArray",
  "Int16Array",
  "Uint16Array",
  "Int32Array",
  "Uint32Array",
  "Float32Array",
  "Float64Array",
  "BigInt64Array",
  "BigUint64Array",
];

function bootstrapSource(): string {
  return `
  {
    const hostCall = globalThis.__asgrepBridge;
    const hostLog = globalThis.__asgrepLog;
    delete globalThis.__asgrepBridge;
    delete globalThis.__asgrepLog;

    for (const name of ${JSON.stringify(BLOCKED_GLOBALS)}) {
      Object.defineProperty(globalThis, name, {
        value: undefined, configurable: false, writable: false,
      });
    }

    const sealCtor = (obj) => {
      if (obj === null || obj === undefined) return;
      try {
        Object.defineProperty(obj, "constructor", {
          value: undefined, configurable: false, writable: false,
        });
      } catch {}
    };
    sealCtor(globalThis);
    sealCtor(Object);
    sealCtor(Object.prototype);
    sealCtor(Array);
    sealCtor(Array.prototype);
    sealCtor(Number);
    sealCtor(Number.prototype);
    sealCtor(String);
    sealCtor(String.prototype);
    sealCtor(Boolean);
    sealCtor(Boolean.prototype);
    sealCtor(Error);
    sealCtor(Error.prototype);
    sealCtor(RegExp);
    sealCtor(RegExp.prototype);
    sealCtor(Date);
    sealCtor(Date.prototype);
    sealCtor(Promise);
    sealCtor(Promise.prototype);
    sealCtor(JSON);
    sealCtor(Math);
    sealCtor(Reflect);
    sealCtor(Proxy);
    sealCtor(Symbol);
    sealCtor(Map);
    sealCtor(Set);
    sealCtor(WeakMap);
    sealCtor(WeakSet);
    sealCtor(hostCall);
    sealCtor(hostLog);

    let resultValue;
    const setResult = (value) => { resultValue = value; };
    const stringify = JSON.stringify;
    // NAPI u64/i64 fields cross the bridge as BigInt; a guest re-passing them
    // must not die on "Do not know how to serialize a BigInt".
    const jsonSafe = (_key, item) => typeof item === "bigint"
      ? (item >= -9007199254740991n && item <= 9007199254740991n ? Number(item) : item.toString())
      : item;
    const stringifyBounded = (value, maxChars, label) => {
      const serialized = stringify(value, jsonSafe);
      if (serialized === undefined) return serialized;
      if (serialized.length > maxChars) {
        throw new Error("codemode " + label + " exceeds " + maxChars + " characters");
      }
      return serialized;
    };
    const serializeResult = () => stringifyBounded(resultValue, ${MAX_RESULT_JSON_CHARS}, "result");
    Object.freeze(setResult);
    Object.freeze(serializeResult);
    Object.defineProperty(globalThis, "__asgrepSetResult", {
      value: setResult, configurable: false, writable: false,
    });
    Object.defineProperty(globalThis, "__asgrepSerializeResult", {
      value: serializeResult, configurable: false, writable: false,
    });

    const invoke = async (method, args = {}) => {
      const payload = stringifyBounded(args, ${MAX_BRIDGE_REQUEST_CHARS}, "call arguments");
      const response = JSON.parse(await hostCall(method, payload));
      if (!response.ok) throw new Error(response.error || ("asgrep." + method + " failed"));
      return response.value;
    };
    const known = ${JSON.stringify([...CODEMODE_HOST_METHODS])};
    const blocked = new Set(["then", "constructor", "prototype", "__proto__"]);
    const call = (method) => (...guestArgs) => invoke(method, { __guestArgs: guestArgs });
    const api = new Proxy(Object.create(null), {
      get(_target, prop) {
        if (typeof prop !== "string" || blocked.has(prop)) return undefined;
        return call(prop);
      },
      has(_target, prop) {
        return typeof prop === "string" && !blocked.has(prop);
      },
      ownKeys() { return known.slice(); },
      getOwnPropertyDescriptor(_target, prop) {
        if (typeof prop !== "string" || blocked.has(prop)) return undefined;
        return { enumerable: known.includes(prop), configurable: true, value: call(prop) };
      },
      set() { return false; },
      defineProperty() { return false; },
      deleteProperty() { return false; },
    });

    const formatLog = (value) => {
      if (typeof value === "string") return value.slice(0, ${MAX_LOG_LINE_CHARS});
      try { return stringifyBounded(value, ${MAX_LOG_LINE_CHARS}, "log line"); }
      catch { return "[unserializable or oversized log value]"; }
    };
    const consoleApi = Object.create(null);
    for (const level of ["log", "info", "warn", "error", "debug"]) {
      Object.defineProperty(consoleApi, level, {
        enumerable: true,
        value: (...args) => {
          let line = "";
          for (const arg of args) {
            const part = formatLog(arg);
            const prefix = line.length === 0 ? "" : " ";
            const remaining = ${MAX_LOG_LINE_CHARS} - line.length;
            if (remaining <= 0) break;
            line += (prefix + part).slice(0, remaining);
          }
          hostLog(line);
        },
      });
    }
    Object.freeze(consoleApi);

    Object.defineProperty(globalThis, "asgrep", { value: api, configurable: false, writable: false });
    Object.defineProperty(globalThis, "console", { value: consoleApi, configurable: false, writable: false });
    sealCtor(api);
    sealCtor(consoleApi);
    sealCtor(setResult);
    sealCtor(serializeResult);
    sealCtor(invoke);
  }
  `;
}

/**
 * Worker-side program. One program per worker; the worker is always terminated
 * when the run ends. Guest microtasks share the worker's own loop, so a detached
 * `Promise.resolve().then(loop)` starves only the isolate — the host kills it
 * via `terminate()` at the wall-clock deadline. This is the boundary the
 * previous in-process vm could not provide: in-process, a detached guest
 * microtask could block Pi's event loop forever.
 */
function workerSource(): string {
  return `
"use strict";
const { parentPort } = require("node:worker_threads");
const vm = require("node:vm");

const BOOTSTRAP_SOURCE = ${JSON.stringify(bootstrapSource())};

let callSeq = 0;
const pendingCalls = new Map();
let outbox = [];
let outboxScheduled = false;

// Calls issued in one guest microtask burst travel as ONE message so the
// host dispatcher sees them in a single tick and can coalesce the wave
// (sticky batch / runBatch). Per-message posts arrive on separate host
// turns and would silently defeat batching.
const bridge = (method, payload) => new Promise((resolve, reject) => {
  const id = "c" + (callSeq++);
  pendingCalls.set(id, { resolve, reject });
  outbox.push({ id, method, payload });
  if (!outboxScheduled) {
    outboxScheduled = true;
    queueMicrotask(() => {
      const calls = outbox;
      outbox = [];
      outboxScheduled = false;
      try {
        parentPort.postMessage({ type: "call-batch", calls });
      } catch (cause) {
        for (const call of calls) {
          const pending = pendingCalls.get(call.id);
          if (pending) {
            pendingCalls.delete(call.id);
            pending.resolve(JSON.stringify({ ok: false, error: "codemode call failed" }));
          }
        }
      }
    });
  }
});
const logSink = (line) => {
  try { parentPort.postMessage({ type: "log", line: String(line) }); } catch {}
};

parentPort.on("message", (msg) => {
  if (!msg || typeof msg !== "object") return;
  if (msg.type === "call-result") {
    const pending = pendingCalls.get(msg.id);
    if (pending) {
      pendingCalls.delete(msg.id);
      pending.resolve(msg.body);
    }
    return;
  }
  if (msg.type === "run") {
    void runProgram(msg);
  }
});

async function runProgram(msg) {
  const contextObject = Object.create(null);
  Object.defineProperty(bridge, "constructor", { value: undefined });
  Object.defineProperty(logSink, "constructor", { value: undefined });
  contextObject.__asgrepBridge = bridge;
  contextObject.__asgrepLog = logSink;
  const context = vm.createContext(contextObject, {
    codeGeneration: { strings: false, wasm: false },
  });
  try {
    const bootstrapScript = new vm.Script(BOOTSTRAP_SOURCE, { filename: "asgrep-codemode-bootstrap.js" });
    bootstrapScript.runInContext(context, { timeout: Math.min(msg.timeoutMs, 1000) });
    const script = new vm.Script(msg.code, { filename: "asgrep-codemode.js" });
    const value = await script.runInContext(context, {
      displayErrors: true,
      timeout: msg.timeoutMs,
    });
    const setResult = context.__asgrepSetResult;
    if (typeof setResult !== "function") {
      throw new Error("codemode result bridge is unavailable");
    }
    setResult(value);
    const serializeScript = new vm.Script("globalThis.__asgrepSerializeResult()", {
      filename: "asgrep-codemode-result.js",
    });
    const serialized = serializeScript.runInContext(context, {
      displayErrors: true,
      timeout: Math.min(msg.timeoutMs, ${RESULT_SERIALIZE_TIMEOUT_MS}),
    });
    parentPort.postMessage({ type: "result", serialized });
  } catch (cause) {
    const message = cause instanceof Error ? cause.message : String(cause);
    try {
      parentPort.postMessage({ type: "error", error: message.slice(0, ${MAX_ERROR_CHARS}) });
    } catch {}
  }
}
`;
}

type HostFn = (
  args: Record<string, unknown>,
  options?: { signal?: AbortSignal },
) => Promise<unknown>;

function bindHostMethods(asgrep: AsgrepConnector): Record<HostMethod, HostFn> {
  const wrap = (
    fn: (args: never, options?: { signal?: AbortSignal }) => Promise<unknown>,
  ): HostFn => (args, options) => fn(args as never, options);
  return {
    search: wrap(asgrep.search.bind(asgrep)),
    find: wrap(asgrep.find.bind(asgrep)),
    read: wrap(asgrep.read.bind(asgrep)),
    edit: wrap(asgrep.edit.bind(asgrep)),
    semantic: wrap(asgrep.semantic.bind(asgrep)),
    chain: wrap(asgrep.chain.bind(asgrep)),
    defs: wrap(asgrep.defs.bind(asgrep)),
    callers: wrap(asgrep.callers.bind(asgrep)),
    imports: wrap(asgrep.imports.bind(asgrep)),
    indexStatus: (_args, options) => asgrep.indexStatus(options),
    indexRepo: wrap(asgrep.indexRepo.bind(asgrep)),
    doctor: (_args, options) => asgrep.doctor(options),
    catalogSearch: wrap(asgrep.catalogSearch.bind(asgrep)),
    catalogDescribe: wrap(asgrep.catalogDescribe.bind(asgrep)),
  };
}

/**
 * Warm standby worker. Each call consumes the standby and immediately spawns a
 * replacement, so the spawn cost is paid off the critical path while every
 * program still gets a virgin isolate. The standby is unref'd so it never
 * holds the Pi process open.
 */
let standby: Worker | null = null;
let standbyStarting: Promise<Worker> | null = null;
const activeWorkers = new Set<Worker>();

function spawnWorker(): Promise<Worker> {
  const worker = new Worker(workerSource(), spawnWorkerOptions());
  return new Promise<Worker>((resolve, reject) => {
    worker.once("online", () => resolve(worker));
    worker.once("error", reject);
  });
}

function spawnWorkerOptions(): import("node:worker_threads").WorkerOptions {
  return {
    eval: true,
    // A clean isolate: the guest must not inherit --import loaders, --test,
    // or any other host flags (tsx bootstrap crashes the eval'd worker).
    execArgv: [],
    resourceLimits: {
      maxOldGenerationSizeMb: WORKER_MAX_OLD_SPACE_MB,
      maxYoungGenerationSizeMb: WORKER_MAX_YOUNG_SPACE_MB,
    },
  };
}

function ensureStandby(): void {
  if (standby || standbyStarting) return;
  // The stored promise doubles as a claim token: takeWorker() may adopt the
  // in-flight spawn for direct use, so the then-callback must only publish to
  // standby while the spawn is still standby-owned. Without the identity
  // check an adopted worker could be re-published and handed out twice.
  const claimed = spawnWorker()
    .then((worker) => {
      if (standbyStarting === claimed) {
        standby = worker;
        standbyStarting = null;
        worker.unref();
      }
      return worker;
    })
    .catch((cause) => {
      if (standbyStarting === claimed) standbyStarting = null;
      throw cause;
    });
  standbyStarting = claimed;
  // The warm path is opportunistic; a spawn failure must not crash the host.
  void claimed.catch(() => undefined);
}

async function takeWorker(): Promise<Worker> {
  const ready = standby;
  standby = null;
  const starting = standbyStarting;
  standbyStarting = null;
  // Refill the warm slot for the next call before doing any real work.
  ensureStandby();
  if (ready) {
    ready.ref();
    return ready;
  }
  if (starting) {
    try {
      const worker = await starting;
      worker.ref();
      return worker;
    } catch {
      // fall through to a cold spawn
    }
  }
  return spawnWorker();
}

/** Spawn the warm standby isolate (session_start / pre-call). */
export async function warmCodemodeSandbox(): Promise<void> {
  ensureStandby();
}

/** Drop the standby isolate (tests / session shutdown). */
export async function resetCodemodeSandboxForTests(): Promise<void> {
  const worker = standby;
  standby = null;
  const starting = standbyStarting;
  standbyStarting = null;
  if (starting) {
    const spawned = await starting.catch(() => undefined);
    if (spawned) await spawned.terminate().catch(() => undefined);
  }
  if (worker) await worker.terminate().catch(() => undefined);
  // In-flight runs own their workers, but shutdown must not leave them behind:
  // terminating surfaces a "codemode worker exited" failure to the caller.
  await Promise.all([...activeWorkers].map((active) => active.terminate().catch(() => undefined)));
}

type WorkerMessage =
  | { type: "call-batch"; calls: Array<{ id: string; method: string; payload: string }> }
  | { type: "log"; line: string }
  | { type: "result"; serialized?: string }
  | { type: "error"; error: string };

/**
 * Run model-generated JavaScript against the typed `asgrep` connector.
 *
 * Execution happens in a single-use `worker_threads` isolate: the guest gets a
 * `node:vm` context inside the worker; `asgrep`/`console` are built there; the
 * only host channel is a JSON postMessage bridge. Timeout and abort call
 * `worker.terminate()`, which is the only mechanism that actually stops a
 * detached guest microtask or a runaway heap (each worker is heap-capped).
 */
export async function runCodemode(
  rawCode: string,
  asgrep: AsgrepConnector,
  options: {
    timeoutMs?: number;
    signal?: AbortSignal;
    stats?: () => DispatchStats;
  } = {},
): Promise<CodemodeRunResult> {
  const requestedTimeout = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const timeoutMs = Number.isFinite(requestedTimeout)
    ? Math.min(MAX_TIMER_MS, Math.max(1, Math.trunc(requestedTimeout)))
    : DEFAULT_TIMEOUT_MS;
  const wall0 = performance.now();
  if (rawCode.length > MAX_CODE_CHARS) {
    return resultErr(`code exceeds ${MAX_CODE_CHARS} characters`, [], rawCode.slice(0, 200), wall0, options.stats);
  }
  if (options.signal?.aborted) {
    return resultErr("codemode aborted", [], rawCode.slice(0, 200), wall0, options.stats);
  }

  const code = normalizeCode(rawCode);
  const hostMethods = bindHostMethods(asgrep);
  const logs: string[] = [];
  let logChars = 0;
  let callCount = 0;

  const runController = new AbortController();

  const hostCall = async (method: string, payload: string): Promise<string> => {
    try {
      if (runController.signal.aborted) {
        throw Object.assign(new Error("codemode aborted"), { name: "AbortError" });
      }
      if (callCount >= MAX_BRIDGE_CALLS) {
        throw new Error(`codemode exceeds ${MAX_BRIDGE_CALLS} host calls`);
      }
      callCount += 1;
      if (payload.length > MAX_BRIDGE_REQUEST_CHARS) {
        throw new Error(`codemode call arguments exceed ${MAX_BRIDGE_REQUEST_CHARS} characters`);
      }
      const resolved = resolveHostMethod(method);
      if (!resolved || !Object.hasOwn(hostMethods, resolved)) {
        throw new Error(unknownMethodError(method));
      }
      const parsed = JSON.parse(payload) as Record<string, unknown>;
      const packed = Array.isArray(parsed.__guestArgs)
        ? packGuestCall(resolved, parsed.__guestArgs)
        : parsed;
      const input = coerceHostArgs(resolved, packed);
      const invokeHost = hostMethods[resolved];
      if (!invokeHost) throw new Error(unknownMethodError(method));
      const value = await invokeHost(input, { signal: runController.signal });
      return JSON.stringify({ ok: true, value }, jsonSafe);
    } catch (cause) {
      return JSON.stringify({
        ok: false,
        error: safeErrorMessage(cause).slice(0, MAX_ERROR_CHARS),
      });
    }
  };

  const hostLog = (line: string): void => {
    if (logs.length >= MAX_LOG_LINES || logChars >= MAX_LOG_CHARS) return;
    const remaining = MAX_LOG_CHARS - logChars;
    const bounded = line.length <= remaining
      ? line
      : `${line.slice(0, Math.max(0, remaining - 1))}…`;
    logs.push(bounded);
    logChars += bounded.length;
  };

  let worker: Worker;
  try {
    worker = await takeWorker();
  } catch (cause) {
    return resultErr(
      `codemode worker unavailable: ${safeErrorMessage(cause)}`,
      logs,
      code,
      wall0,
      options.stats,
    );
  }

  activeWorkers.add(worker);
  return new Promise<CodemodeRunResult>((resolveRun) => {
    let settled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const finish = (outcome: CodemodeRunResult): void => {
      if (settled) return;
      settled = true;
      activeWorkers.delete(worker);
      if (timer) clearTimeout(timer);
      options.signal?.removeEventListener("abort", onAbort);
      runController.abort();
      void worker.terminate().catch(() => undefined);
      resolveRun(outcome);
    };
    const fail = (message: string): void => {
      finish(resultErr(timeoutHint(message).slice(0, MAX_ERROR_CHARS), logs, code, wall0, options.stats));
    };

    const onAbort = (): void => fail("codemode aborted");

    const onMessage = (msg: WorkerMessage): void => {
      if (!msg || typeof msg !== "object") return;
      if (msg.type === "log") {
        hostLog(msg.line);
        return;
      }
      if (msg.type === "call-batch") {
        const calls = Array.isArray(msg.calls) ? msg.calls : [];
        for (const call of calls) {
          const respond = (body: string): void => {
            try {
              worker.postMessage({ type: "call-result", id: call.id, body });
            } catch {
              // Worker already terminated; the run is settled.
            }
          };
          void hostCall(call.method, call.payload).then(
            respond,
            () => respond(JSON.stringify({ ok: false, error: "codemode call failed" })),
          );
        }
        return;
      }
      if (msg.type === "result") {
        try {
          const serialized = msg.serialized;
          const result = serialized === undefined ? undefined : JSON.parse(serialized) as unknown;
          finish(resultOk(result, logs, code, wall0, options.stats));
        } catch (cause) {
          fail("codemode result decode failed: " + safeErrorMessage(cause));
        }
        return;
      }
      if (msg.type === "error") {
        fail(msg.error);
      }
    };

    const onError = (cause: Error): void => {
      fail(`codemode worker error: ${cause.message}`);
    };
    const onExit = (exitCode: number): void => {
      fail(`codemode worker exited code=${exitCode}`);
    };

    worker.on("message", onMessage);
    worker.once("error", onError);
    worker.once("exit", onExit);

    if (options.signal) {
      if (options.signal.aborted) {
        fail("codemode aborted");
        return;
      }
      options.signal.addEventListener("abort", onAbort, { once: true });
    }
    timer = setTimeout(() => {
      fail(`codemode timeout after ${timeoutMs}ms`);
    }, timeoutMs);
    timer.unref?.();

    worker.postMessage({ type: "run", code, timeoutMs });
  });
}

function safeErrorMessage(cause: unknown): string {
  try {
    return String(cause instanceof Error ? cause.message : cause);
  } catch {
    return "codemode call failed";
  }
}

function resultOk(
  result: unknown,
  logs: string[],
  code: string,
  wall0: number,
  statsFn?: () => DispatchStats,
): CodemodeRunSuccess {
  const out: CodemodeRunSuccess = { ok: true, result, logs, code, wallMs: performance.now() - wall0 };
  const stats = statsFn?.();
  if (stats) out.stats = stats;
  return out;
}

function resultErr(
  error: string,
  logs: string[],
  code: string,
  wall0: number,
  statsFn?: () => DispatchStats,
): CodemodeRunFailure {
  const out: CodemodeRunFailure = { ok: false, result: null, logs, error, code, wallMs: performance.now() - wall0 };
  const stats = statsFn?.();
  if (stats) out.stats = stats;
  return out;
}
