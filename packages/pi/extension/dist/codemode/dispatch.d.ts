/**
 * Same-tick call coalescing + optional warm batch process.
 *
 * Amdahl: for N independent Code Mode tool calls started in one Promise.all,
 * one process (warm Searcher / parallel SQLite readers) beats N cold CLI spawns.
 * Serial fraction ≈ process start + SQLite open; parallel fraction ≈ search work.
 */
import type { ConnectorHost } from "./connector.js";
export type DispatchStats = {
    waves: number;
    calls: number;
    batchedCalls: number;
    parallelSpawnCalls: number;
    wallMs: number;
};
export type BatchCapableHost = ConnectorHost & {
    /** Optional: one warm process for many tool calls. */
    runBatch?(calls: Array<{
        id: string;
        tool: string;
        args: Record<string, unknown>;
    }>, context: {
        cwd: string;
    }, options?: {
        signal?: AbortSignal;
    }): Promise<{
        results: Array<{
            id: string;
            ok: boolean;
            value?: unknown;
            error?: string;
        }>;
        mode?: string;
        wall_ms?: number;
    }>;
};
/**
 * Wraps a ConnectorHost so Promise.all([asgrep.search, asgrep.defs, …]) collapses
 * into one microtask wave and prefers a single batch process when available.
 */
export declare function createCodemodeDispatcher(host: BatchCapableHost): {
    host: ConnectorHost;
    stats: () => DispatchStats;
    resetStats: () => void;
};
/** Runtime helper: write requests file and invoke `asgrep codemode-batch`. */
export declare function runNativeBatch(run: ConnectorHost["run"], calls: Array<{
    id: string;
    tool: string;
    args: Record<string, unknown>;
}>, context: {
    cwd: string;
}, options?: {
    signal?: AbortSignal;
}): Promise<{
    results: Array<{
        id: string;
        ok: boolean;
        value?: unknown;
        error?: string;
    }>;
    mode?: string;
    wall_ms?: number;
}>;
