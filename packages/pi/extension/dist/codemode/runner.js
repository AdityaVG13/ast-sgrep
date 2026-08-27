import vm from "node:vm";
import { CODEMODE_HOST_METHODS } from "./types.js";
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
function bootstrapSource() {
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
    const stringifyBounded = (value, maxChars, label) => {
      let remaining = maxChars;
      const serialized = stringify(value, (key, item) => {
        remaining -= key.length + 8;
        if (typeof item === "string") remaining -= item.length;
        if (remaining < 0) throw new Error("codemode " + label + " exceeds " + maxChars + " characters");
        return item;
      });
      if (serialized !== undefined && serialized.length > maxChars) {
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
    const api = Object.create(null);
    for (const method of ${JSON.stringify([...CODEMODE_HOST_METHODS])}) {
      Object.defineProperty(api, method, {
        enumerable: true,
        value: (args = {}) => invoke(method, args),
      });
    }
    Object.freeze(api);

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
const bootstrapScript = new vm.Script(bootstrapSource(), {
    filename: "asgrep-codemode-bootstrap.js",
});
const serializeScript = new vm.Script("globalThis.__asgrepSerializeResult()", {
    filename: "asgrep-codemode-result.js",
});
/** Strip markdown fences and normalize to an async IIFE expression. */
export function normalizeCode(raw) {
    let code = raw.trim();
    if (code.startsWith("```")) {
        code = code.replace(/^```(?:javascript|js|typescript|ts)?\s*/i, "").replace(/\s*```$/, "").trim();
    }
    if (/^async\s*\(/.test(code) || /^async\s+function\b/.test(code)) {
        return `(${code.endsWith(";") ? code.slice(0, -1) : code})()`;
    }
    return `(async () => {\n${code}\n})()`;
}
function bindHostMethods(asgrep) {
    const wrap = (fn) => (args, options) => fn(args, options);
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
        catalogSearch: wrap(asgrep.catalogSearch.bind(asgrep)),
        catalogDescribe: wrap(asgrep.catalogDescribe.bind(asgrep)),
    };
}
/** No-op: programs run in-process. Kept so session_start / tests stay stable. */
export async function warmCodemodeSandbox() { }
/** No-op: there is no sticky Worker isolate to drop. */
export async function resetCodemodeSandboxForTests() { }
/**
 * Run model-generated JavaScript against the typed `asgrep` connector.
 *
 * In-process `node:vm` (OpenCode/nicknisi: no Worker, no OS sandbox). `asgrep`
 * and `console` are built inside the context; the only host objects are a
 * JSON bridge and a log sink. Same trust as Pi `bash`.
 */
export async function runCodemode(rawCode, asgrep, options = {}) {
    const requestedTimeout = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    const timeoutMs = Number.isFinite(requestedTimeout)
        ? Math.min(MAX_TIMER_MS, Math.max(1, Math.trunc(requestedTimeout)))
        : DEFAULT_TIMEOUT_MS;
    const wall0 = Date.now();
    if (rawCode.length > MAX_CODE_CHARS) {
        return resultErr(`code exceeds ${MAX_CODE_CHARS} characters`, [], rawCode.slice(0, 200), wall0, options.stats);
    }
    if (options.signal?.aborted) {
        return resultErr("codemode aborted", [], rawCode.slice(0, 200), wall0, options.stats);
    }
    const code = normalizeCode(rawCode);
    const runController = new AbortController();
    const hostMethods = bindHostMethods(asgrep);
    const logs = [];
    let logChars = 0;
    let callCount = 0;
    const hostCall = async (method, payload) => {
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
            if (!Object.hasOwn(hostMethods, method)) {
                throw new Error(`unknown asgrep method: ${method}`);
            }
            const input = JSON.parse(payload);
            const value = await hostMethods[method](input, { signal: runController.signal });
            return JSON.stringify({ ok: true, value });
        }
        catch (cause) {
            return JSON.stringify({
                ok: false,
                error: safeErrorMessage(cause).slice(0, MAX_ERROR_CHARS),
            });
        }
    };
    const hostLog = (line) => {
        if (logs.length >= MAX_LOG_LINES || logChars >= MAX_LOG_CHARS)
            return;
        const remaining = MAX_LOG_CHARS - logChars;
        const bounded = line.length <= remaining
            ? line
            : `${line.slice(0, Math.max(0, remaining - 1))}…`;
        logs.push(bounded);
        logChars += bounded.length;
    };
    const contextObject = Object.create(null);
    Object.defineProperty(hostCall, "constructor", { value: undefined });
    Object.defineProperty(hostLog, "constructor", { value: undefined });
    contextObject.__asgrepBridge = hostCall;
    contextObject.__asgrepLog = hostLog;
    const context = vm.createContext(contextObject, {
        codeGeneration: { strings: false, wasm: false },
    });
    let timer;
    const onAbort = () => {
        runController.abort();
    };
    options.signal?.addEventListener("abort", onAbort, { once: true });
    try {
        bootstrapScript.runInContext(context, { timeout: Math.min(timeoutMs, 1_000) });
        const script = new vm.Script(code, { filename: "asgrep-codemode.js" });
        const timeout = new Promise((_, reject) => {
            timer = setTimeout(() => {
                runController.abort();
                reject(new Error(`codemode timeout after ${timeoutMs}ms`));
            }, timeoutMs);
        });
        const aborted = options.signal
            ? new Promise((_, reject) => {
                if (options.signal?.aborted) {
                    reject(new Error("codemode aborted"));
                    return;
                }
                options.signal?.addEventListener("abort", () => reject(new Error("codemode aborted")), { once: true });
            })
            : undefined;
        const value = await Promise.race([
            Promise.resolve(script.runInContext(context, {
                displayErrors: true,
                timeout: timeoutMs,
            })),
            timeout,
            ...(aborted ? [aborted] : []),
        ]);
        const setResult = context.__asgrepSetResult;
        if (typeof setResult !== "function") {
            throw new Error("codemode result bridge is unavailable");
        }
        setResult(value);
        const serialized = serializeScript.runInContext(context, {
            displayErrors: true,
            timeout: Math.min(timeoutMs, RESULT_SERIALIZE_TIMEOUT_MS),
        });
        const result = serialized === undefined ? undefined : JSON.parse(serialized);
        return resultOk(result, logs, code, wall0, options.stats);
    }
    catch (cause) {
        return resultErr(safeErrorMessage(cause).slice(0, MAX_ERROR_CHARS), logs, code, wall0, options.stats);
    }
    finally {
        if (timer)
            clearTimeout(timer);
        options.signal?.removeEventListener("abort", onAbort);
        runController.abort();
    }
}
function safeErrorMessage(cause) {
    try {
        return String(cause instanceof Error ? cause.message : cause);
    }
    catch {
        return "codemode call failed";
    }
}
function resultOk(result, logs, code, wall0, statsFn) {
    const out = { ok: true, result, logs, code, wallMs: Date.now() - wall0 };
    const stats = statsFn?.();
    if (stats)
        out.stats = stats;
    return out;
}
function resultErr(error, logs, code, wall0, statsFn) {
    const out = { ok: false, result: null, logs, error, code, wallMs: Date.now() - wall0 };
    const stats = statsFn?.();
    if (stats)
        out.stats = stats;
    return out;
}
