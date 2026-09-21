/**
 * Shared leaf module: version constants, wire types, RuntimeError.
 * Imports nothing from sibling modules — any file may depend on it.
 */

export const RUNTIME_VERSION = "2.5.2";
export const MACHINE_SCHEMA_VERSION = "1.0.0";
export const CONFIG_SCHEMA_VERSION = 1 as const;
/** Index format this release ships. Must equal INDEX_SCHEMA_VERSION in crates/ast-sgrep-core (check-contract gates it). */
export const INDEX_FORMAT_VERSION = 16 as const;
export const DEFAULT_TIMEOUT_MS = 30_000;
export const DEFAULT_MAX_OUTPUT_BYTES = 4 * 1024 * 1024;
export const DEFAULT_REFRESH_INTERVAL_MS = 30_000;
/** Max one caller waits on a shared index refresh before serving stale. */
export const DEFAULT_FRESHNESS_WAIT_MS = 10_000;

export interface RuntimeContext { cwd: string }
export const RESOLVED_ROOT = Symbol("resolvedRoot");
export type InternalRuntimeContext = RuntimeContext & { [RESOLVED_ROOT]?: true };
export interface RunOptions { signal?: AbortSignal; timeoutMs?: number; env?: Readonly<Record<string, string>> }
export interface ExecOptions { cwd: string; env: NodeJS.ProcessEnv; signal?: AbortSignal; timeout?: number }
export interface ExecResult { stdout: string; stderr: string; code?: number | null; exitCode?: number | null; signal?: string | null }
export interface PiExec { exec(command: string, args: readonly string[], options: ExecOptions): Promise<ExecResult> }
export interface MachineEnvelope { tool: "asgrep"; schema_version: string; ok: boolean; version?: string; machine_schema_version?: string; [key: string]: unknown }

export class RuntimeError extends Error {
  constructor(public readonly code: string, message: string, public readonly details: Readonly<Record<string, unknown>> = {}) {
    super(message);
    this.name = "AstSgrepRuntimeError";
  }
}
