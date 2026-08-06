import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import {
  createAsgrepConnector,
  runCodemode,
  runNativeBatch,
  runBatchViaStdin,
  CODEMODE_TYPES_FOR_MODEL,
  NativeSessionPool,
  argvFor,
  asEnvelope,
  type StickyWorker,
} from "./codemode/index.js";
import { AstSgrepRuntime, FreshnessCoordinator, RuntimeError, type FreshnessRuntime, type MachineEnvelope, type RunOptions } from "./runtime.js";

const DEFAULT_LIMIT = 8;
const MAX_LIMIT = 100;
const MAX_EXCERPT_LINES = 100;
const MAX_CONTENT_CHARS = 1_200;
const MAX_CODEMODE_RESULT_CHARS = 8_000;

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

type RuntimeLike = {
  run(args: readonly string[], context: { cwd: string }, options?: RunOptions): Promise<MachineEnvelope>;
  resolveRoot?(context: { cwd: string }): Promise<string>;
  resolveBinaryPath?(options?: { env?: NodeJS.ProcessEnv }): string;
  nativeEnv?(options?: { env?: NodeJS.ProcessEnv }): NodeJS.ProcessEnv;
  config?: { timeoutMs?: number; refreshIntervalMs?: number };
  inspectIndexCompatibility?(context: { cwd: string }): Promise<"ready" | "missing" | "incompatible">;
  rebuildIncompatibleIndex?(context: { cwd: string }, options?: RunOptions): Promise<MachineEnvelope>;
};
type FreshnessLike = Pick<FreshnessCoordinator, "ensureFresh" | "markAffectedPath">;
type ToolContext = { cwd: string };
type CommandContext = ToolContext & {
  hasUI: boolean;
  ui: { notify(message: string, type?: "info" | "warning" | "error"): void };
};
type CommandResult =
  | { ok: true; command: string; response: MachineEnvelope }
  | { ok: false; command: string; error: { code: string; message: string; details: Readonly<Record<string, unknown>> } };
type Update = (result: { content: Array<{ type: "text"; text: string }>; details: Record<string, unknown> }) => void;

function bounded(text: string): string {
  return text.length <= MAX_CONTENT_CHARS ? text : `${text.slice(0, MAX_CONTENT_CHARS - 1)}…`;
}

function success(command: string, response: MachineEnvelope) {
  const count = Array.isArray(response.hits) ? response.hits.length :
    typeof response.count === "number" ? response.count :
    typeof response.total === "number" ? response.total : undefined;
  const summary = count === undefined ? `${command} completed` : `${command} completed: ${count} result${count === 1 ? "" : "s"}`;
  return {
    content: [{ type: "text" as const, text: bounded(summary) }],
    // The tool execute owns its machine command: normalize the envelope's
    // command (native catalog names like index_status/index_repo must surface
    // as the machine commands status/index/reindex).
    details: { ok: true, command, response: { ...response, command } },
  };
}

function errorDetails(cause: unknown, signal?: AbortSignal): { code: string; message: string; details: Readonly<Record<string, unknown>> } {
  if (signal?.aborted) {
    return { code: "CANCELLED", message: "cancelled", details: {} };
  }
  return cause instanceof RuntimeError
    ? { code: cause.code, message: cause.message, details: cause.details }
    : { code: "UNEXPECTED_ERROR", message: cause instanceof Error ? cause.message : String(cause), details: {} };
}

function failure(command: string, cause: unknown, signal?: AbortSignal) {
  const error = errorDetails(cause, signal);
  return {
    content: [{ type: "text" as const, text: bounded(`${command} failed [${error.code}]: ${error.message}`) }],
    details: { ok: false, command, error },
  };
}

function report(onUpdate: Update | undefined, command: string, phase: "started" | "completed"): void {
  onUpdate?.({
    content: [{ type: "text", text: `${command} ${phase}` }],
    details: { command, phase },
  });
}

type SearchMode = "natural" | "pattern" | "defs" | "callers" | "chain" | "semantic" | "word" | "literal" | "regex" | "imports";

