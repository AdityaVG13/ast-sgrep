/**
 * Shared leaf module: version constants, wire types, RuntimeError.
 * Imports nothing from sibling modules — any file may depend on it.
 */
export declare const RUNTIME_VERSION = "2.5.4";
export declare const MACHINE_SCHEMA_VERSION = "1.0.0";
export declare const CONFIG_SCHEMA_VERSION: 1;
/** Index format this release ships. Must equal INDEX_SCHEMA_VERSION in crates/ast-sgrep-core (check-contract gates it). */
export declare const INDEX_FORMAT_VERSION: 16;
export declare const DEFAULT_TIMEOUT_MS = 30000;
export declare const DEFAULT_MAX_OUTPUT_BYTES: number;
export declare const DEFAULT_REFRESH_INTERVAL_MS = 30000;
/** Max one caller waits on a shared index refresh before serving stale. */
export declare const DEFAULT_FRESHNESS_WAIT_MS = 10000;
export interface RuntimeContext {
    cwd: string;
}
export declare const RESOLVED_ROOT: unique symbol;
export type InternalRuntimeContext = RuntimeContext & {
    [RESOLVED_ROOT]?: true;
};
export interface RunOptions {
    signal?: AbortSignal;
    timeoutMs?: number;
    env?: Readonly<Record<string, string>>;
}
export interface ExecOptions {
    cwd: string;
    env: NodeJS.ProcessEnv;
    signal?: AbortSignal;
    timeout?: number;
}
export interface ExecResult {
    stdout: string;
    stderr: string;
    code?: number | null;
    exitCode?: number | null;
    signal?: string | null;
}
export interface PiExec {
    exec(command: string, args: readonly string[], options: ExecOptions): Promise<ExecResult>;
}
export interface MachineEnvelope {
    tool: "asgrep";
    schema_version: string;
    ok: boolean;
    version?: string;
    machine_schema_version?: string;
    [key: string]: unknown;
}
export declare class RuntimeError extends Error {
    readonly code: string;
    readonly details: Readonly<Record<string, unknown>>;
    constructor(code: string, message: string, details?: Readonly<Record<string, unknown>>);
}
