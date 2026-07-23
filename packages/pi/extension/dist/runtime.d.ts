import { resolveBinary } from "ast-sgrep";
export declare const RUNTIME_VERSION = "1.3.2";
export declare const MACHINE_SCHEMA_VERSION = "1.0.0";
export declare const CONFIG_SCHEMA_VERSION: 1;
export declare const INDEX_FORMAT_VERSION: 5;
export declare const DEFAULT_TIMEOUT_MS = 30000;
export declare const DEFAULT_MAX_OUTPUT_BYTES: number;
export declare const DEFAULT_REFRESH_INTERVAL_MS = 30000;
export interface RuntimeConfig {
    schemaVersion?: typeof CONFIG_SCHEMA_VERSION;
    binaryPath?: string;
    root?: string;
    allowOutsideProject?: boolean;
    timeoutMs?: number;
    maxOutputBytes?: number;
    refreshIntervalMs?: number;
    env?: Readonly<Record<string, string>>;
}
export interface LegacyRuntimeConfig extends Omit<RuntimeConfig, "schemaVersion" | "timeoutMs" | "maxOutputBytes" | "refreshIntervalMs"> {
    schemaVersion?: 0;
    timeout?: number;
    maxOutput?: number;
    refreshInterval?: number;
}
export type RuntimeConfigInput = RuntimeConfig | LegacyRuntimeConfig;
export interface ConfigSources {
    explicitProjectConfig?: RuntimeConfigInput;
    projectSettings?: RuntimeConfigInput;
    globalSettings?: RuntimeConfigInput;
    environment?: NodeJS.ProcessEnv;
    defaults?: RuntimeConfigInput;
}
export interface RuntimeContext {
    cwd: string;
}
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
/** Convert schema 0/unversioned settings without mutating the rollback source. */
export declare function migrateConfig(input?: RuntimeConfigInput): RuntimeConfig;
/** Serialize current settings for a schema-0 rollback without mutating the current value. */
export declare function rollbackConfig(input: RuntimeConfig): LegacyRuntimeConfig;
/** Merge each setting independently, from the documented lowest to highest priority. */
export declare function resolveConfig(sources?: ConfigSources): Required<Pick<RuntimeConfig, "timeoutMs" | "maxOutputBytes">> & RuntimeConfig;
export declare function resolveRuntimeRoot(projectCwd: string, requestedRoot?: string, allowOutsideProject?: boolean): Promise<string>;
type BinaryResolver = typeof resolveBinary;
export interface RuntimeDependencies {
    resolveBinary?: BinaryResolver;
}
export interface FreshnessRuntime {
    run(args: readonly string[], context: RuntimeContext, options?: RunOptions): Promise<MachineEnvelope>;
    resolveRoot(context: RuntimeContext): Promise<string>;
    inspectIndexCompatibility?(context: RuntimeContext): Promise<IndexHealth>;
    rebuildIncompatibleIndex?(context: RuntimeContext, options?: RunOptions): Promise<MachineEnvelope>;
}
export interface FreshnessCoordinatorOptions {
    refreshIntervalMs?: number;
    now?: () => number;
}
export type IndexHealth = "ready" | "missing" | "incompatible";
export declare class FreshnessCoordinator {
    #private;
    constructor(options?: FreshnessCoordinatorOptions);
    markAffectedPath(path: string, cwd: string): void;
    markRootDirty(root: string): void;
    ensureFresh(runtime: FreshnessRuntime, context: RuntimeContext, options?: RunOptions): Promise<string>;
}
export declare class AstSgrepRuntime {
    #private;
    private readonly pi;
    readonly config: ReturnType<typeof resolveConfig>;
    constructor(pi: PiExec, sources?: ConfigSources, dependencies?: RuntimeDependencies);
    resolveRoot(context: RuntimeContext): Promise<string>;
    inspectIndexCompatibility(context: RuntimeContext): Promise<IndexHealth>;
    rebuildIncompatibleIndex(context: RuntimeContext, options?: RunOptions): Promise<MachineEnvelope>;
    run(args: readonly string[], context: RuntimeContext, options?: RunOptions): Promise<MachineEnvelope>;
    checkCompatibility(context: RuntimeContext, options?: RunOptions): Promise<MachineEnvelope>;
}
export {};