function queryForMode(query: string, mode: SearchMode): string {
  if (mode === "pattern" || mode === "defs" || mode === "callers" || mode === "word" || mode === "literal" || mode === "regex" || mode === "imports") {
    return `${mode}: ${query}`;
  }
  return query;
}

function searchArgs(params: { query: string; mode?: SearchMode; limit?: number; excerptLines?: number }): string[] {
  const mode = params.mode ?? "natural";
  const query = queryForMode(params.query, mode);
  const output = ["--json", "--format", "agent-capsule", "--limit", String(params.limit ?? DEFAULT_LIMIT), "--excerpt-lines", String(params.excerptLines ?? 0)];
  return mode === "chain" || mode === "semantic"
    ? [mode, query, ".", ...output]
    : [...output, query, "."];
}

async function execute(
  runtime: RuntimeLike,
  command: string,
  args: readonly string[],
  signal: AbortSignal | undefined,
  onUpdate: Update | undefined,
  ctx: ToolContext,
  before?: () => Promise<void>,
) {
  report(onUpdate, command, "started");
  try {
    await before?.();
    const response = await runtime.run(args, { cwd: ctx.cwd }, signal ? { signal } : {});
    report(onUpdate, command, "completed");
    return success(command, response);
  } catch (cause) {
    return failure(command, cause);
  }
}

