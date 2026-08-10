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
 * Run model-generated JavaScript against the typed `asgrep` connector.
 *
 * Trust model (OpenCode-style): Code Mode is an orchestration pattern, not an OS
 * jail. The Pi package already runs with the installing user's privileges. Authority
 * is the explicit `asgrep.*` surface passed into the program — same idea as
 * OpenCode CodeMode exposing only host-supplied tools. We intentionally do **not**
 * use `node:vm` / isolates: same-realm `AsyncFunction` is faster and enough for
 * composition (`Promise.all`, filter, shape).
 */
export async function runCodemode(rawCode, asgrep, options = {}) {
    const timeoutMs = Math.max(1, options.timeoutMs ?? DEFAULT_TIMEOUT_MS);
    const wall0 = Date.now();
    if (rawCode.length > MAX_CODE_CHARS) {
        return resultErr(`code exceeds ${MAX_CODE_CHARS} characters`, [], rawCode.slice(0, 200), wall0, options.stats);
    }
    if (options.signal?.aborted) {
        return resultErr("codemode aborted", [], rawCode.slice(0, 200), wall0, options.stats);
    }
    const logs = [];
    const formatLog = (value) => {
        if (typeof value === "string")
            return value;
        try {
            return JSON.stringify(value);
        }
        catch {
            return String(value);
        }
    };
    const consoleApi = Object.freeze({
        log: (...args) => {
            logs.push(args.map(formatLog).join(" "));
        },
        info: (...args) => {
            logs.push(args.map(formatLog).join(" "));
        },
        warn: (...args) => {
            logs.push(args.map(formatLog).join(" "));
        },
        error: (...args) => {
            logs.push(args.map(formatLog).join(" "));
        },
        debug: (...args) => {
            logs.push(args.map(formatLog).join(" "));
        },
    });
    const methods = [
        "search",
        "semantic",
        "chain",
        "defs",
        "callers",
        "imports",
        "indexStatus",
        "indexRepo",
        "catalogSearch",
        "catalogDescribe",
    ];
    const api = Object.create(null);
    for (const method of methods) {
        const fn = asgrep[method].bind(asgrep);
        api[method] = (args = {}) => fn(args);
    }
    Object.freeze(api);
    const code = normalizeCode(rawCode);
    // Same-realm AsyncFunction: faster than node:vm; no microtask-queue isolation issues.
    const AsyncFunction = Object.getPrototypeOf(async function () { }).constructor;
    let run;
    try {
        run = new AsyncFunction("asgrep", "console", `"use strict";\nreturn await (${code});`);
    }
    catch (cause) {
        return resultErr(cause instanceof Error ? cause.message : String(cause), logs, code, wall0, options.stats);
    }
    let timer;
    let onAbort;
    try {
        const races = [
            Promise.resolve(run(api, consoleApi)),
            new Promise((_, reject) => {
                timer = setTimeout(() => reject(new Error(`codemode timeout after ${timeoutMs}ms`)), timeoutMs);
            }),
        ];
        if (options.signal) {
            races.push(new Promise((_, reject) => {
                onAbort = () => reject(new Error("codemode aborted"));
                options.signal.addEventListener("abort", onAbort, { once: true });
            }));
        }
        const value = await Promise.race(races);
        return resultOk(cloneOut(value), logs, code, wall0, options.stats);
    }
    catch (cause) {
        return resultErr(cause instanceof Error ? cause.message : String(cause), logs, code, wall0, options.stats);
    }
    finally {
        if (timer)
            clearTimeout(timer);
        if (onAbort)
            options.signal?.removeEventListener("abort", onAbort);
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
