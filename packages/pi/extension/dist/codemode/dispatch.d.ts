/**
 * Same-tick call coalescing + typed batch / sticky-serve dispatch.
 *
 * Amdahl: serial cost is process spawn + SQLite open. Sticky serve kills spawn
 * for the whole Code Mode program; batch coalescing kills it per Promise.all wave.
 */
import type { MachineEnvelope } from "../runtime.js";
import type { ConnectorHost } from "./connector.js";
export type CodemodeToolCall = {
    tool: string;
    args: Record<string, unknown>;
};
export type DispatchStats = {
    waves: number;
    calls: number;
    batchedCalls: number;
    parallelSpawnCalls: number;
    stickyCalls: number;
    wallMs: number;
};
export type BatchResult = {
    results: Array<{
        id: string;
        ok: boolean;
        value?: unknown;
        error?: string;
    }>;
    mode?: string;
    wall_ms?: number;
    all_ok?: boolean;
};
export type StickyWorker = {
    call(tool: string, args: Record<string, unknown>, options?: {
        signal?: AbortSignal;
    }): Promise<MachineEnvelope>;
    batch(calls: Array<{
        id: string;
        tool: string;
        args: Record<string, unknown>;
    }>, options?: {
        signal?: AbortSignal;
    }): Promise<BatchResult>;
    end(): Promise<void>;
};
export type BatchCapableHost = ConnectorHost & {
    /** One-shot warm batch (codemode-batch). */
    runBatch?(calls: Array<{
        id: string;
        tool: string;
        args: Record<string, unknown>;
    }>, context: {
        cwd: string;
    }, options?: {
        signal?: AbortSignal;
    }): Promise<BatchResult>;
    /** Sticky NDJSON worker for the whole Code Mode program (preferred). */
    sticky?: StickyWorker | null;
};
/**
 * Wraps a host so Promise.all([asgrep.search, asgrep.defs, …]) collapses into
 * one microtask wave. Prefers sticky serve → one-shot batch → overlapped spawn.
 */
export declare function createCodemodeDispatcher(host: BatchCapableHost): {
    host: ConnectorHost;
    stats: () => DispatchStats;
    resetStats: () => void;
};
/** Build CLI argv for spawn fallback (typed path preferred). */
export declare function argvFor(tool: string, args: Record<string, unknown>): string[];
export declare function asEnvelope(value: unknown, command?: string): MachineEnvelope;
/** One-shot batch via stdin (no tempfile) when spawn-with-stdin is available. */
export declare function runNativeBatch(run: ConnectorHost["run"], calls: Array<{
    id: string;
    tool: string;
    args: Record<string, unknown>;
}>, context: {
    cwd: string;
}, options?: {
    signal?: AbortSignal;
}, writeBatch?: (body: string, context: {
    cwd: string;
}, options?: {
    signal?: AbortSignal;
}) => Promise<MachineEnvelope>): Promise<BatchResult>;