export function registerAstSgrepTools(
  pi: ExtensionAPI,
  runtime: RuntimeLike = new AstSgrepRuntime(pi),
  freshness: FreshnessLike = runtime instanceof AstSgrepRuntime
    ? new FreshnessCoordinator({ refreshIntervalMs: runtime.config.refreshIntervalMs! })
    : new FreshnessCoordinator(),
): void {
  const pool = new NativeSessionPool();
  // Prefer a registration-local pool so tests / multi-agent hosts do not share
  // sticky state. sharedNativePool remains for advanced single-session reuse.

  const ensurePool = (): void => {
    try {
      const env = runtime.nativeEnv?.() ?? { NO_COLOR: "1" };
      let binary: string | undefined;
      try {
        binary = runtime.resolveBinaryPath?.({ env });
      } catch {
        binary = undefined;
      }
      const opts: {
        binary?: string;
        env: NodeJS.ProcessEnv;
        timeoutMs?: number;
        useEmbed?: boolean;
        indexPath?: string;
      } = { env };
      if (binary) opts.binary = binary;
      if (runtime.config?.timeoutMs !== undefined) opts.timeoutMs = runtime.config.timeoutMs;
      if (typeof env.ASGREP_NO_EMBED === "string") {
        opts.useEmbed = env.ASGREP_NO_EMBED !== "1" && env.ASGREP_NO_EMBED !== "true";
      }
      if (typeof env.ASGREP_INDEX_PATH === "string") opts.indexPath = env.ASGREP_INDEX_PATH;
      pool.configure(opts);
    } catch {
      pool.configure({});
    }
  };

  const resolveRoot = async (cwd: string): Promise<string> =>
    runtime.resolveRoot ? await runtime.resolveRoot({ cwd }) : cwd;

  const nativeCall = async (
    tool: string,
    args: Record<string, unknown>,
    context: { cwd: string },
    options: RunOptions = {},
  ): Promise<MachineEnvelope> => {
    ensurePool();
    const root = await resolveRoot(context.cwd);
    const worker = await pool.acquire(root);
    if (worker) {
      return asEnvelope(await worker.call(tool, args, options.signal ? { signal: options.signal } : {}));
    }
    // Last resort: cold CLI argv (tests without NAPI / missing addon).
    return runtime.run(argvFor(tool, args), context, options);
  };

  // Freshness + tools share the same warm in-process Searcher as Code Mode.
  const warmRuntime: FreshnessRuntime = {
    run: (args, context, options) => runtime.run(args, context, options),
    resolveRoot: (context) => resolveRoot(context.cwd),
    nativeCall,
  };
  if (runtime.inspectIndexCompatibility) {
    warmRuntime.inspectIndexCompatibility = (context) => runtime.inspectIndexCompatibility!(context);
  }
  if (runtime.rebuildIncompatibleIndex) {
    warmRuntime.rebuildIncompatibleIndex = (context, options) => runtime.rebuildIncompatibleIndex!(context, options);
  }

  pi.on("tool_result", (event, ctx) => {
    if (event.isError || (event.toolName !== "write" && event.toolName !== "edit")) return;
    const path = event.input.path;
    if (typeof path === "string") freshness.markAffectedPath(path, ctx.cwd);
  });

  // Primary surface: Code Mode — in-process NAPI (MCP-class), compose in JS.
  // Sibling to MCP: pick one surface; both link core, never each other.
  pi.registerTool({
    name: "asgrep_codemode",
    label: "ast-sgrep Code Mode",
    description: [
      "Primary ast-sgrep tool. Write JavaScript that calls typed asgrep.* methods.",
      "Compose with await / Promise.all, filter in code, return only the shaped final value.",
      "Runs in-process (native addon) — no CLI spawn; warm Searcher for the Pi session.",
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
    async execute(_toolCallId, params, signal, onUpdate, ctx) {
      report(onUpdate, "codemode", "started");
      try {
        const options = signal ? { signal } : {};
        await freshness.ensureFresh(warmRuntime, { cwd: ctx.cwd }, options);
        ensurePool();
        const root = await resolveRoot(ctx.cwd);
        const env = runtime.nativeEnv?.() ?? { NO_COLOR: "1" };
        let binary: string | null = null;
        try {
          binary = runtime.resolveBinaryPath?.({ env }) ?? null;
        } catch {
          binary = null;
        }
        // In-process NAPI first; CLI sticky only if addon missing.
        const sticky: StickyWorker | null = await pool.acquire(root);
        const batchHost: {
          run: (args: readonly string[], context: { cwd: string }, runOptions?: { signal?: AbortSignal }) => Promise<MachineEnvelope>;
          sticky: StickyWorker | null;
          runBatch?: (
            calls: Array<{ id: string; tool: string; args: Record<string, unknown> }>,
            context: { cwd: string },
            runOptions?: { signal?: AbortSignal },
          ) => ReturnType<typeof runNativeBatch>;
        } = {
          run: (args, context, runOptions) => runtime.run(args, context, runOptions ?? {}),
          sticky,
        };
        if (binary) {
          batchHost.runBatch = (calls, context, runOptions) =>
            runNativeBatch(
              (a, c, o) => runtime.run(a, c, o ?? {}),
              calls,
              context,
              runOptions,
              (body, c, o) => {
                const stdinOpts: Parameters<typeof runBatchViaStdin>[0] = {
                  binary: binary!,
                  cwd: c.cwd,
                  body,
                  env,
                };
                if (o?.signal) stdinOpts.signal = o.signal;
                if (runtime.config?.timeoutMs !== undefined) stdinOpts.timeoutMs = runtime.config.timeoutMs;
                return runBatchViaStdin(stdinOpts);
              },
            );
        }
        const bundle = createAsgrepConnector(batchHost, { cwd: ctx.cwd }, options);
        bundle.resetStats();
        const codemodeOptions: {
          stats: () => ReturnType<typeof bundle.stats>;
          timeoutMs?: number;
          signal?: AbortSignal;
        } = { stats: bundle.stats };
        if (runtime.config?.timeoutMs !== undefined) codemodeOptions.timeoutMs = runtime.config.timeoutMs;
        if (signal) codemodeOptions.signal = signal;
        const outcome = await runCodemode(params.code, bundle.asgrep, codemodeOptions);
        report(onUpdate, "codemode", "completed");
        if (!outcome.ok) {
          return {
            content: [{ type: "text" as const, text: bounded(`codemode failed: ${outcome.error ?? "unknown error"}`) }],
            details: {
              ok: false,
              command: "codemode",
              error: { code: "CODEMODE_ERROR", message: outcome.error ?? "unknown error", details: { logs: outcome.logs, stats: outcome.stats } },
              code: outcome.code,
              stats: outcome.stats,
              wallMs: outcome.wallMs,
              backend: pool.backend(),
            },
          };
        }
        const rendered = safeRender(outcome.result);
        return {
          content: [{ type: "text" as const, text: bounded(summarizeCodemode(outcome.result, outcome.stats, outcome.wallMs, pool.backend())) }],
          details: {
            ok: true,
            command: "codemode",
            result: outcome.result,
            logs: outcome.logs,
            rendered,
            stats: outcome.stats,
            wallMs: outcome.wallMs,
            backend: pool.backend(),
          },
        };
      } catch (cause) {
        return failure("codemode", cause);
      }
    },
  });

  // Escape hatches: one-shot tools for simple lookups. Prefer asgrep_codemode.
  // They ride the same session sticky pool when available (no cold spawn).
  pi.registerTool({
    name: "asgrep_search",
    label: "ast-sgrep search",
    description: "One-shot search. Prefer asgrep_codemode for anything multi-step, parallel, or filtered.",
    parameters: searchParameters,
    async execute(_toolCallId, params, signal, onUpdate, ctx) {
      const options = signal ? { signal } : {};
      report(onUpdate, "search", "started");
      try {
        await freshness.ensureFresh(warmRuntime, { cwd: ctx.cwd }, options);
        ensurePool();
        const root = await resolveRoot(ctx.cwd);
        const sticky = await pool.acquire(root);
        const response = sticky
          ? await sticky.call(...searchToolCall(params), options)
          : await runtime.run(searchArgs(params), { cwd: ctx.cwd }, options);
        report(onUpdate, "search", "completed");
        return success("search", response);
      } catch (cause) {
        return failure("search", cause);
      }
    },
  });

  pi.registerTool({
    name: "asgrep_index",
    label: "ast-sgrep index",
    description: "Build or rebuild the index. Prefer asgrep.indexRepo inside asgrep_codemode.",
    parameters: indexParameters,
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
          : await runtime.run([command, ".", "--json"], { cwd: ctx.cwd }, signal ? { signal } : {});
        report(onUpdate, command, "completed");
        return success(command, response);
      } catch (cause) {
        return failure(command, cause, signal);
      }
    },
  });

  pi.registerTool({
    name: "asgrep_status",
    label: "ast-sgrep status",
    description: "Index/runtime status. Prefer asgrep.indexStatus inside asgrep_codemode.",
    parameters: statusParameters,
    async execute(_toolCallId, _params, signal, onUpdate, ctx) {
      report(onUpdate, "status", "started");
      try {
        ensurePool();
        const root = await resolveRoot(ctx.cwd);
        const sticky = await pool.acquire(root);
        const response = sticky
          ? await sticky.call("index_status", {}, signal ? { signal } : {})
          : await runtime.run(["status", ".", "--json"], { cwd: ctx.cwd }, signal ? { signal } : {});
        report(onUpdate, "status", "completed");
        return success("status", response);
      } catch (cause) {
        return failure("status", cause);
      }
    },
  });
}

