import { isAbsolute } from "node:path";
import { Type } from "typebox";
import { createAsgrepConnector, runCodemode, runNativeBatch, runBatchViaStdin, CODEMODE_TYPES_FOR_MODEL, NativeSessionPool, argvFor, asEnvelope, applyQueryScope, warmCodemodeSandbox, resetCodemodeSandboxForTests, isClosedWorkerError, } from "../codemode/index.js";
import { AstSgrepRuntime, FreshnessCoordinator, RuntimeError } from "../runtime/runtime.js";
import { ASGREP_PROMPT_GUIDELINES, ASGREP_PROMPT_SNIPPET, formatCodemodeCall, formatCodemodeResult, formatEditCall, formatIndexCall, formatReadCall, formatSearchCall, formatStatusCall, presentText, } from "../ui/present.js";
import { renderAsgrepResult } from "../ui/card.js";
import { bounded, errorDetails, failure, isFreshnessTimeout, extractInPath, report, success, } from "./results.js";
export const DEFAULT_LIMIT = 8;
const MAX_LIMIT = 100;
const MAX_EXCERPT_LINES = 100;
const searchParameters = Type.Object({
    query: Type.String({ minLength: 1, maxLength: 4_096, description: "Natural-language query, symbol, or structural pattern" }),
    mode: Type.Optional(Type.Union([
        Type.Literal("natural"),
        Type.Literal("pattern"),
        Type.Literal("defs"),
        Type.Literal("callers"),
        Type.Literal("chain"),
        Type.Literal("semantic"),
        Type.Literal("word"),
        Type.Literal("literal"),
        Type.Literal("regex"),
        Type.Literal("imports"),
    ], { default: "natural", description: "Search strategy (CLI-aligned modes)" })),
    limit: Type.Optional(Type.Integer({ minimum: 1, maximum: MAX_LIMIT, default: DEFAULT_LIMIT })),
    excerptLines: Type.Optional(Type.Integer({ minimum: 0, maximum: MAX_EXCERPT_LINES, default: 0, description: "Opt in to excerpt body lines" })),
    in: Type.Optional(Type.String({ minLength: 1, maxLength: 512, description: "Directory or glob to bound the search (in:path)" })),
    lang: Type.Optional(Type.String({ minLength: 1, maxLength: 32, description: "Language id or extension (rs, ts, py)" })),
    fileFilter: Type.Optional(Type.String({ minLength: 1, maxLength: 512, description: "Repository-relative glob; alias of in" })),
}, { additionalProperties: false });
const indexParameters = Type.Object({
    force: Type.Optional(Type.Boolean({ default: false, description: "Rebuild the index from scratch" })),
}, { additionalProperties: false });
const statusParameters = Type.Object({}, { additionalProperties: false });
const editParameters = Type.Object({
    path: Type.Optional(Type.String({ minLength: 1, maxLength: 512, description: "Repository-relative file to edit" })),
    oldText: Type.Optional(Type.String({ description: "Exact text to replace — must match exactly once in the file" })),
    newText: Type.Optional(Type.String({ description: "Replacement text" })),
    edits: Type.Optional(Type.Array(Type.Object({
        path: Type.Optional(Type.String({ minLength: 1, maxLength: 512 })),
        oldText: Type.String({ minLength: 1 }),
        newText: Type.String(),
    }), { maxItems: 64, description: "Multi-edit entries; top-level path is the default for entries that omit it" })),
}, { additionalProperties: false });
const readParameters = Type.Object({
    path: Type.Optional(Type.String({ minLength: 1, maxLength: 512, description: "Repository-relative file path" })),
    ref: Type.Optional(Type.String({ minLength: 1, description: "Hit ref (path#L12-L40) from a search result" })),
    refs: Type.Optional(Type.Array(Type.String({ minLength: 1 }), { maxItems: 24, description: "Multiple refs to read in one call" })),
    start: Type.Optional(Type.Integer({ minimum: 1 })),
    end: Type.Optional(Type.Integer({ minimum: 1 })),
    contextLines: Type.Optional(Type.Integer({ minimum: 0, maximum: 40 })),
    maxChars: Type.Optional(Type.Integer({ minimum: 64, maximum: 64_000 })),
}, { additionalProperties: false });
const codemodeParameters = Type.Object({
    code: Type.String({
        minLength: 1,
        maxLength: 32_000,
        description: "JavaScript: async () => { ... } or a bare body with return. Call asgrep.search(\"query\"), asgrep.defs(\"Symbol\"). Prefer Promise.all. Return only the shaped final value.",
    }),
    timeoutMs: Type.Optional(Type.Integer({ minimum: 1_000, maximum: 120_000, description: "Hard timeout in ms (default 30000)" })),
}, { additionalProperties: false });
function queryForMode(query, mode) {
    if (mode === "pattern" || mode === "defs" || mode === "callers" || mode === "word" || mode === "literal" || mode === "regex" || mode === "imports") {
        return `${mode}: ${query}`;
    }
    return query;
}
function searchArgs(params) {
    const mode = params.mode ?? "natural";
    const query = queryForMode(scopedSearchQuery(params), mode);
    const output = withSearchLang(["--json", "--format", "agent-capsule", "--limit", String(params.limit ?? DEFAULT_LIMIT), "--excerpt-lines", String(params.excerptLines ?? 0)], params.lang);
    return mode === "chain" || mode === "semantic"
        ? [mode, query, ".", ...output]
        : [...output, query, "."];
}
function scopedSearchQuery(params) {
    return applyQueryScope(params.query, {
        ...(typeof params.in === "string" ? { in: params.in } : {}),
        ...(typeof params.fileFilter === "string" ? { fileFilter: params.fileFilter } : {}),
    }) ?? params.query;
}
function withSearchLang(argv, lang) {
    const trimmed = lang?.trim();
    return trimmed ? ["--lang", trimmed, ...argv] : argv;
}
export function registerAstSgrepTools(pi, runtime = new AstSgrepRuntime(pi), freshness = runtime instanceof AstSgrepRuntime
    ? new FreshnessCoordinator({ refreshIntervalMs: runtime.config.refreshIntervalMs })
    : new FreshnessCoordinator()) {
    const pool = new NativeSessionPool();
    let poolConfigured = false;
    // Prefer a registration-local pool so tests / multi-agent hosts do not share
    // sticky state. sharedNativePool remains for advanced single-session reuse.
    const ensurePool = () => {
        if (poolConfigured)
            return;
        try {
            const env = runtime.nativeEnv?.() ?? { NO_COLOR: "1" };
            let binary;
            try {
                binary = runtime.resolveBinaryPath?.({ env });
            }
            catch {
                binary = undefined;
            }
            const opts = { env };
            if (binary)
                opts.binary = binary;
            if (runtime.config?.timeoutMs !== undefined)
                opts.timeoutMs = runtime.config.timeoutMs;
            if (runtime.config?.maxOutputBytes !== undefined)
                opts.maxOutputBytes = runtime.config.maxOutputBytes;
            if (typeof env.ASGREP_NO_EMBED === "string") {
                opts.useEmbed = env.ASGREP_NO_EMBED !== "1" && env.ASGREP_NO_EMBED !== "true";
            }
            if (typeof env.ASGREP_INDEX_PATH === "string")
                opts.indexPath = env.ASGREP_INDEX_PATH;
            pool.configure(opts);
        }
        catch {
            pool.configure({});
        }
        poolConfigured = true;
    };
    const resolveRoot = async (cwd) => runtime.resolveRoot ? await runtime.resolveRoot({ cwd }) : cwd;
    const probeCli = (options = {}) => {
        // Test fixtures inject `run` without a resolver; production always has resolveBinaryPath.
        if (typeof runtime.resolveBinaryPath !== "function")
            return { kind: "cli" };
        try {
            const base = runtime.nativeEnv?.() ?? {};
            if (options.env) {
                runtime.resolveBinaryPath({ env: { ...base, ...options.env } });
            }
            else {
                runtime.resolveBinaryPath({ env: base });
            }
            return { kind: "cli" };
        }
        catch (cause) {
            return {
                kind: "unavailable",
                cause: cause instanceof Error ? cause.message : String(cause),
            };
        }
    };
    const requireBackend = (availability, context) => {
        if (availability.kind !== "unavailable")
            return;
        throw new RuntimeError("BACKEND_UNAVAILABLE", "ast-sgrep backend unavailable (no NAPI session and no CLI binary)", {
            backend: "unavailable",
            // Agent-facing mirrors of the closed unavailable variant (not an open product).
            napi: false,
            cli: false,
            cwd: context.cwd,
            hint: "Install @ast-sgrep/<platform> or run npm run build:native in packages/pi/extension",
            ...(availability.cause ? { cause: availability.cause } : {}),
        });
    };
    const runCli = async (args, context, options = {}) => {
        requireBackend(probeCli(options), context);
        return runtime.run(args, context, options);
    };
    /** Typed sticky call that degrades to null when no backend is available or
     * the session is crash-looping — the caller's CLI fallback owns the error
     * fidelity when the binary itself is also broken. pool.call owns the
     * respawn-on-closed retry; we only translate its outcomes. */
    const callSticky = async (root, tool, args, options = {}) => {
        try {
            return await pool.call(root, tool, args, options.signal ? { signal: options.signal } : {});
        }
        catch (cause) {
            // Aborted: return null so the CLI fallback surfaces the cancellation with
            // its own semantics (runCli -> runtime.run maps abort to CANCELLED).
            if (options.signal?.aborted)
                return null;
            const message = cause instanceof Error ? cause.message : String(cause);
            if (isClosedWorkerError(cause) || message.includes("backend unavailable"))
                return null;
            throw cause;
        }
    };
    const nativeCall = async (tool, args, context, options = {}) => {
        ensurePool();
        const root = await resolveRoot(context.cwd);
        const sticky = await callSticky(root, tool, args, options);
        if (sticky)
            return asEnvelope(sticky);
        // Cold CLI only when a real binary resolves -- never remap missing natives to BINARY_RESOLUTION_FAILED.
        return runCli(argvFor(tool, args), context, options);
    };
    // Freshness + tools share the same warm in-process Searcher as Code Mode.
    const warmRuntime = {
        run: (args, context, options) => runtime.run(args, context, options),
        resolveRoot: (context) => resolveRoot(context.cwd),
        nativeCall,
    };
    // Optional runtime capabilities pass through when present — bound, since
    // they are class methods whose private fields live on the runtime instance.
    for (const key of ["watchExternalChanges", "resolveIndexPath", "inspectIndexCompatibility"]) {
        const member = runtime[key];
        if (member !== undefined) {
            warmRuntime[key] =
                typeof member === "function" ? member.bind(runtime) : member;
        }
    }
    if (runtime.rebuildIncompatibleIndex) {
        warmRuntime.rebuildIncompatibleIndex = async (context, options) => {
            const root = await resolveRoot(context.cwd);
            await pool.invalidate(root);
            return runtime.rebuildIncompatibleIndex(context, options);
        };
    }
    /** Freshness gate shared by the one-shot tools: ensureFresh with a bounded
     * timeout fallback, or a scoped-path index when the query carries in:/fileFilter. */
    const freshRoot = async (cwd, signal, scopedPath) => {
        const options = signal ? { signal } : {};
        if (scopedPath) {
            const root = await resolveRoot(cwd);
            try {
                await nativeCall("index_repo", { paths: [scopedPath] }, { cwd }, options);
            }
            catch (cause) {
                if (!isFreshnessTimeout(cause, signal))
                    throw cause;
                await pool.invalidate(root).catch(() => undefined);
            }
            return root;
        }
        try {
            return await freshness.ensureFresh(warmRuntime, { cwd }, options);
        }
        catch (cause) {
            if (!isFreshnessTimeout(cause, signal))
                throw cause;
            const root = await resolveRoot(cwd);
            await pool.invalidate(root).catch(() => undefined);
            return root;
        }
    };
    /** Typed sticky call first, argv fallback when no session — the shape every
     * one-shot tool shares. */
    const machineCall = async (root, tool, stickyArgs, argv, context, options) => (await callSticky(root, tool, stickyArgs, options)) ?? await runCli(argv, context, options);
    /** Native launch env + resolved binary for the codemode batch host. */
    const nativeLaunch = () => {
        const env = runtime.nativeEnv?.() ?? { NO_COLOR: "1" };
        let binary = null;
        try {
            binary = runtime.resolveBinaryPath?.({ env }) ?? null;
        }
        catch {
            binary = null;
        }
        return { env, binary };
    };
    /** Host surface for codemode: argv run + optional sticky + stdin batch. */
    const buildBatchHost = (sticky, env, binary) => {
        const host = {
            run: (args, context, runOptions) => runtime.run(args, context, runOptions ?? {}),
            sticky,
        };
        if (binary) {
            host.runBatch = (calls, context, runOptions) => runNativeBatch((a, c, o) => runtime.run(a, c, o ?? {}), calls, context, runOptions, (body, c, o) => {
                const stdinOpts = { binary, cwd: c.cwd, body, env };
                if (o?.signal)
                    stdinOpts.signal = o.signal;
                if (runtime.config?.timeoutMs !== undefined)
                    stdinOpts.timeoutMs = runtime.config.timeoutMs;
                if (runtime.config?.maxOutputBytes !== undefined)
                    stdinOpts.maxOutputBytes = runtime.config.maxOutputBytes;
                return runBatchViaStdin(stdinOpts);
            });
        }
        return host;
    };
    let stopWorkspaceEvents;
    function watchWorkspaceChanges() {
        stopWorkspaceEvents ??= pi.events?.on("workspace:changed", (data) => {
            if (!data || typeof data !== "object")
                return;
            const event = data;
            if (event.version !== 1 || typeof event.cwd !== "string" || !isAbsolute(event.cwd))
                return;
            if (event.paths === null) {
                freshness.markRootDirty?.(event.cwd);
            }
            else if (Array.isArray(event.paths) && event.paths.every(p => typeof p === "string" && isAbsolute(p))) {
                for (const file of event.paths)
                    freshness.markAffectedPath(file, event.cwd);
            }
        });
    }
    watchWorkspaceChanges();
    pi.on("tool_result", (event, ctx) => {
        if (event.isError)
            return;
        if (event.toolName !== "write" && event.toolName !== "edit")
            return;
        const path = event.input.path;
        if (typeof path === "string")
            freshness.markAffectedPath(path, ctx.cwd);
    });
    pi.on("session_start", (_event, ctx) => {
        watchWorkspaceChanges();
        // Warm the in-process Searcher at session start so the first asgrep
        // search does not pay NAPI/SQLite open on the user's first lookup.
        void (async () => {
            try {
                ensurePool();
                const root = await resolveRoot(ctx.cwd);
                await Promise.all([pool.acquire(root), warmCodemodeSandbox()]);
            }
            catch {
                // Doctor reports backend errors; a failed warmup must not block the session.
            }
        })();
    });
    pi.on("session_shutdown", () => {
        stopWorkspaceEvents?.();
        stopWorkspaceEvents = undefined;
        freshness.shutdown?.();
        void pool.shutdown();
        void resetCodemodeSandboxForTests();
    });
    // Primary surface: Code Mode -- in-process NAPI (MCP-class), compose in JS.
    // Sibling to MCP: pick one surface; both link core, never each other.
    pi.registerTool({
        name: "asgrep",
        label: "asgrep",
        promptSnippet: ASGREP_PROMPT_SNIPPET,
        promptGuidelines: [...ASGREP_PROMPT_GUIDELINES],
        description: [
            "Primary code-search tool for this project. Call it whenever you need to find, trace, or understand code — do not wait for the user to mention asgrep.",
            "Write JavaScript that calls asgrep.search, asgrep.defs, asgrep.callers, asgrep.read, and asgrep.edit. Positional args work: search(\"auth\"), defs(\"Foo\"). Compose with await / Promise.all, filter in code, return only the shaped final value.",
            "Runs in-process (native addon) with a warm Searcher for the Pi session.",
            "",
            CODEMODE_TYPES_FOR_MODEL,
            "",
            "Example:",
            "async () => {",
            "  const seed = await asgrep.search('auth refresh', { limit: 5 });",
            "  const hit = seed.hits?.[0];",
            "  if (!hit?.symbol) return { seed, next: seed.suggested_next };",
            "  const [defs, window] = await Promise.all([",
            "    asgrep.defs(hit.symbol, { limit: 5 }),",
            "    asgrep.read({ refs: [hit.ref] }),",
            "  ]);",
            "  return { symbol: hit.symbol, defs: defs.hits, window };",
            "}",
        ].join("\n"),
        parameters: codemodeParameters,
        renderCall(args, theme, context) {
            return presentText(formatCodemodeCall(args.code, theme), context.lastComponent);
        },
        renderResult(result, options, theme, context) {
            return renderAsgrepResult(result, options, theme, context);
        },
        async execute(_toolCallId, params, signal, onUpdate, ctx) {
            report(onUpdate, "codemode", "started");
            try {
                const timeoutMs = typeof params.timeoutMs === "number"
                    ? params.timeoutMs
                    : runtime.config?.timeoutMs ?? 30_000;
                const deadline = Date.now() + timeoutMs;
                const timeoutSignal = AbortSignal.timeout(timeoutMs);
                const operationSignal = signal
                    ? AbortSignal.any([signal, timeoutSignal])
                    : timeoutSignal;
                const options = { signal: operationSignal };
                ensurePool();
                const root = await freshRoot(ctx.cwd, signal);
                const { env, binary } = nativeLaunch();
                // In-process NAPI first; CLI sticky only if addon missing.
                const sticky = await pool.acquire(root);
                const bundle = createAsgrepConnector(buildBatchHost(sticky, env, binary), { cwd: ctx.cwd }, options);
                bundle.resetStats();
                const codemodeOptions = { stats: bundle.stats };
                codemodeOptions.timeoutMs = Math.max(1, deadline - Date.now());
                codemodeOptions.signal = operationSignal;
                await warmCodemodeSandbox().catch(() => undefined);
                const outcome = await runCodemode(params.code, bundle.asgrep, codemodeOptions);
                report(onUpdate, "codemode", "completed");
                if (!outcome.ok) {
                    return {
                        content: [{ type: "text", text: bounded(`codemode failed: ${outcome.error}`) }],
                        details: {
                            ok: false,
                            command: "codemode",
                            error: { code: "CODEMODE_ERROR", message: outcome.error, details: { logs: outcome.logs, stats: outcome.stats } },
                            code: outcome.code,
                            stats: outcome.stats,
                            trace: bundle.trace(),
                            wallMs: outcome.wallMs,
                            backend: pool.backend(),
                        },
                    };
                }
                const rendered = formatCodemodeResult(outcome.result, {
                    ...(outcome.stats ? { stats: outcome.stats } : {}),
                    wallMs: outcome.wallMs,
                    backend: pool.backend(),
                });
                const activationMs = outcome.wallMs;
                return {
                    content: [{ type: "text", text: bounded(rendered) }],
                    details: {
                        ok: true,
                        command: "codemode",
                        result: outcome.result,
                        logs: outcome.logs,
                        rendered,
                        stats: outcome.stats,
                        trace: bundle.trace(),
                        wallMs: outcome.wallMs,
                        activationMs,
                        backend: pool.backend(),
                    },
                };
            }
            catch (cause) {
                return failure("codemode", cause, signal);
            }
        },
    });
    // Escape hatches: one-shot tools for simple lookups. Prefer asgrep.
    // They ride the same session sticky pool when available (no cold spawn).
    pi.registerTool({
        name: "asgrep_search",
        label: "asgrep search",
        promptSnippet: "One-shot asgrep search (natural, defs, callers, pattern, chain, semantic)",
        description: "One-shot search. Prefer asgrep for anything multi-step, parallel, or filtered. Call this on your own whenever a single lookup is enough.",
        parameters: searchParameters,
        renderCall(args, theme, context) {
            return presentText(formatSearchCall(args, theme), context.lastComponent);
        },
        renderResult(result, options, theme, context) {
            return renderAsgrepResult(result, options, theme, context);
        },
        async execute(_toolCallId, params, signal, onUpdate, ctx) {
            const options = signal ? { signal } : {};
            const started = performance.now();
            report(onUpdate, "search", "started");
            try {
                ensurePool();
                const scopedPath = (typeof params.in === "string" ? params.in : undefined)
                    ?? (typeof params.fileFilter === "string" ? params.fileFilter : undefined)
                    ?? extractInPath(params.query);
                const root = await freshRoot(ctx.cwd, signal, scopedPath);
                const [tool, args] = searchToolCall(params);
                const response = await machineCall(root, tool, args, searchArgs(params), { cwd: ctx.cwd }, options);
                report(onUpdate, "search", "completed");
                return success("search", response, {
                    query: params.query,
                    mode: params.mode ?? "natural",
                    activationMs: performance.now() - started,
                    backend: pool.backend(),
                });
            }
            catch (cause) {
                return failure("search", cause, signal);
            }
        },
    });
    // Trained-priorty escape hatches: direct edit/read without writing JS.
    // Both ride the connector's arg plumbing and the same sticky pool.
    pi.registerTool({
        name: "asgrep_edit",
        label: "asgrep edit",
        promptSnippet: "Edit a file by exact-string replace (asgrep_edit; use asgrep for multi-step)",
        description: "Edit a file by exact-string replace — oldText must match exactly once. edits[] applies many edits atomically. Prefer the asgrep tool for anything multi-step or filtered.",
        parameters: editParameters,
        renderCall(args, theme, context) {
            return presentText(formatEditCall(args, theme), context.lastComponent);
        },
        renderResult(result, options, theme, context) {
            return renderAsgrepResult(result, options, theme, context);
        },
        async execute(_toolCallId, params, signal, onUpdate, ctx) {
            report(onUpdate, "edit", "started");
            try {
                ensurePool();
                const options = signal ? { signal } : {};
                const root = await freshRoot(ctx.cwd, signal);
                const sticky = await pool.acquire(root);
                const bundle = createAsgrepConnector({ run: (a, c, o) => runtime.run(a, c, o), sticky }, { cwd: ctx.cwd }, options);
                const response = await bundle.asgrep.edit(params);
                report(onUpdate, "edit", "completed");
                return success("edit", response, { backend: pool.backend() });
            }
            catch (cause) {
                return failure("edit", cause, signal);
            }
        },
    });
    pi.registerTool({
        name: "asgrep_read",
        label: "asgrep read",
        promptSnippet: "Read file windows or hit refs (asgrep_read; use asgrep for multi-step)",
        description: "Read a file window or resolve hit refs (path#L1-L40) into content. Prefer the asgrep tool for composed lookups.",
        parameters: readParameters,
        renderCall(args, theme, context) {
            return presentText(formatReadCall(args, theme), context.lastComponent);
        },
        renderResult(result, options, theme, context) {
            return renderAsgrepResult(result, options, theme, context);
        },
        async execute(_toolCallId, params, signal, onUpdate, ctx) {
            report(onUpdate, "read", "started");
            try {
                ensurePool();
                const options = signal ? { signal } : {};
                const root = await freshRoot(ctx.cwd, signal);
                const sticky = await pool.acquire(root);
                const bundle = createAsgrepConnector({ run: (a, c, o) => runtime.run(a, c, o), sticky }, { cwd: ctx.cwd }, options);
                const response = await bundle.asgrep.read(params);
                report(onUpdate, "read", "completed");
                return success("read", response, { backend: pool.backend() });
            }
            catch (cause) {
                return failure("read", cause, signal);
            }
        },
    });
    pi.registerTool({
        name: "asgrep_index",
        label: "asgrep index",
        promptSnippet: "Build or rebuild the asgrep index",
        description: "Build or rebuild the index. Prefer asgrep.indexRepo inside asgrep.",
        parameters: indexParameters,
        renderCall(args, theme, context) {
            return presentText(formatIndexCall(args.force === true, theme), context.lastComponent);
        },
        renderResult(result, options, theme, context) {
            return renderAsgrepResult(result, options, theme, context);
        },
        async execute(_toolCallId, params, signal, onUpdate, ctx) {
            const force = params.force === true;
            const command = force ? "reindex" : "index";
            report(onUpdate, command, "started");
            try {
                ensurePool();
                const options = signal ? { signal } : {};
                const root = await freshRoot(ctx.cwd, signal);
                const response = await machineCall(root, "index_repo", { force }, [command, ".", "--json"], { cwd: ctx.cwd }, options);
                report(onUpdate, command, "completed");
                return success(command, response);
            }
            catch (cause) {
                return failure(command, cause, signal);
            }
        },
    });
    pi.registerTool({
        name: "asgrep_status",
        label: "asgrep status",
        promptSnippet: "asgrep index and backend status",
        description: "Index/runtime status. Prefer asgrep.indexStatus inside asgrep.",
        parameters: statusParameters,
        renderCall(_args, theme, context) {
            return presentText(formatStatusCall(theme), context.lastComponent);
        },
        renderResult(result, options, theme, context) {
            return renderAsgrepResult(result, options, theme, context);
        },
        async execute(_toolCallId, _params, signal, onUpdate, ctx) {
            report(onUpdate, "status", "started");
            try {
                ensurePool();
                const options = signal ? { signal } : {};
                const root = await freshRoot(ctx.cwd, signal);
                const response = await machineCall(root, "index_status", {}, ["status", ".", "--json"], { cwd: ctx.cwd }, options);
                report(onUpdate, "status", "completed");
                return success("status", response);
            }
            catch (cause) {
                return failure("status", cause, signal);
            }
        },
    });
}
const SEARCH_CALL_SPEC = {
    semantic: { tool: "semantic" },
    chain: { tool: "chain" },
    defs: { tool: "defs", key: "symbol" },
    callers: { tool: "callers", key: "symbol" },
    imports: { tool: "imports", key: "module" },
    pattern: { tool: "search", prefix: "pattern" },
    word: { tool: "search", prefix: "word" },
    literal: { tool: "search", prefix: "literal" },
    regex: { tool: "search", prefix: "regex" },
    natural: { tool: "search" },
};
function searchToolCall(params) {
    const mode = params.mode ?? "natural";
    const limit = params.limit ?? DEFAULT_LIMIT;
    const excerpt_lines = params.excerptLines ?? 0;
    const query = scopedSearchQuery(params);
    const spec = SEARCH_CALL_SPEC[mode];
    const lang = typeof params.lang === "string" ? params.lang.trim() : "";
    if (spec.tool === "semantic") {
        return ["semantic", { query, limit, excerpt_lines, format: "capsule", ...(lang ? { lang } : {}) }];
    }
    if (spec.tool === "chain") {
        return ["chain", { query, limit, top_n: 20 }];
    }
    if (spec.tool === "search") {
        const prefixed = spec.prefix ? `${spec.prefix}: ${query}` : query;
        return ["search", { query: prefixed, limit, excerpt_lines, format: "capsule", ...(lang ? { lang } : {}) }];
    }
    return [spec.tool, { [spec.key]: params.query, limit, excerpt_lines, ...(lang ? { lang } : {}) }];
}
