import vm from "node:vm";
const DEFAULT_TIMEOUT_MS = 30_000;
const MAX_CODE_CHARS = 32_000;
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
 * Run model-generated JavaScript with only `asgrep` + safe builtins.
 *
 * Uses the shared microtask queue so host Promises from `asgrep.*` resolve under
 * `Promise.all`. Do not enable `microtaskMode: 'afterEvaluate'` — that isolates
 * queues and breaks cross-context await.
 */
export async function runCodemode(rawCode, asgrep, options = {}) {
    const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    const wall0 = Date.now();
    if (rawCode.length > MAX_CODE_CHARS) {
        return resultErr(`code exceeds ${MAX_CODE_CHARS} characters`, [], rawCode.slice(0, 200), wall0, options.stats);
    }
    const logs = [];
    const pushLog = (...args) => {
        logs.push(args.map((a) => (typeof a === "string" ? a : safeJson(a))).join(" "));
    };
    const api = Object.freeze({
        search: asgrep.search,
        semantic: asgrep.semantic,
        chain: asgrep.chain,
        defs: asgrep.defs,
        callers: asgrep.callers,
        imports: asgrep.imports,
        indexStatus: asgrep.indexStatus,
        indexRepo: asgrep.indexRepo,
    });
    const context = vm.createContext({
        asgrep: api,
        console: { log: pushLog, info: pushLog, warn: pushLog, error: pushLog, debug: pushLog },
        Promise,
        JSON,
        Array,
        Object,
        Map,
        Set,
        Math,
        Number,
        String,
        Boolean,
        Date,
        RegExp,
        Error,
        TypeError,
        RangeError,
        parseInt,
        parseFloat,
        isNaN,
        isFinite,
        undefined,
    });
    const code = normalizeCode(rawCode);
    let script;
    try {
        script = new vm.Script(code, { filename: "asgrep-codemode.js" });
    }
    catch (cause) {
        return resultErr(cause instanceof Error ? cause.message : String(cause), logs, code, wall0, options.stats);
    }
    try {
        const produced = script.runInContext(context, { displayErrors: true });
        const value = await Promise.race([
            Promise.resolve(produced),
            new Promise((_, reject) => {
                setTimeout(() => reject(new Error(`codemode timeout after ${timeoutMs}ms`)), timeoutMs);
            }),
        ]);
        return resultOk(cloneOut(value), logs, code, wall0, options.stats);
    }
    catch (cause) {
        return resultErr(cause instanceof Error ? cause.message : String(cause), logs, code, wall0, options.stats);
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
function safeJson(value) {
    try {
        return JSON.stringify(value);
    }
    catch {
        return String(value);
    }
}
function cloneOut(value) {
    if (value === undefined)
        return undefined;
    try {
        return structuredClone(value);
    }
    catch {
        try {
            return JSON.parse(JSON.stringify(value));
        }
        catch {
            return value;
        }
    }
}