/** Map one-shot search params to typed sticky tool+args. */
function searchToolCall(params: {
  query: string;
  mode?: SearchMode;
  limit?: number;
  excerptLines?: number;
}): [string, Record<string, unknown>] {
  const mode = params.mode ?? "natural";
  const limit = params.limit ?? DEFAULT_LIMIT;
  const excerpt_lines = params.excerptLines ?? 0;
  switch (mode) {
    case "semantic":
      return ["semantic", { query: params.query, limit, excerpt_lines, format: "capsule" }];
    case "chain":
      return ["chain", { query: params.query, limit, top_n: 20 }];
    case "defs":
      return ["defs", { symbol: params.query, limit, excerpt_lines }];
    case "callers":
      return ["callers", { symbol: params.query, limit, excerpt_lines }];
    case "imports":
      return ["imports", { module: params.query, limit, excerpt_lines }];
    case "pattern":
    case "word":
    case "literal":
    case "regex":
      return ["search", { query: `${mode}: ${params.query}`, limit, excerpt_lines, format: "capsule" }];
    case "natural":
      return ["search", { query: params.query, limit, excerpt_lines, format: "capsule" }];
    default: {
      const _exhaustive: never = mode;
      void _exhaustive;
      return ["search", { query: params.query, limit, excerpt_lines, format: "capsule" }];
    }
  }
}

