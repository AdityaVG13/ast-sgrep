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
    // Bound function properties (not methods) so vm call sites cannot lose `this`.
    const asgrep = {
        search: (input, callOptions) => call("search", {
            query: input.query,
            limit: clampLimit(input.limit),
            excerpt_lines: clampExcerpt(input.excerptLines),
            format: input.format === "agent" ? "agent" : "capsule",
        }, callOptions?.signal),
        semantic: (input, callOptions) => call("semantic", {
            query: input.query,
            limit: clampLimit(input.limit),
            excerpt_lines: clampExcerpt(input.excerptLines),
            format: input.format === "agent" ? "agent" : "capsule",
        }, callOptions?.signal),
        chain: (input, callOptions) => call("chain", {
            query: input.query,
            limit: clampLimit(input.limit),
            top_n: 20,
        }, callOptions?.signal),
        defs: (input, callOptions) => call("defs", {
            symbol: input.symbol,
            limit: clampLimit(input.limit),
            excerpt_lines: clampExcerpt(input.excerptLines),
        }, callOptions?.signal),
        callers: (input, callOptions) => call("callers", {
            symbol: input.symbol,
            limit: clampLimit(input.limit),
            excerpt_lines: clampExcerpt(input.excerptLines),
        }, callOptions?.signal),
        imports: (input, callOptions) => call("imports", {
            module: input.module,
            limit: clampLimit(input.limit),
            excerpt_lines: clampExcerpt(input.excerptLines),
        }, callOptions?.signal),
        indexStatus: (callOptions) => call("index_status", {}, callOptions?.signal),
        indexRepo: (input = {}, callOptions) => call("index_repo", { force: input.force === true }, callOptions?.signal),
        catalogSearch: (input, callOptions) => call("catalog_search", { query: input.query }, callOptions?.signal),
        catalogDescribe: (input, callOptions) => call("catalog_describe", { name: input.name }, callOptions?.signal),
    };
    return {
        asgrep,
        stats: dispatcher.stats,
        resetStats: dispatcher.resetStats,
    };
}
