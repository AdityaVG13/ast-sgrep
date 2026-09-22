/**
 * Slash commands: /asgrep-doctor /asgrep-status /asgrep-index /asgrep-reindex.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { AstSgrepRuntime } from "../runtime/runtime.js";
import { bounded, errorDetails, type CommandContext, type CommandResult, type RuntimeLike, type ToolContext } from "./results.js";
import type { MachineEnvelope } from "../runtime/types.js";

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
  let diagnostics: Record<string, unknown> | undefined;
  try {
    if (command === "asgrep-doctor") diagnostics = await runtime.diagnostics?.({ cwd: ctx.cwd });
    const response = await runtime.run([command.slice("asgrep-".length), ".", "--json"], { cwd: ctx.cwd });
    return { ok: true, command, response, ...(diagnostics ? { diagnostics } : {}) };
  } catch (cause) {
    return { ok: false, command, error: errorDetails(cause), ...(diagnostics ? { diagnostics } : {}) };
  }
}

function compactCommandResult(result: CommandResult): string {
  const diagnostics = result.diagnostics;
  const versions = diagnostics
    ? `extension=${diagnostics.extension ?? "unknown"} launcher=${diagnostics.launcher ?? "unknown"} native=${diagnostics.native ?? "unavailable"}`
    : "";
  const warning = typeof diagnostics?.warning === "string" ? diagnostics.warning : "";
  if (!result.ok) return bounded([`${result.command} failed [${result.error.code}]: ${result.error.message}`, versions, warning].filter(Boolean).join(" · "));
  const response = result.response;
  const counts = response.counts && typeof response.counts === "object"
    ? Object.entries(response.counts).map(([key, value]) => `${key}=${String(value)}`).join(" ")
    : "";
  const state = typeof response.status === "string" ? response.status
    : typeof response.index_status === "string" ? response.index_status
    : response.ok ? "healthy" : "failed";
  return bounded([`${result.command}: ${state}`, counts, versions, warning].filter(Boolean).join(" · "));
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
        ctx.ui.notify(output, result.ok ? result.diagnostics?.warning ? "warning" : "info" : "error");
      },
    });
  }
}
