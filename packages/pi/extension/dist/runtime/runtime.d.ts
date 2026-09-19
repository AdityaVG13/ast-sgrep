import { resolveBinary } from "ast-sgrep";
import { type MachineEnvelope, type PiExec, type RunOptions, type RuntimeContext } from "./types.js";
import { resolveConfig, type ConfigSources } from "./config.js";
import { type IndexHealth } from "./index-health.js";
export { CONFIG_SCHEMA_VERSION, DEFAULT_FRESHNESS_WAIT_MS, DEFAULT_MAX_OUTPUT_BYTES, DEFAULT_REFRESH_INTERVAL_MS, DEFAULT_TIMEOUT_MS, INDEX_FORMAT_VERSION, MACHINE_SCHEMA_VERSION, RUNTIME_VERSION, RuntimeError, } from "./types.js";
export type { ExecOptions, ExecResult, MachineEnvelope, PiExec, RunOptions, RuntimeContext, } from "./types.js";
export { migrateConfig, resolveConfig, rollbackConfig } from "./config.js";
export type { ConfigSources, LegacyRuntimeConfig, RuntimeConfig, RuntimeConfigInput, } from "./config.js";
export { FreshnessCoordinator, type FreshnessCoordinatorOptions, type FreshnessRuntime, type FreshnessWatchFactory, } from "./freshness.js";
export type { IndexHealth } from "./index-health.js";
export declare function resolveRuntimeRoot(projectCwd: string, requestedRoot?: string, allowOutsideProject?: boolean): Promise<string>;
type BinaryResolver = typeof resolveBinary;
export interface RuntimeDependencies {
    resolveBinary?: BinaryResolver;
}
export declare class AstSgrepRuntime {
    #private;
    private readonly pi;
    readonly watchExternalChanges = true;
    readonly config: ReturnType<typeof resolveConfig>;
    constructor(pi: PiExec, sources?: ConfigSources, dependencies?: RuntimeDependencies);
    resolveRoot(context: RuntimeContext): Promise<string>;
    resolveIndexPath(root: string): string;
    /**
     * Index format check. The configured binary is the sole authority on its own
     * schema window (exact-match: it refuses both older and newer), so the local
     * probe is only a pre-filter:
     *   missing/unreadable -> cheap no-spawn health answers;
     *   version == INDEX_FORMAT_VERSION (this release's shipped format) -> ready;
     *   otherwise -> consult the binary's declared index_schema_version once
     *   (cached), because a configured ASGREP_BIN/dev build may be newer than the
     *   shipped constant. Index newer than the binary -> INDEX_VERSION_TOO_NEW
     *   (never modified); older -> "incompatible" and rebuild migrates in place.
     */
    inspectIndexCompatibility(context: RuntimeContext): Promise<IndexHealth>;
    private supportedIndexFormat;
    rebuildIncompatibleIndex(context: RuntimeContext, options?: RunOptions): Promise<MachineEnvelope>;
    run(args: readonly string[], context: RuntimeContext, options?: RunOptions): Promise<MachineEnvelope>;
    /** Absolute path to the native binary (for sticky serve / stdin batch spawn). */
    resolveBinaryPath(options?: {
        env?: NodeJS.ProcessEnv;
    }): string;
    /** Merged process env for native Code Mode workers. */
    nativeEnv(options?: {
        env?: NodeJS.ProcessEnv;
    }): NodeJS.ProcessEnv;
    checkCompatibility(context: RuntimeContext, options?: RunOptions): Promise<MachineEnvelope>;
}
