/**
 * Slash commands: /asgrep-doctor /asgrep-status /asgrep-index /asgrep-reindex.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { AstSgrepRuntime } from "./runtime.js";
import { bounded, errorDetails, type CommandContext, type CommandResult, type RuntimeLike, type ToolContext } from "./envelope.js";
import type { MachineEnvelope } from "./types.js";

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
