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
    const runOptions = options.signal ? { signal: options.signal } : {};
    const call = (tool, args) => dispatcher.host.call(tool, args, context, runOptions);
    // Bound function properties (not methods) so vm call sites cannot lose `this`.
    const asgrep = {
        search: (input) => call("search", {
            query: input.query,
            limit: clampLimit(input.limit),
            excerpt_lines: clampExcerpt(input.excerptLines),
            format: input.format === "agent" ? "agent" : "capsule",
        }),
        semantic: (input) => call("semantic", {
            query: input.query,
            limit: clampLimit(input.limit),
            excerpt_lines: clampExcerpt(input.excerptLines),
            format: input.format === "agent" ? "agent" : "capsule",
        }),
        chain: (input) => call("chain", {
            query: input.query,
            limit: clampLimit(input.limit),
            top_n: 20,
        }),
        defs: (input) => call("defs", {
            symbol: input.symbol,
            limit: clampLimit(input.limit),
            excerpt_lines: clampExcerpt(input.excerptLines),
        }),
        callers: (input) => call("callers", {
            symbol: input.symbol,
            limit: clampLimit(input.limit),
            excerpt_lines: clampExcerpt(input.excerptLines),
        }),
        imports: (input) => call("imports", {
            module: input.module,
            limit: clampLimit(input.limit),
            excerpt_lines: clampExcerpt(input.excerptLines),
        }),
        indexStatus: () => call("index_status", {}),
        indexRepo: (input = {}) => call("index_repo", { force: input.force === true }),
        catalogSearch: (input) => call("catalog_search", { query: input.query }),
        catalogDescribe: (input) => call("catalog_describe", { name: input.name }),
    };
    return {
        asgrep,
        stats: dispatcher.stats,
        resetStats: dispatcher.resetStats,
    };
}
