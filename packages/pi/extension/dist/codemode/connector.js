import { coerceHostArgs } from "./guest-api.js";
import { createCodemodeDispatcher, } from "./dispatch.js";
const DEFAULT_LIMIT = 8;
function clampLimit(limit) {
    if (limit === undefined)
        return DEFAULT_LIMIT;
    return Math.min(100, Math.max(1, Math.trunc(limit)));
}
function clampExcerpt(excerptLines) {
    if (excerptLines === undefined)
        return 0;
    return Math.min(100, Math.max(0, Math.trunc(excerptLines)));
}
/**
 * Host-side connector: typed methods the Code Mode program calls.
 *
 * Same-tick calls (Promise.all) are coalesced by CodemodeDispatcher so N
 * lookups share sticky serve / one warm batch process when available.
 */
export function createAsgrepConnector(host, context, options = {}) {
    const dispatcher = createCodemodeDispatcher(host);
    const combinedSignals = new WeakMap();
    const callOptions = (signal) => {
        if (!options.signal)
            return signal ? { signal } : {};
        if (!signal || signal === options.signal)
            return { signal: options.signal };
        let combined = combinedSignals.get(signal);
        if (!combined) {
            combined = AbortSignal.any([options.signal, signal]);
            combinedSignals.set(signal, combined);
        }
        return { signal: combined };
    };
    const call = (tool, args, signal) => dispatcher.host.call(tool, args, context, callOptions(signal));
    const searchPayload = (method, input) => {
        const scoped = coerceHostArgs(method, { ...input });
        const payload = {
            query: scoped.query,
            limit: clampLimit(input.limit),
            excerpt_lines: clampExcerpt(input.excerptLines),
            format: input.format === "agent" ? "agent" : "capsule",
        };
        if (typeof scoped.lang === "string" && scoped.lang.trim())
            payload.lang = scoped.lang.trim();
        return payload;
    };
    // Bound function properties (not methods) so vm call sites cannot lose `this`.
    const asgrep = {
        search: (input, callOptions) => call("search", searchPayload("search", input), callOptions?.signal),
        find: (input, callOptions) => call("find", searchPayload("find", input), callOptions?.signal),
        read: (input, callOptions) => call("read", {
            ...(typeof input.path === "string" ? { path: input.path } : {}),
            ...(input.start !== undefined ? { start: input.start } : {}),
            ...(input.end !== undefined ? { end: input.end } : {}),
            ...(typeof input.ref === "string" ? { ref: input.ref } : {}),
            ...(input.refs !== undefined ? { refs: input.refs } : {}),
            ...(input.contextLines !== undefined ? { context_lines: input.contextLines } : {}),
            ...(input.maxChars !== undefined ? { max_chars: input.maxChars } : {}),
        }, callOptions?.signal),
        edit: (input, callOptions) => call("edit", {
            ...(typeof input.path === "string" ? { path: input.path } : {}),
            ...(typeof input.oldText === "string" ? { oldText: input.oldText } : {}),
            ...(typeof input.newText === "string" ? { newText: input.newText } : {}),
            ...(input.edits !== undefined ? { edits: input.edits } : {}),
        }, callOptions?.signal),
        semantic: (input, callOptions) => call("semantic", searchPayload("semantic", input), callOptions?.signal),
        chain: (input, callOptions) => call("chain", {
            query: input.query,
            limit: clampLimit(input.limit),
            top_n: 20,
        }, callOptions?.signal),
        defs: (input, callOptions) => {
            const scoped = coerceHostArgs("defs", { ...input });
            return call("defs", {
                symbol: scoped.symbol,
                limit: clampLimit(input.limit),
                excerpt_lines: clampExcerpt(input.excerptLines),
            }, callOptions?.signal);
        },
        callers: (input, callOptions) => {
            const scoped = coerceHostArgs("callers", { ...input });
            return call("callers", {
                symbol: scoped.symbol,
                limit: clampLimit(input.limit),
                excerpt_lines: clampExcerpt(input.excerptLines),
            }, callOptions?.signal);
        },
        imports: (input, callOptions) => call("imports", {
            module: input.module,
            limit: clampLimit(input.limit),
            excerpt_lines: clampExcerpt(input.excerptLines),
        }, callOptions?.signal),
        indexStatus: (callOptions) => call("index_status", {}, callOptions?.signal),
        indexRepo: (input = {}, callOptions) => call("index_repo", { force: input.force === true }, callOptions?.signal),
        catalogSearch: (input, callOptions) => call("catalog_search", { query: input.query }, callOptions?.signal),
        catalogDescribe: (input, callOptions) => call("catalog_describe", { name: input.name }, callOptions?.signal),
        doctor: (callOptions) => host.run(["doctor", ".", "--json"], context, callOptions),
    };
    return {
        asgrep,
        stats: dispatcher.stats,
        trace: dispatcher.trace,
        resetStats: dispatcher.resetStats,
    };
}
