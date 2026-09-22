import { AstSgrepRuntime } from "../runtime/runtime.js";
import { bounded, errorDetails } from "./results.js";
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
    let diagnostics;
    try {
        if (command === "asgrep-doctor")
            diagnostics = await runtime.diagnostics?.({ cwd: ctx.cwd });
        const response = await runtime.run([command.slice("asgrep-".length), ".", "--json"], { cwd: ctx.cwd });
        return { ok: true, command, response, ...(diagnostics ? { diagnostics } : {}) };
    }
    catch (cause) {
        return { ok: false, command, error: errorDetails(cause), ...(diagnostics ? { diagnostics } : {}) };
    }
}
function compactCommandResult(result) {
    const diagnostics = result.diagnostics;
    const versions = diagnostics
        ? `extension=${diagnostics.extension ?? "unknown"} launcher=${diagnostics.launcher ?? "unknown"} native=${diagnostics.native ?? "unavailable"}`
        : "";
    const warning = typeof diagnostics?.warning === "string" ? diagnostics.warning : "";
    if (!result.ok)
        return bounded([`${result.command} failed [${result.error.code}]: ${result.error.message}`, versions, warning].filter(Boolean).join(" · "));
    const response = result.response;
    const counts = response.counts && typeof response.counts === "object"
        ? Object.entries(response.counts).map(([key, value]) => `${key}=${String(value)}`).join(" ")
        : "";
    const state = typeof response.status === "string" ? response.status
        : typeof response.index_status === "string" ? response.index_status
            : response.ok ? "healthy" : "failed";
    return bounded([`${result.command}: ${state}`, counts, versions, warning].filter(Boolean).join(" · "));
}
export function registerAstSgrepCommands(pi, runtime = new AstSgrepRuntime(pi)) {
    for (const [name, description] of COMMANDS) {
        pi.registerCommand(name, {
            description,
            async handler(args, context) {
                const ctx = context;
                const result = await runCommand(runtime, name, ctx, args);
                const output = ctx.hasUI ? compactCommandResult(result) : JSON.stringify(result);
                ctx.ui.notify(output, result.ok ? result.diagnostics?.warning ? "warning" : "info" : "error");
            },
        });
    }
}