function safeRender(value: unknown): string {
  try {
    const text = JSON.stringify(value);
    if (text === undefined) return String(value);
    return text.length <= MAX_CODEMODE_RESULT_CHARS ? text : `${text.slice(0, MAX_CODEMODE_RESULT_CHARS - 1)}…`;
  } catch {
    return String(value);
  }
}

function summarizeCodemode(
  value: unknown,
  stats?: { calls: number; batchedCalls: number; parallelSpawnCalls: number; stickyCalls?: number; waves: number },
  wallMs?: number,
  backend?: "napi" | "cli" | "none",
): string {
  const parts: string[] = ["codemode completed"];
  if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    if (Array.isArray(record.hits)) parts[0] = `codemode completed: ${record.hits.length} hit${record.hits.length === 1 ? "" : "s"}`;
    else if (typeof record.hit_count === "number") parts[0] = `codemode completed: ${record.hit_count} hit${record.hit_count === 1 ? "" : "s"}`;
    else if (typeof record.node_count === "number") parts[0] = `codemode completed: ${record.node_count} node${record.node_count === 1 ? "" : "s"}`;
  }
  if (backend === "napi") parts.push("in-process");
  else if (backend === "cli") parts.push("cli-sticky");
  if (stats && stats.calls > 0) {
    const via =
      (stats.stickyCalls ?? 0) > 0
        ? `native ${stats.stickyCalls}`
        : stats.batchedCalls > 0
          ? `batched ${stats.batchedCalls}`
          : stats.parallelSpawnCalls > 0
            ? `parallel-spawn ${stats.parallelSpawnCalls}`
            : `${stats.calls} call${stats.calls === 1 ? "" : "s"}`;
    parts.push(via);
    if (stats.waves > 1) parts.push(`${stats.waves} waves`);
  }
  if (wallMs !== undefined) parts.push(`${wallMs}ms`);
  return parts.join(" · ");
}

const COMMANDS = [
  ["asgrep-doctor", "Check the ast-sgrep runtime, native binary, index, and project configuration", "doctor"],
  ["asgrep-status", "Show ast-sgrep runtime, index, backend, and capability status", "status"],
  ["asgrep-index", "Build the ast-sgrep index for the current project", "index"],
  ["asgrep-reindex", "Rebuild the ast-sgrep index for the current project", "reindex"],
] as const;

async function runCommand(runtime: RuntimeLike, command: string, ctx: ToolContext, args: string): Promise<CommandResult> {
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
  } catch (cause) {
    return { ok: false, command, error: errorDetails(cause) };
  }
}

function compactCommandResult(result: CommandResult): string {
  if (!result.ok) return `${result.command} failed [${result.error.code}]: ${result.error.message}`;
  const response = result.response;
  const counts = response.counts && typeof response.counts === "object"
    ? Object.entries(response.counts).map(([key, value]) => `${key}=${String(value)}`).join(" ")
    : "";
  const state = typeof response.status === "string" ? response.status
    : typeof response.index_status === "string" ? response.index_status
    : response.ok ? "healthy" : "failed";
  return bounded([`${result.command}: ${state}`, counts].filter(Boolean).join(" · "));
}

export function registerAstSgrepCommands(
  pi: ExtensionAPI,
  runtime: RuntimeLike = new AstSgrepRuntime(pi),
): void {
  for (const [name, description] of COMMANDS) {
    pi.registerCommand(name, {
      description,
      async handler(args, context) {
        const ctx = context as CommandContext;
        const result = await runCommand(runtime, name, ctx, args);
        const output = ctx.hasUI ? compactCommandResult(result) : JSON.stringify(result);
        ctx.ui.notify(output, result.ok ? "info" : "error");
      },
    });
  }
}

export default function astSgrepExtension(pi: ExtensionAPI): void {
  const runtime = new AstSgrepRuntime(pi);
  const freshness = new FreshnessCoordinator({ refreshIntervalMs: runtime.config.refreshIntervalMs! });
  registerAstSgrepTools(pi, runtime, freshness);
  registerAstSgrepCommands(pi, runtime);
}
