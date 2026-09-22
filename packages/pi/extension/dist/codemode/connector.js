import { isAbsolute } from "node:path";
import { argvFor, asEnvelope, createCodemodeDispatcher, } from "./dispatch.js";
import { editFilesFallback, readWindowsFallback } from "./fallback.js";
import { coerceHostArgs } from "./guest-api.js";
import { defined } from "./types.js";
const DEFAULT_LIMIT = 8;
/** Native tools the extension serves itself when the launcher predates them. */
const NATIVE_FALLBACK_TOOLS = new Set(["read", "edit", "find"]);
/** Stable unknown-tool prefix, byte-identical in every native generation. */
function isUnknownToolError(cause) {
    const message = cause instanceof Error ? cause.message : String(cause);
    return message.includes("unknown tool:");
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
 * Host-side connector: typed methods the Code Mode program calls.
 *
 * Same-tick calls (Promise.all) are coalesced by CodemodeDispatcher so N
 * lookups share sticky serve / one warm batch process when available.
 */
export function createAsgrepConnector(host, context, options = {}) {
    const dispatcher = createCodemodeDispatcher(host);
    // One index per checkout: when the caller's cwd is a subdirectory of the
    // checkout that owns the index, path arguments are rebased onto that checkout
    // root and searches are scoped to the subdirectory. Callers never see a
    // second `.asgrep` grow inside the tree.
    const scope = options.scope && options.scope !== "."
        ? options.scope.replace(/^(?:\.\/)+/u, "").replace(/\/+$/u, "")
        : undefined;
    const rebasePath = (path) => {
        if (!scope || isAbsolute(path))
            return path;
        const clean = path.replace(/^(?:\.\/)+/u, "");
        if (clean === scope || clean.startsWith(`${scope}/`))
            return clean;
        return clean === "" || clean === "." ? scope : `${scope}/${clean}`;
    };
    const rebaseReadSpec = (value) => {
        // Refs emitted by search/read are already checkout-relative. Path-form
        // requests may be cwd-relative, including objects inside refs[].
        if (!value || typeof value !== "object" || Array.isArray(value))
            return value;
        const spec = value;
        if (typeof spec.ref === "string")
            return spec;
        return { ...spec,
            ...(typeof spec.path === "string" ? { path: rebasePath(spec.path) } : {}),
            ...(typeof spec.file === "string" ? { file: rebasePath(spec.file) } : {}),
        };
    };
    /** Directories below the checkout combine with the anchor scope, never replace it. */
    const withScope = (input) => {
        if (!scope)
            return input;
        const nested = input.in ?? input.fileFilter ?? input.file_filter;
        const combined = typeof nested === "string" && nested.trim() ? `${scope}/${nested.replace(/^(?:\.\/)+/u, "")}` : scope;
        return { ...input, in: combined };
    };
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
    // Unknown-tool fallback engages once per tool: after the first miss the
    // native attempt is skipped for this bundle (a backend cannot gain tools
    // mid-session). Any other error still propagates untouched, unmemoized.
    const nativeMissing = new Set();
    const fallbackCall = async (tool, args, signal) => {
        if (tool === "find")
            return host.run(argvFor("find", args), context, signal === undefined ? {} : { signal });
        const value = tool === "read"
            ? await readWindowsFallback(context.cwd, args, signal)
            : await editFilesFallback(context.cwd, args, signal);
        return asEnvelope(value, tool);
    };
    const call = async (tool, args, signal) => {
        const opts = callOptions(signal);
        if (tool === "read" && options.localReads)
            return fallbackCall(tool, args, opts.signal);
        if (NATIVE_FALLBACK_TOOLS.has(tool) && nativeMissing.has(tool))
            return fallbackCall(tool, args, opts.signal);
        try {
            return await dispatcher.host.call(tool, args, context, opts);
        }
        catch (cause) {
            // No memo check here: concurrent same-tool misses must all fall back —
            // the memo only skips future native attempts, never a fresh miss.
            if (NATIVE_FALLBACK_TOOLS.has(tool) && isUnknownToolError(cause)) {
                nativeMissing.add(tool);
                return fallbackCall(tool, args, opts.signal);
            }
            throw cause;
        }
    };
    const searchPayload = (method, input) => {
        const scoped = coerceHostArgs(method, withScope({ ...input }));
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
    const symbolCall = (tool, input, signal) => {
        const args = coerceHostArgs(tool, input);
        const key = tool === "imports" ? "module" : "symbol";
        if (scope || ["in", "fileFilter", "file_filter", "lang"].some(key => typeof args[key] === "string" && args[key].trim())) {
            return call("search", searchPayload("search", { ...args, query: `${tool}:${args[key] ?? ""}` }), signal);
        }
        return call(tool, defined({ [key]: args[key], limit: clampLimit(args.limit),
            excerpt_lines: clampExcerpt(args.excerptLines) }), signal);
    };
    // Bound function properties (not methods) so vm call sites cannot lose `this`.
    const asgrep = {
        search: (input, callOptions) => call("search", searchPayload("search", input), callOptions?.signal),
        find: (input, callOptions) => call("find", searchPayload("find", input), callOptions?.signal),
        read: (input, callOptions) => call("read", defined({
            path: typeof input.path === "string" ? rebasePath(input.path) : input.path,
            start: input.start,
            end: input.end,
            ref: input.ref,
            refs: Array.isArray(input.refs) ? input.refs.map(rebaseReadSpec) : input.refs,
            context_lines: input.contextLines,
            max_chars: input.maxChars,
        }), callOptions?.signal),
        edit: (input, callOptions) => {
            // Multi-edit wire contract: every entry carries its own path; the
            // top-level path is the default for entries that omit it.
            const edits = input.edits?.map((entry) => ({
                ...(typeof input.path === "string" ? { path: rebasePath(input.path) } : {}),
                ...entry,
                ...(typeof entry.path === "string" ? { path: rebasePath(entry.path) } : {}),
            }));
            return call("edit", defined({
                path: typeof input.path === "string" ? rebasePath(input.path) : input.path,
                oldText: input.oldText,
                newText: input.newText,
                edits,
            }), callOptions?.signal);
        },
        semantic: (input, callOptions) => call("semantic", searchPayload("semantic", input), callOptions?.signal),
        chain: (input, callOptions) => call("chain", { query: input.query, limit: clampLimit(input.limit), top_n: 20 }, callOptions?.signal),
        defs: (input, callOptions) => symbolCall("defs", input, callOptions?.signal),
        callers: (input, callOptions) => symbolCall("callers", input, callOptions?.signal),
        imports: (input, callOptions) => symbolCall("imports", input, callOptions?.signal),
        indexStatus: (callOptions) => call("index_status", {}, callOptions?.signal),
        indexRepo: (input = {}, callOptions) => call("index_repo", { force: input.force === true }, callOptions?.signal),
        catalogSearch: (input, callOptions) => call("catalog_search", { query: input.query }, callOptions?.signal),
        catalogDescribe: (input, callOptions) => call("catalog_describe", { name: input.name }, callOptions?.signal),
        doctor: async (perCall) => {
            const opts = callOptions(perCall?.signal);
            opts.signal?.throwIfAborted();
            return host.run(["doctor", ".", "--json"], context, opts);
        },
    };
    return {
        asgrep,
        stats: dispatcher.stats,
        trace: dispatcher.trace,
        resetStats: dispatcher.resetStats,
    };
}
