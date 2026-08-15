import { Type } from "typebox";
import { createAsgrepConnector, runCodemode, runNativeBatch, runBatchViaStdin, CODEMODE_TYPES_FOR_MODEL, NativeSessionPool, argvFor, asEnvelope, } from "./codemode/index.js";
import { AstSgrepRuntime, FreshnessCoordinator, RuntimeError } from "./runtime.js";
import { ASGREP_PROMPT_GUIDELINES, ASGREP_PROMPT_SNIPPET, formatCodemodeCall, formatCodemodeResult, formatIndexCall, formatIndexResult, formatSearchCall, formatSearchResult, formatStatusCall, formatStatusResult, presentText, } from "./present.js";
const DEFAULT_LIMIT = 8;
const MAX_LIMIT = 100;
const MAX_EXCERPT_LINES = 100;
const MAX_CONTENT_CHARS = 1_200;
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
}, { additionalProperties: false });
const indexParameters = Type.Object({
    force: Type.Optional(Type.Boolean({ default: false, description: "Rebuild the index from scratch" })),
}, { additionalProperties: false });
const statusParameters = Type.Object({}, { additionalProperties: false });
const codemodeParameters = Type.Object({
    code: Type.String({
        minLength: 1,
        maxLength: 32_000,
        description: "JavaScript async body that calls asgrep.* methods. Prefer Promise.all for independent lookups. Return only the shaped final value.",
    }),
}, { additionalProperties: false });
function bounded(text) {
    return text.length <= MAX_CONTENT_CHARS ? text : `${text.slice(0, MAX_CONTENT_CHARS - 1)}…`;
}
function success(command, response, extra = {}) {
    const text = command === "status"
        ? formatStatusResult(response)
        : command === "index" || command === "reindex"
            ? formatIndexResult(command, response)
            : formatSearchResult(response, { command, ...extra });
    return {
        content: [{ type: "text", text: bounded(text) }],
        // The tool execute owns its machine command: normalize the envelope's
        // command (native catalog names like index_status/index_repo must surface
        // as the machine commands status/index/reindex).
        details: { ok: true, command, response: { ...response, command }, ...extra },
    };
}
function errorDetails(cause, signal) {
    if (signal?.aborted) {
        return { code: "CANCELLED", message: "cancelled", details: {} };
    }
    return cause instanceof RuntimeError
        ? { code: cause.code, message: cause.message, details: cause.details }
        : { code: "UNEXPECTED_ERROR", message: cause instanceof Error ? cause.message : String(cause), details: {} };
}
function failure(command, cause, signal) {
    const error = errorDetails(cause, signal);
    return {
        content: [{ type: "text", text: bounded(`${command} failed [${error.code}]: ${error.message}`) }],
        details: { ok: false, command, error },
    };
}
function report(onUpdate, command, phase) {
    onUpdate?.({
        content: [{ type: "text", text: `${command} ${phase}` }],
        details: { command, phase },
    });
}
function queryForMode(query, mode) {
    if (mode === "pattern" || mode === "defs" || mode === "callers" || mode === "word" || mode === "literal" || mode === "regex" || mode === "imports") {
        return `${mode}: ${query}`;
    }
    return query;
}
function searchArgs(params) {
    const mode = params.mode ?? "natural";
    const query = queryForMode(params.query, mode);
    const output = ["--json", "--format", "agent-capsule", "--limit", String(params.limit ?? DEFAULT_LIMIT), "--excerpt-lines", String(params.excerptLines ?? 0)];
    return mode === "chain" || mode === "semantic"
        ? [mode, query, ".", ...output]
        : [...output, query, "."];
}
async function execute(runtime, command, args, signal, onUpdate, ctx, before) {
    report(onUpdate, command, "started");
    try {
        await before?.();
        const response = await runtime.run(args, { cwd: ctx.cwd }, signal ? { signal } : {});
        report(onUpdate, command, "completed");
        return success(command, response);
    }
    catch (cause) {
        return failure(command, cause);
    }
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
    const nativeCall = async (tool, args, context, options = {}) => {
        ensurePool();
        const root = await resolveRoot(context.cwd);
        const worker = await pool.acquire(root);
        if (worker) {
            return asEnvelope(await worker.call(tool, args, options.signal ? { signal: options.signal } : {}));
        }
        // Cold CLI only when a real binary resolves -- never remap missing natives to BINARY_RESOLUTION_FAILED.
        return runCli(argvFor(tool, args), context, options);
    };
    // Freshness + tools share the same warm in-process Searcher as Code Mode.
    const warmRuntime = {
        run: (args, context, options) => runtime.run(args, context, options),
        resolveRoot: (context) => resolveRoot(context.cwd),
        nativeCall,
    };
    if (runtime.watchExternalChanges !== undefined) {
        warmRuntime.watchExternalChanges = runtime.watchExternalChanges;
    }
    if (runtime.resolveIndexPath) {
        warmRuntime.resolveIndexPath = (root) => runtime.resolveIndexPath(root);
    }
    if (runtime.inspectIndexCompatibility) {
        warmRuntime.inspectIndexCompatibility = (context) => runtime.inspectIndexCompatibility(context);
    }
    if (runtime.rebuildIncompatibleIndex) {
        warmRuntime.rebuildIncompatibleIndex = async (context, options) => {
            const root = await resolveRoot(context.cwd);
            await pool.invalidate(root);
            return runtime.rebuildIncompatibleIndex(context, options);
        };
    }
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
        // Warm the in-process Searcher at session start so the first asgrep
        // search does not pay NAPI/SQLite open on the user's first lookup.
        void (async () => {
            try {
                ensurePool();
                const root = await resolveRoot(ctx.cwd);
                await pool.acquire(root);
            }
            catch {
                // Doctor reports backend errors; a failed warmup must not block the session.
            }
        })();
    });
    pi.on("session_shutdown", () => {
        freshness.shutdown?.();
        void pool.shutdown();
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
            "Write JavaScript that calls typed asgrep.* methods. Compose with await / Promise.all, filter in code, return only the shaped final value.",
            "Runs in-process (native addon) with a warm Searcher for the Pi session.",
            "",
            CODEMODE_TYPES_FOR_MODEL,
            "",
            "Example:",
            "async () => {",
            "  const [seed, status] = await Promise.all([",
            "    asgrep.search({ query: 'auth refresh', limit: 5 }),",
            "    asgrep.indexStatus(),",
            "  ]);",
            "  const symbol = seed.hits?.[0]?.symbol;",
            "  if (!symbol) return { seed, status };",
            "  const graph = await asgrep.chain({ query: symbol, limit: 20 });",
            "  return { symbol, nodes: graph.nodes?.slice?.(0, 10) ?? graph, status };",
            "}",
        ].join("\n"),
        parameters: codemodeParameters,
        renderCall(args, theme, context) {
            return presentText(formatCodemodeCall(args.code, theme), context.lastComponent);
        },
        renderResult(result, options, theme, context) {
            const text = result.content[0]?.type === "text" ? result.content[0].text : "";
            const lines = text.split("\n");
            const max = options.expanded ? lines.length : 16;
            const shown = lines.slice(0, max).join("\n");
            const rest = lines.length > max ? `\n… ${lines.length - max} more` : "";
            return presentText(shown + rest, context.lastComponent);
        },
        async execute(_toolCallId, params, signal, onUpdate, ctx) {
            report(onUpdate, "codemode", "started");
            try {
                const timeoutMs = runtime.config?.timeoutMs ?? 30_000;
                const deadline = Date.now() + timeoutMs;
                const timeoutSignal = AbortSignal.timeout(timeoutMs);
                const operationSignal = signal
                    ? AbortSignal.any([signal, timeoutSignal])
                    : timeoutSignal;
                const options = { signal: operationSignal };
                ensurePool();
                const root = await freshness.ensureFresh(warmRuntime, { cwd: ctx.cwd }, options);
                const env = runtime.nativeEnv?.() ?? { NO_COLOR: "1" };
                let binary = null;
                try {
                    binary = runtime.resolveBinaryPath?.({ env }) ?? null;
                }
                catch {
                    binary = null;
                }
                // In-process NAPI first; CLI sticky only if addon missing.
                const sticky = await pool.acquire(root);
                const batchHost = {
                    run: (args, context, runOptions) => runtime.run(args, context, runOptions ?? {}),
                    sticky,
                };
                if (binary) {
                    batchHost.runBatch = (calls, context, runOptions) => runNativeBatch((a, c, o) => runtime.run(a, c, o ?? {}), calls, context, runOptions, (body, c, o) => {
                        const stdinOpts = {
                            binary: binary,
                            cwd: c.cwd,
                            body,
                            env,
                        };
                        if (o?.signal)
                            stdinOpts.signal = o.signal;
                        if (runtime.config?.timeoutMs !== undefined)
                            stdinOpts.timeoutMs = runtime.config.timeoutMs;
                        if (runtime.config?.maxOutputBytes !== undefined)
                            stdinOpts.maxOutputBytes = runtime.config.maxOutputBytes;
                        return runBatchViaStdin(stdinOpts);
                    });
                }
                const bundle = createAsgrepConnector(batchHost, { cwd: ctx.cwd }, options);
                bundle.resetStats();
                const codemodeOptions = { stats: bundle.stats };
                codemodeOptions.timeoutMs = Math.max(1, deadline - Date.now());
                codemodeOptions.signal = operationSignal;
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
                        wallMs: outcome.wallMs,
                        activationMs,
                        backend: pool.backend(),
                    },
                };
            }
            catch (cause) {
                return failure("codemode", cause);
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
        renderResult(result, _options, _theme, context) {
            const text = result.content[0]?.type === "text" ? result.content[0].text : "";
            return presentText(text, context.lastComponent);
        },
        async execute(_toolCallId, params, signal, onUpdate, ctx) {
            const options = signal ? { signal } : {};
            const started = performance.now();
            report(onUpdate, "search", "started");
            try {
                ensurePool();
                const root = await freshness.ensureFresh(warmRuntime, { cwd: ctx.cwd }, options);
                const sticky = await pool.acquire(root);
                const response = sticky
                    ? await sticky.call(...searchToolCall(params), options)
                    : await runCli(searchArgs(params), { cwd: ctx.cwd }, options);
                report(onUpdate, "search", "completed");
                return success("search", response, {
                    query: params.query,
                    mode: params.mode ?? "natural",
                    activationMs: performance.now() - started,
                    backend: pool.backend(),
                });
            }
            catch (cause) {
                return failure("search", cause);
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
        renderResult(result, _options, _theme, context) {
            const text = result.content[0]?.type === "text" ? result.content[0].text : "";
            return presentText(text, context.lastComponent);
        },
        async execute(_toolCallId, params, signal, onUpdate, ctx) {
            const force = params.force === true;
            const command = force ? "reindex" : "index";
            report(onUpdate, command, "started");
            try {
                ensurePool();
                const root = await resolveRoot(ctx.cwd);
                const sticky = await pool.acquire(root);
                const response = sticky
                    ? await sticky.call("index_repo", { force }, signal ? { signal } : {})
                    : await runCli([command, ".", "--json"], { cwd: ctx.cwd }, signal ? { signal } : {});
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
        renderResult(result, _options, _theme, context) {
            const text = result.content[0]?.type === "text" ? result.content[0].text : "";
            return presentText(text, context.lastComponent);
        },
        async execute(_toolCallId, _params, signal, onUpdate, ctx) {
            report(onUpdate, "status", "started");
            try {
                ensurePool();
                const root = await resolveRoot(ctx.cwd);
                const sticky = await pool.acquire(root);
                const response = sticky
                    ? await sticky.call("index_status", {}, signal ? { signal } : {})
                    : await runCli(["status", ".", "--json"], { cwd: ctx.cwd }, signal ? { signal } : {});
                report(onUpdate, "status", "completed");
                return success("status", response);
            }
            catch (cause) {
                return failure("status", cause);
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
    const spec = SEARCH_CALL_SPEC[mode];
    if (spec.tool === "semantic") {
        return ["semantic", { query: params.query, limit, excerpt_lines, format: "capsule" }];
    }
    if (spec.tool === "chain") {
        return ["chain", { query: params.query, limit, top_n: 20 }];
    }
    if (spec.tool === "search") {
        const query = spec.prefix ? `${spec.prefix}: ${params.query}` : params.query;
        return ["search", { query, limit, excerpt_lines, format: "capsule" }];
    }
    // defs / callers / imports
    return [spec.tool, { [spec.key]: params.query, limit, excerpt_lines }];
}
const COMMANDS = [
    ["asgrep-doctor", "Check the ast-sgrep runtime, native binary, index, and project configuration", "doctor"],
    ["asgrep-status", "Show ast-sgrep runtime, index, backend, and capability status", "status"],
    ["asgrep-index", "Build the ast-sgrep index for the current project", "index"],
    ["asgrep-reindex", "Rebuild the ast-sgrep index for the current project", "reindex"],
];
async function runCommand(runtime, command, ctx, args) {
    if (args.trim() !== "") {
        return {
            ok: false,
            command,
            error: { code: "INVALID_ARGUMENTS", message: `/${command} does not accept arguments`, details: { args } },
        };
    }
    try {
        const response = await runtime.run([command.slice("asgrep-".length), ".", "--json"], { cwd: ctx.cwd });
        return { ok: true, command, response };
    }
    catch (cause) {
        return { ok: false, command, error: errorDetails(cause) };
    }
}
function compactCommandResult(result) {
    if (!result.ok)
        return `${result.command} failed [${result.error.code}]: ${result.error.message}`;
    const response = result.response;
    const counts = response.counts && typeof response.counts === "object"
        ? Object.entries(response.counts).map(([key, value]) => `${key}=${String(value)}`).join(" ")
        : "";
    const state = typeof response.status === "string" ? response.status
        : typeof response.index_status === "string" ? response.index_status
            : response.ok ? "healthy" : "failed";
    return bounded([`${result.command}: ${state}`, counts].filter(Boolean).join(" · "));
}
export function registerAstSgrepCommands(pi, runtime = new AstSgrepRuntime(pi)) {
    for (const [name, description] of COMMANDS) {
        pi.registerCommand(name, {
            description,
            async handler(args, context) {
                const ctx = context;
                const result = await runCommand(runtime, name, ctx, args);
                const output = ctx.hasUI ? compactCommandResult(result) : JSON.stringify(result);
                ctx.ui.notify(output, result.ok ? "info" : "error");
            },
        });
    }
}
export default function astSgrepExtension(pi) {
    const runtime = new AstSgrepRuntime(pi);
    const freshness = new FreshnessCoordinator({ refreshIntervalMs: runtime.config.refreshIntervalMs });
    registerAstSgrepTools(pi, runtime, freshness);
    registerAstSgrepCommands(pi, runtime);
}
