import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { createAsgrepConnector, runCodemode, runNativeBatch, CODEMODE_TYPES_FOR_MODEL } from "./codemode/index.js";
import { AstSgrepRuntime, FreshnessCoordinator, RuntimeError, type FreshnessRuntime, type MachineEnvelope } from "./runtime.js";

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

type RuntimeLike = FreshnessRuntime;
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
    details: { ok: true, command, response },
  };
}

function errorDetails(cause: unknown): { code: string; message: string; details: Readonly<Record<string, unknown>> } {
  return cause instanceof RuntimeError
    ? { code: cause.code, message: cause.message, details: cause.details }
    : { code: "UNEXPECTED_ERROR", message: cause instanceof Error ? cause.message : String(cause), details: {} };
}

function failure(command: string, cause: unknown) {
  const error = errorDetails(cause);
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
  pi.on("tool_result", (event, ctx) => {
    if (event.isError || (event.toolName !== "write" && event.toolName !== "edit")) return;
    const path = event.input.path;
    if (typeof path === "string") freshness.markAffectedPath(path, ctx.cwd);
  });

  // Primary surface: Code Mode (model writes JS; parallel asgrep.* calls; shaped return).
  // Independent of MCP — both sit on the native binary only.
  pi.registerTool({
    name: "asgrep_codemode",
    label: "ast-sgrep Code Mode",
    description: [
      "Prefer this tool for multi-step or parallel code search.",
      "Write JavaScript that calls typed asgrep.* methods inside a restricted executor.",
      "Compose lookups with await / Promise.all, filter results in code, and return only what you need.",
      "Do not dump every intermediate hit list — shape the final value.",
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
        await freshness.ensureFresh(runtime, { cwd: ctx.cwd }, options);
        const batchHost = {
          run: (args: readonly string[], context: { cwd: string }, runOptions?: { signal?: AbortSignal }) =>
            runtime.run(args, context, runOptions ?? {}),
          runBatch: (
            calls: Array<{ id: string; tool: string; args: Record<string, unknown> }>,
            context: { cwd: string },
            runOptions?: { signal?: AbortSignal },
          ) => runNativeBatch((a, c, o) => runtime.run(a, c, o ?? {}), calls, context, runOptions),
        };
        const bundle = createAsgrepConnector(batchHost, { cwd: ctx.cwd }, options);
        bundle.resetStats();
        const outcome = await runCodemode(params.code, bundle.asgrep, { stats: bundle.stats });
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
            },
          };
        }
        const rendered = safeRender(outcome.result);
        return {
          content: [{ type: "text" as const, text: bounded(summarizeCodemode(outcome.result, outcome.stats, outcome.wallMs)) }],
          details: {
            ok: true,
            command: "codemode",
            result: outcome.result,
            logs: outcome.logs,
            rendered,
            stats: outcome.stats,
            wallMs: outcome.wallMs,
          },
        };
      } catch (cause) {
        return failure("codemode", cause);
      }
    },
  });

  // Direct one-shot tools remain available for simple lookups (not Code Mode).
  pi.registerTool({
    name: "asgrep_search",
    label: "ast-sgrep search",
    description: "Single hybrid search call. Prefer asgrep_codemode when you need multiple lookups, filtering, or parallel work.",
    parameters: searchParameters,
    async execute(_toolCallId, params, signal, onUpdate, ctx) {
      const options = signal ? { signal } : {};
      return execute(runtime, "search", searchArgs(params), signal, onUpdate, ctx,
        () => freshness.ensureFresh(runtime, { cwd: ctx.cwd }, options).then(() => undefined));
    },
  });

  pi.registerTool({
    name: "asgrep_index",
    label: "ast-sgrep index",
    description: "Build or rebuild the ast-sgrep project index. Prefer asgrep_codemode (asgrep.indexRepo) inside multi-step programs.",
    parameters: indexParameters,
    async execute(_toolCallId, params, signal, onUpdate, ctx) {
      const command = params.force === true ? "reindex" : "index";
      return execute(runtime, command, [command, ".", "--json"], signal, onUpdate, ctx);
    },
  });

  pi.registerTool({
    name: "asgrep_status",
    label: "ast-sgrep status",
    description: "Return runtime version, protocol, root, index, counts, backend, IVF, and capability status.",
    parameters: statusParameters,
    async execute(_toolCallId, _params, signal, onUpdate, ctx) {
      return execute(runtime, "status", ["status", ".", "--json"], signal, onUpdate, ctx);
    },
  });
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
  stats?: { calls: number; batchedCalls: number; parallelSpawnCalls: number; waves: number },
  wallMs?: number,
): string {
  const parts: string[] = ["codemode completed"];
  if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    if (Array.isArray(record.hits)) parts[0] = `codemode completed: ${record.hits.length} hit${record.hits.length === 1 ? "" : "s"}`;
    else if (typeof record.hit_count === "number") parts[0] = `codemode completed: ${record.hit_count} hit${record.hit_count === 1 ? "" : "s"}`;
    else if (typeof record.node_count === "number") parts[0] = `codemode completed: ${record.node_count} node${record.node_count === 1 ? "" : "s"}`;
  }
  if (stats && stats.calls > 0) {
    const via = stats.batchedCalls > 0 ? `batched ${stats.batchedCalls}` : stats.parallelSpawnCalls > 0 ? `parallel-spawn ${stats.parallelSpawnCalls}` : `${stats.calls} call${stats.calls === 1 ? "" : "s"}`;
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
