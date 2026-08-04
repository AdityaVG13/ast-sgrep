import { createCodemodeDispatcher } from "./dispatch.js";
const DEFAULT_LIMIT = 8;
function capsuleArgs(limit, excerptLines) {
    return ["--json", "--format", "agent-capsule", "--limit", String(limit), "--excerpt-lines", String(excerptLines)];
}
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
 * Host-side connector: typed methods the sandbox calls.
 *
 * Same-tick calls (Promise.all) are coalesced by CodemodeDispatcher so N
 * lookups share one warm `codemode-batch` process when available, otherwise
 * overlapped CLI spawns.
 */
export function createAsgrepConnector(host, context, options = {}) {
    const dispatcher = createCodemodeDispatcher(host);
    const run = (args) => dispatcher.host.run(args, context, options.signal ? { signal: options.signal } : {});
    const searchLike = (query, limit, excerptLines) => run([...capsuleArgs(clampLimit(limit), clampExcerpt(excerptLines)), query, "."]);
    // Bound function properties (not methods) so vm call sites cannot lose `this`.
    const asgrep = {
        search: (input) => searchLike(input.query, input.limit, input.excerptLines),
        semantic: (input) => run([
            "semantic",
            input.query,
            ".",
            ...capsuleArgs(clampLimit(input.limit), clampExcerpt(input.excerptLines)),
        ]),
        chain: (input) => run([
            "chain",
            input.query,
            ".",
            ...capsuleArgs(clampLimit(input.limit), clampExcerpt(input.excerptLines)),
        ]),
        defs: (input) => searchLike(`defs: ${input.symbol}`, input.limit, input.excerptLines),
        callers: (input) => searchLike(`callers: ${input.symbol}`, input.limit, input.excerptLines),
        imports: (input) => searchLike(`imports: ${input.module}`, input.limit, input.excerptLines),
        indexStatus: () => run(["status", ".", "--json"]),
        indexRepo: (input = {}) => {
            const command = input.force === true ? "reindex" : "index";
            return run([command, ".", "--json"]);
        },
    };
    return {
        asgrep,
        stats: dispatcher.stats,
        resetStats: dispatcher.resetStats,
    };
}
