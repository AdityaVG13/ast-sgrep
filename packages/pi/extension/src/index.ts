import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { AstSgrepRuntime, FreshnessCoordinator, RuntimeError, type FreshnessRuntime, type MachineEnvelope } from "./runtime.js";

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

  pi.registerTool({
    name: "asgrep_search",
    label: "ast-sgrep search",
    description: "Search project code with natural language, structural patterns, symbol relationships, chains, or semantic retrieval.",
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
    description: "Build or rebuild the ast-sgrep project index.",
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
