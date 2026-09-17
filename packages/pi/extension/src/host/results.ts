/**
 * Tool-result plumbing shared by tools.ts and commands.ts: bounded text,
 * success/failure envelopes, freshness-timeout classification, reporting.
 */
import { isClosedWorkerError } from "../codemode/index.js";
import { RuntimeError, type MachineEnvelope, type RunOptions, type RuntimeContext } from "../runtime/types.js";
import type { FreshnessCoordinator } from "../runtime/freshness.js";
import { formatIndexResult, formatSearchResult, formatStatusResult } from "../ui/present.js";

export const MAX_CONTENT_CHARS = 8_000;

export type RuntimeLike = {
  run(args: readonly string[], context: { cwd: string }, options?: RunOptions): Promise<MachineEnvelope>;
  resolveRoot?(context: { cwd: string }): Promise<string>;
  resolveBinaryPath?(options?: { env?: NodeJS.ProcessEnv }): string;
  nativeEnv?(options?: { env?: NodeJS.ProcessEnv }): NodeJS.ProcessEnv;
  config?: { timeoutMs?: number; maxOutputBytes?: number; refreshIntervalMs?: number };
  inspectIndexCompatibility?(context: { cwd: string }): Promise<"ready" | "missing" | "incompatible">;
  rebuildIncompatibleIndex?(context: { cwd: string }, options?: RunOptions): Promise<MachineEnvelope>;
  resolveIndexPath?(root: string): string;
  watchExternalChanges?: boolean;
};
export type FreshnessLike = Pick<FreshnessCoordinator, "ensureFresh" | "markAffectedPath"> & {
  markRootDirty?(root: string): void;
  shutdown?(): void;
};
export type ToolContext = { cwd: string };
export type CommandContext = ToolContext & {
  hasUI: boolean;
  ui: { notify(message: string, type?: "info" | "warning" | "error"): void };
};
export type CommandResult =
  | { ok: true; command: string; response: MachineEnvelope }
  | { ok: false; command: string; error: { code: string; message: string; details: Readonly<Record<string, unknown>> } };
export type Update = (result: { content: Array<{ type: "text"; text: string }>; details: Record<string, unknown> }) => void;

export function bounded(text: string): string {
  return text.length <= MAX_CONTENT_CHARS ? text : `${text.slice(0, MAX_CONTENT_CHARS - 1)}…`;
}

export function success(
  command: string,
  response: MachineEnvelope,
  extra: { query?: string; mode?: string; activationMs?: number; backend?: string } = {},
) {
  const text = command === "status"
    ? formatStatusResult(response)
    : command === "index" || command === "reindex"
      ? formatIndexResult(command, response)
      : formatSearchResult(response, { command, ...extra });
  return {
    content: [{ type: "text" as const, text: bounded(text) }],
    // The tool execute owns its machine command: normalize the envelope's
    // command (native catalog names like index_status/index_repo must surface
    // as the machine commands status/index/reindex).
    details: { ok: true, command, response: { ...response, command }, ...extra },
  };
}

export function errorDetails(cause: unknown, signal?: AbortSignal): { code: string; message: string; details: Readonly<Record<string, unknown>> } {
  if (signal?.aborted) {
    return { code: "CANCELLED", message: "cancelled", details: {} };
  }
  if (cause instanceof RuntimeError) {
    return { code: cause.code, message: cause.message, details: cause.details };
  }
  const message = cause instanceof Error ? cause.message : String(cause);
  if (/timed out after \d+ms|timeout after \d+ms|exceeded \d+ms/i.test(message)) {
    return { code: "TIMEOUT", message, details: {} };
  }
  if (isClosedWorkerError(cause)) {
    return {
      code: "SESSION_CLOSED",
      message: "asgrep session closed; retry the search",
      details: {},
    };
  }
  return { code: "UNEXPECTED_ERROR", message, details: {} };
}

export function isFreshnessTimeout(cause: unknown, userSignal?: AbortSignal): boolean {
  if (userSignal?.aborted) return false;
  if (cause instanceof RuntimeError && (cause.code === "TIMEOUT" || cause.code === "CANCELLED")) return true;
  if (isClosedWorkerError(cause)) return true;
  const message = cause instanceof Error ? cause.message : String(cause);
  return /timed out after \d+ms|timeout after \d+ms|exceeded \d+ms/i.test(message);
}

/** Leading or mid-query `in:path` scope used to bound a fresh-directory index. */
export function extractInPath(query: string): string | undefined {
  const match = /(?:^|\s)in:([^\s]+)/.exec(query);
  const path = match?.[1];
  if (!path || path.split(/[/\\]/u).includes("..")) return undefined;
  return path;
}

export function failure(command: string, cause: unknown, signal?: AbortSignal) {
  const error = errorDetails(cause, signal);
  return {
    content: [{ type: "text" as const, text: bounded(`${command} failed [${error.code}]: ${error.message}`) }],
    details: { ok: false, command, error },
  };
}

export function report(onUpdate: Update | undefined, command: string, phase: "started" | "completed"): void {
  onUpdate?.({
    content: [{ type: "text", text: `${command} ${phase}` }],
    details: { command, phase },
  });
}
