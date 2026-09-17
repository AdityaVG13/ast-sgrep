/**
 * Index freshness coordination: dirty tracking, shared root-owned refreshes,
 * bounded per-caller waits, filesystem watchers.
 */
import { type FSWatcher } from "node:fs";
import { type MachineEnvelope, type RunOptions, type RuntimeContext } from "./types.js";
import { type IndexHealth } from "./index-health.js";
export { type IndexHealth } from "./index-health.js";
export interface FreshnessRuntime {
    run(args: readonly string[], context: RuntimeContext, options?: RunOptions): Promise<MachineEnvelope>;
    resolveRoot(context: RuntimeContext): Promise<string>;
    inspectIndexCompatibility?(context: RuntimeContext): Promise<IndexHealth>;
    rebuildIncompatibleIndex?(context: RuntimeContext, options?: RunOptions): Promise<MachineEnvelope>;
    /** Absolute database path whose SQLite/derived writes are owned by this runtime. */
    resolveIndexPath?(root: string): string;
    /**
     * Optional warm native call (session sticky pool). When present, freshness
     * prefers this over cold `run` for status/index — same Searcher as Code Mode.
     */
    nativeCall?(tool: string, args: Record<string, unknown>, context: RuntimeContext, options?: RunOptions): Promise<MachineEnvelope>;
    /** Enable low-latency external filesystem change detection for real runtimes. */
    watchExternalChanges?: boolean;
}
export interface FreshnessCoordinatorOptions {
    refreshIntervalMs?: number;
    /** Cap on how long one caller waits for an in-flight refresh (serve-stale after). */
    maxWaitMs?: number;
    now?: () => number;
    watchFactory?: FreshnessWatchFactory;
}
export type FreshnessWatchFactory = (root: string, options: {
    recursive: true;
    persistent: false;
    encoding: "utf8";
}, listener: (eventType: "rename" | "change", filename: string | null) => void) => FSWatcher;
export declare class FreshnessCoordinator {
    #private;
    constructor(options?: FreshnessCoordinatorOptions);
    markAffectedPath(path: string, cwd: string): void;
    markRootDirty(root: string): void;
    ensureFresh(runtime: FreshnessRuntime, context: RuntimeContext, options?: RunOptions): Promise<string>;
    shutdown(): void;
}
