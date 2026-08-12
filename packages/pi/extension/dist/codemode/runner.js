import { Worker } from "node:worker_threads";
const DEFAULT_TIMEOUT_MS = 30_000;
const MAX_CODE_CHARS = 32_000;
const MAX_BRIDGE_CALLS = 256;
const MAX_BRIDGE_REQUEST_CHARS = 64_000;
const MAX_BRIDGE_RESPONSE_CHARS = 4 * 1024 * 1024;
const MAX_ERROR_CHARS = 8_192;
const MAX_LOG_LINES = 100;
const MAX_LOG_CHARS = 64_000;
const MAX_LOG_LINE_CHARS = 4_096;
const MAX_RESULT_JSON_CHARS = 1_000_000;
const RESULT_SERIALIZE_TIMEOUT_MS = 1_000;
const MAX_TIMER_MS = 2_147_483_647;
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
/**
 * Run model-generated JavaScript against the typed `asgrep` connector.
 *
 * Model-generated code is not trusted with the extension host's ambient Node
 * authority. A dedicated worker contains CPU/microtask denial of service; its
 * VM hides `process`, module loading, and host constructors, with a JSON bridge
 * as the only exposed capability. This is not an OS sandbox, so deployments
 * requiring adversarial-code isolation should still restrict the Pi process.
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
    const hostMethods = {
        search: asgrep.search.bind(asgrep),
        semantic: asgrep.semantic.bind(asgrep),
        chain: asgrep.chain.bind(asgrep),
        defs: asgrep.defs.bind(asgrep),
        callers: asgrep.callers.bind(asgrep),
        imports: asgrep.imports.bind(asgrep),
        indexStatus: asgrep.indexStatus.bind(asgrep),
        indexRepo: asgrep.indexRepo.bind(asgrep),
        catalogSearch: asgrep.catalogSearch.bind(asgrep),
        catalogDescribe: asgrep.catalogDescribe.bind(asgrep),
    };
    const workerUrl = new URL(import.meta.url.endsWith(".ts") ? "./sandbox-worker.ts" : "./sandbox-worker.js", import.meta.url);
    let worker;
    try {
        worker = new Worker(workerUrl, {
            workerData: {
                code,
                timeoutMs,
                limits: {
                    bridgeCalls: MAX_BRIDGE_CALLS,
                    bridgeRequestChars: MAX_BRIDGE_REQUEST_CHARS,
                    errorChars: MAX_ERROR_CHARS,
                    logLines: MAX_LOG_LINES,
                    logChars: MAX_LOG_CHARS,
                    logLineChars: MAX_LOG_LINE_CHARS,
                    resultJsonChars: MAX_RESULT_JSON_CHARS,
                    serializeTimeoutMs: RESULT_SERIALIZE_TIMEOUT_MS,
                },
            },
            resourceLimits: {
                maxOldGenerationSizeMb: 64,
                maxYoungGenerationSizeMb: 16,
                stackSizeMb: 4,
            },
        });
    }
    catch (cause) {
        return resultErr(cause instanceof Error ? cause.message : String(cause), [], code, wall0, options.stats);
    }
    return new Promise((resolve) => {
        let active = true;
        const receivedCallIds = new Set();
        const finish = (outcome) => {
            if (!active)
                return;
            active = false;
            clearTimeout(timer);
            options.signal?.removeEventListener("abort", onAbort);
            // Cancel host work that the disposable worker was awaiting or abandoned.
            runController.abort();
            void worker.terminate().catch(() => undefined).then(() => {
                outcome.wallMs = Date.now() - wall0;
                resolve(outcome);
            });
        };
        const fail = (error, logs = []) => {
            finish(resultErr(error, logs, code, wall0, options.stats));
        };
        const onAbort = () => fail("codemode aborted");
        const timer = setTimeout(() => fail(`codemode timeout after ${timeoutMs}ms`), timeoutMs);
        worker.on("message", (message) => {
            if (!active)
                return;
            if (!isSandboxMessage(message)) {
                fail("codemode worker sent an invalid message");
                return;
            }
            if (message.type === "done") {
                if (message.ok) {
                    finish(resultOk(message.result, message.logs, code, wall0, options.stats));
                }
                else {
                    fail(message.error ?? "codemode worker failed", message.logs);
                }
                return;
            }
            for (const call of message.calls) {
                if (call.id >= MAX_BRIDGE_CALLS || receivedCallIds.has(call.id)) {
                    fail("codemode worker exceeded its bridge call allowance");
                    return;
                }
                receivedCallIds.add(call.id);
            }
            for (const call of message.calls)
                void handleSandboxCall(call);
        });
        worker.once("error", (error) => fail(error.message));
        worker.once("exit", (code) => {
            if (active)
                fail(`codemode worker exited ${code}`);
        });
        const handleSandboxCall = async (call) => {
            if (!active)
                return;
            let payload;
            try {
                if (call.payload.length > MAX_BRIDGE_REQUEST_CHARS) {
                    throw new Error(`codemode call arguments exceed ${MAX_BRIDGE_REQUEST_CHARS} characters`);
                }
                if (!Object.hasOwn(hostMethods, call.method)) {
                    throw new Error(`unknown asgrep method: ${call.method}`);
                }
                const input = JSON.parse(call.payload);
                const methodCall = hostMethods[call.method];
                const value = await methodCall(input, { signal: runController.signal });
                payload = stringifyBounded({ ok: true, value }, MAX_BRIDGE_RESPONSE_CHARS, "codemode call result");
            }
            catch (cause) {
                payload = JSON.stringify({
                    ok: false,
                    error: safeErrorMessage(cause).slice(0, MAX_ERROR_CHARS),
                });
            }
            if (active)
                worker.postMessage({ type: "callResult", id: call.id, payload });
        };
        options.signal?.addEventListener("abort", onAbort, { once: true });
        if (options.signal?.aborted)
            onAbort();
    });
}
function isSandboxMessage(message) {
    if (typeof message !== "object" || message === null || !("type" in message))
        return false;
    if (message.type === "calls") {
        return "calls" in message
            && Array.isArray(message.calls)
            && message.calls.length > 0
            && message.calls.length <= MAX_BRIDGE_CALLS
            && message.calls.every((call) => isSandboxCall(call));
    }
    if (message.type !== "done"
        || !("ok" in message)
        || typeof message.ok !== "boolean"
        || !("logs" in message)
        || !Array.isArray(message.logs)
        || message.logs.length > MAX_LOG_LINES
        || !message.logs.every((line) => typeof line === "string" && line.length <= MAX_LOG_LINE_CHARS)
        || message.logs.reduce((total, line) => total + line.length, 0) > MAX_LOG_CHARS) {
        return false;
    }
    return !("error" in message)
        || message.error === undefined
        || (typeof message.error === "string" && message.error.length <= MAX_ERROR_CHARS);
}
function isSandboxCall(call) {
    return typeof call === "object"
        && call !== null
        && "id" in call
        && typeof call.id === "number"
        && Number.isSafeInteger(call.id)
        && call.id >= 0
        && "method" in call
        && typeof call.method === "string"
        && "payload" in call
        && typeof call.payload === "string";
}
function safeErrorMessage(cause) {
    try {
        return String(cause instanceof Error ? cause.message : cause);
    }
    catch {
        return "codemode call failed";
    }
}
function stringifyBounded(value, maxBytes, label) {
    let remaining = maxBytes;
    const payload = JSON.stringify(value, (key, item) => {
        remaining -= Buffer.byteLength(key) + 8;
        if (typeof item === "string")
            remaining -= Buffer.byteLength(item);
        if (remaining < 0)
            throw new Error(`${label} exceeds ${maxBytes} bytes`);
        return item;
    });
    if (payload === undefined || Buffer.byteLength(payload) > maxBytes) {
        throw new Error(`${label} exceeds ${maxBytes} bytes`);
    }
    return payload;
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
