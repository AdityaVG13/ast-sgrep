// Guest worker for asgrep Code Mode. A REAL FILE (not an embedded template
// string) so it gets syntax checking, real line numbers in stack traces, and
// normal review. Loaded by runner.ts via new URL("./guest-worker.mjs", ...) and
// spawned through an eval'd `import()` bootstrap so host execArgv flags never
// leak into the isolate.
//
// Protocol (all messages carry runId; stale-runId frames are dropped):
//   in:  { op: "run", runId, code, timeoutMs }
//        { op: "call-result", runId, id, body }
//   out: { op: "call-batch", runId, calls: [{ id, method, payload }] }
//        { op: "log", runId, line }
//        { op: "result", runId, serialized }
//        { op: "error", runId, error }
import { parentPort } from "node:worker_threads";
import vm from "node:vm";

// In-guest caps are a second fence; the host enforces the same bounds on its
// side (runner.ts owns the authoritative limits).
const MAX_RESULT_JSON_CHARS = 1_000_000;
const MAX_BRIDGE_REQUEST_CHARS = 64_000;
const MAX_LOG_LINE_CHARS = 4_096;
const MAX_ERROR_CHARS = 8_192;
const RESULT_SERIALIZE_TIMEOUT_MS = 1_000;

const BLOCKED_GLOBALS = [
  "ArrayBuffer", "SharedArrayBuffer", "DataView", "Atomics", "WebAssembly",
  "eval", "Function", "AsyncFunction", "GeneratorFunction",
  "Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array",
  "Int32Array", "Uint32Array", "Float32Array", "Float64Array",
  "BigInt64Array", "BigUint64Array",
];

// Mirrors CODEMODE_HOST_METHODS (types.ts); duplicated so this file stays
// dependency-free plain JS.
const KNOWN_METHODS = [
  "search", "find", "read", "edit", "semantic", "chain", "defs", "callers",
  "imports", "indexStatus", "indexRepo", "doctor", "catalogSearch", "catalogDescribe",
];

let runId = -1;
let callSeq = 0;
const pendingCalls = new Map();
let outbox = [];
let outboxScheduled = false;

const post = (msg) => {
  try {
    parentPort.postMessage({ ...msg, runId });
    return true;
  } catch {
    return false;
  }
};

// Calls issued in one guest microtask burst travel as ONE message so the host
// dispatcher sees them in a single tick and can coalesce the wave (sticky
// batch / runBatch). Per-message posts arrive on separate host turns and
// would silently defeat batching.
const bridge = (method, payload) => new Promise((resolve, reject) => {
  const id = "c" + callSeq++;
  pendingCalls.set(id, { resolve, reject });
  outbox.push({ id, method, payload });
  if (outboxScheduled) return;
  outboxScheduled = true;
  queueMicrotask(() => {
    const calls = outbox;
    outbox = [];
    outboxScheduled = false;
    if (!post({ op: "call-batch", calls })) {
      for (const call of calls) {
        const pending = pendingCalls.get(call.id);
        if (pending) {
          pendingCalls.delete(call.id);
          pending.resolve(JSON.stringify({ ok: false, error: "codemode call failed" }));
        }
      }
    }
  });
});

const logSink = (line) => {
  post({ op: "log", line: String(line) });
};

// NAPI u64/i64 fields cross the bridge as BigInt; a guest re-passing them must
// not die on "Do not know how to serialize a BigInt".
const jsonSafe = (_key, item) =>
  typeof item === "bigint"
    ? (item >= -9007199254740991n && item <= 9007199254740991n ? Number(item) : item.toString())
    : item;

const stringify = JSON.stringify;
const stringifyBounded = (value, maxChars, label) => {
  const serialized = stringify(value, jsonSafe);
  if (serialized === undefined) return serialized;
  if (serialized.length > maxChars) {
    throw new Error("codemode " + label + " exceeds " + maxChars + " characters");
  }
  return serialized;
};

const sealCtor = (obj) => {
  if (obj === null || obj === undefined) return;
  try {
    Object.defineProperty(obj, "constructor", { value: undefined, configurable: false, writable: false });
  } catch {}
};

// Everything below runs INSIDE the vm context once, installing the sandbox
// surface: blocked globals, sealed constructors, the asgrep proxy, console.
function installSandbox(contextObject) {
  const context = vm.createContext(contextObject, { codeGeneration: { strings: false, wasm: false } });
  const install = new vm.Script(INSTALL_SOURCE, { filename: "asgrep-codemode-bootstrap.js" });
  return { context, install };
}

const INSTALL_SOURCE = `
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
  sealCtor(Object); sealCtor(Object.prototype);
  sealCtor(Array); sealCtor(Array.prototype);
  sealCtor(Number); sealCtor(Number.prototype);
  sealCtor(String); sealCtor(String.prototype);
  sealCtor(Boolean); sealCtor(Boolean.prototype);
  sealCtor(Error); sealCtor(Error.prototype);
  sealCtor(RegExp); sealCtor(RegExp.prototype);
  sealCtor(Date); sealCtor(Date.prototype);
  sealCtor(Promise); sealCtor(Promise.prototype);
  sealCtor(JSON); sealCtor(Math);
  sealCtor(Reflect); sealCtor(Proxy); sealCtor(Symbol);
  sealCtor(Map); sealCtor(Set); sealCtor(WeakMap); sealCtor(WeakSet);
  sealCtor(hostCall); sealCtor(hostLog);

  let resultValue;
  const setResult = (value) => { resultValue = value; };
  const stringify = JSON.stringify;
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
  const known = ${JSON.stringify(KNOWN_METHODS)};
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
  sealCtor(api); sealCtor(consoleApi); sealCtor(setResult); sealCtor(serializeResult); sealCtor(invoke);
}
`;

async function runProgram(msg) {
  const contextObject = Object.create(null);
  Object.defineProperty(bridge, "constructor", { value: undefined });
  Object.defineProperty(logSink, "constructor", { value: undefined });
  contextObject.__asgrepBridge = bridge;
  contextObject.__asgrepLog = logSink;
  const { context, install } = installSandbox(contextObject);
  try {
    install.runInContext(context, { timeout: Math.min(msg.timeoutMs, 1000) });
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
      timeout: Math.min(msg.timeoutMs, RESULT_SERIALIZE_TIMEOUT_MS),
    });
    post({ op: "result", serialized });
  } catch (cause) {
    const message = cause instanceof Error ? cause.message : String(cause);
    post({ op: "error", error: message.slice(0, MAX_ERROR_CHARS) });
  }
}

parentPort.on("message", (msg) => {
  if (!msg || typeof msg !== "object") return;
  // "run" claims the runId; every later frame must carry it or be dropped.
  if (msg.op === "run") {
    if (runId !== -1) return;
    runId = msg.runId;
    void runProgram(msg);
    return;
  }
  if (typeof msg.runId === "number" && msg.runId !== runId) return;
  if (msg.op === "call-result") {
    const pending = pendingCalls.get(msg.id);
    if (pending) {
      pendingCalls.delete(msg.id);
      pending.resolve(msg.body);
    }
    return;
  }
});

post({ op: "ready" });
