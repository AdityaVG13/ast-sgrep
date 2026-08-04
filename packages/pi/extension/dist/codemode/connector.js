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
 * Host-side connector: typed methods the sandbox calls. Each method maps to one
 * native CLI invocation. Independent methods may run concurrently via Promise.all.
 */
export function createAsgrepConnector(host, context, options = {}) {
    const run = (args) => host.run(args, context, options.signal ? { signal: options.signal } : {});
    const searchLike = (query, limit, excerptLines) => run([...capsuleArgs(clampLimit(limit), clampExcerpt(excerptLines)), query, "."]);
    return {
        search(input) {
            return searchLike(input.query, input.limit, input.excerptLines);
        },
        semantic(input) {
            return run([
                "semantic",
                input.query,
                ".",
                ...capsuleArgs(clampLimit(input.limit), clampExcerpt(input.excerptLines)),
            ]);
        },
        chain(input) {
            return run([
                "chain",
                input.query,
                ".",
                ...capsuleArgs(clampLimit(input.limit), clampExcerpt(input.excerptLines)),
            ]);
        },
        defs(input) {
            return searchLike(`defs: ${input.symbol}`, input.limit, input.excerptLines);
        },
        callers(input) {
            return searchLike(`callers: ${input.symbol}`, input.limit, input.excerptLines);
        },
        imports(input) {
            return searchLike(`imports: ${input.module}`, input.limit, input.excerptLines);
        },
        indexStatus() {
            return run(["status", ".", "--json"]);
        },
        indexRepo(input = {}) {
            const command = input.force === true ? "reindex" : "index";
            return run([command, ".", "--json"]);
        },
    };
}
