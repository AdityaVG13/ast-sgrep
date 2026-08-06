/**
 * Sticky NDJSON Code Mode worker (`asgrep codemode-serve`).
 *
 * One process, one warm Searcher, for the entire Code Mode program — the biggest
 * Amdahl win over per-wave `codemode-batch` spawns.
 */
import type { MachineEnvelope } from "../runtime.js";
import { type StickyWorker } from "./dispatch.js";
export type StickyWorkerOptions = {
    binary: string;
    cwd: string;
    env?: NodeJS.ProcessEnv;
    signal?: AbortSignal;
    /** Kill worker when one request exceeds this duration (ms). */
    timeoutMs?: number;
};
export declare function startStickyWorker(options: StickyWorkerOptions): Promise<StickyWorker>;
/** One-shot batch via stdin (avoids tempfile). */
export declare function runBatchViaStdin(options: {
    binary: string;
    cwd: string;
    body: string;
    env?: NodeJS.ProcessEnv;
    signal?: AbortSignal;
    timeoutMs?: number;
}): Promise<MachineEnvelope>;
