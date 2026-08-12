import vm from "node:vm";
import { parentPort, workerData } from "node:worker_threads";
const port = (() => {
    if (!parentPort)
        throw new Error("codemode sandbox requires a parent port");
    return parentPort;
})();
const data = workerData;
const pending = new Map();
const outgoing = [];
let nextCallId = 0;
let flushScheduled = false;
port.on("message", (message) => {
    if (message.type !== "callResult")
        return;
    const resolve = pending.get(message.id);
    if (!resolve)
        return;
    pending.delete(message.id);
    resolve(message.payload);
});
const bridge = (method, payload) => new Promise((resolve) => {
    if (nextCallId >= data.limits.bridgeCalls) {
        resolve(JSON.stringify({
            ok: false,
            error: `codemode exceeds ${data.limits.bridgeCalls} host calls`,
        }));
        return;
    }
    const id = nextCallId++;
    pending.set(id, resolve);
    outgoing.push({ id, method, payload });
    if (!flushScheduled) {
        flushScheduled = true;
        queueMicrotask(() => {
            flushScheduled = false;
            const calls = outgoing.splice(0);
            if (calls.length > 0)
                port.postMessage({ type: "calls", calls });
        });
    }
});
void run();
async function run() {
    const logs = [];
    let logChars = 0;
    const logBridge = (line) => {
        if (logs.length >= data.limits.logLines || logChars >= data.limits.logChars)
            return;
        const remaining = data.limits.logChars - logChars;
        const bounded = line.length <= remaining
            ? line
            : `${line.slice(0, Math.max(0, remaining - 1))}…`;
        logs.push(bounded);
        logChars += bounded.length;
    };
    Object.setPrototypeOf(bridge, null);
    Object.setPrototypeOf(logBridge, null);
    Object.freeze(bridge);
    Object.freeze(logBridge);
    try {
        const globals = Object.create(null);
        globals.__asgrepBridge = bridge;
        globals.__asgrepLog = logBridge;
        const context = vm.createContext(globals, {
            codeGeneration: { strings: false, wasm: false },
        });
        new vm.Script(bootstrap(data.limits), {
            filename: "asgrep-codemode-bootstrap.js",
        }).runInContext(context, { timeout: Math.min(data.timeoutMs, 1_000) });
        const script = new vm.Script(data.code, { filename: "asgrep-codemode.js" });
        const value = await Promise.resolve(script.runInContext(context, {
            displayErrors: true,
            timeout: data.timeoutMs,
        }));
        const setResult = context.__asgrepSetResult;
        if (typeof setResult !== "function") {
            throw new Error("codemode result bridge is unavailable");
        }
        setResult(value);
        const serialized = new vm.Script("globalThis.__asgrepSerializeResult()", {
            filename: "asgrep-codemode-result.js",
        }).runInContext(context, {
            displayErrors: true,
            timeout: Math.min(data.timeoutMs, data.limits.serializeTimeoutMs),
        });
        const result = serialized === undefined ? undefined : JSON.parse(serialized);
        finish({ type: "done", ok: true, result, logs });
    }
    catch (cause) {
        finish({
            type: "done",
            ok: false,
            error: safeErrorMessage(cause).slice(0, data.limits.errorChars),
            logs,
        });
    }
}
function safeErrorMessage(cause) {
    try {
        return String(cause instanceof Error ? cause.message : cause);
    }
    catch {
        return "codemode worker failed";
    }
}
function finish(message) {
    port.postMessage(message);
    port.close();
}
function bootstrap(limits) {
    return `
  {
    const hostCall = globalThis.__asgrepBridge;
    const hostLog = globalThis.__asgrepLog;
    delete globalThis.__asgrepBridge;
    delete globalThis.__asgrepLog;

    let resultValue;
    const setResult = (value) => { resultValue = value; };
    const stringify = JSON.stringify;
    const stringifyBounded = (value, maxChars, label) => {
      let remaining = maxChars;
      const serialized = stringify(value, (key, item) => {
        remaining -= key.length + 8;
        if (typeof item === "string") remaining -= item.length;
        if (remaining < 0) throw new Error(\`codemode \${label} exceeds \${maxChars} characters\`);
        return item;
      });
      if (serialized !== undefined && serialized.length > maxChars) {
        throw new Error(\`codemode \${label} exceeds \${maxChars} characters\`);
      }
      return serialized;
    };
    const serializeResult = () => stringifyBounded(resultValue, ${limits.resultJsonChars}, "result");
    Object.freeze(setResult);
    Object.freeze(serializeResult);
    Object.defineProperty(globalThis, "__asgrepSetResult", {
      value: setResult, configurable: false, writable: false,
    });
    Object.defineProperty(globalThis, "__asgrepSerializeResult", {
      value: serializeResult, configurable: false, writable: false,
    });

    // Worker heap limits do not reliably account for backing stores. Code Mode
    // exchanges JSON, so raw-memory and WebAssembly APIs add risk without utility.
    for (const name of [
      "ArrayBuffer", "SharedArrayBuffer", "DataView", "Atomics", "WebAssembly",
      "Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array",
      "Int32Array", "Uint32Array", "Float32Array", "Float64Array",
      "BigInt64Array", "BigUint64Array",
    ]) {
      Object.defineProperty(globalThis, name, {
        value: undefined, configurable: false, writable: false,
      });
    }

    const invoke = async (method, args = {}) => {
      const payload = stringifyBounded(args, ${limits.bridgeRequestChars}, "call arguments");
      const response = JSON.parse(await hostCall(method, payload));
      if (!response.ok) throw new Error(response.error || \`asgrep.\${method} failed\`);
      return response.value;
    };
    const api = Object.create(null);
    for (const method of [
      "search", "semantic", "chain", "defs", "callers", "imports",
      "indexStatus", "indexRepo", "catalogSearch", "catalogDescribe",
    ]) {
      Object.defineProperty(api, method, {
        enumerable: true,
        value: (args = {}) => invoke(method, args),
      });
    }
    Object.freeze(api);

    const formatLog = (value) => {
      if (typeof value === "string") return value.slice(0, ${limits.logLineChars});
      try { return stringifyBounded(value, ${limits.logLineChars}, "log line"); }
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
            const remaining = ${limits.logLineChars} - line.length;
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
  }
  `;
}
