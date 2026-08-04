/**
 * Session-scoped sticky native workers — one warm `codemode-serve` per project root.
 *
 * Like pi-codex-conversion's SharedCodeModeRuntime / host session: pay spawn +
 * SQLite open once per root for the Pi session, then all Code Mode programs,
 * direct tools, and freshness checks reuse the same Searcher.
 *
 * Still a CLI child (packaging constraint — see docs/codemode.md). Eliminating
 * the process boundary entirely needs a NAPI addon; this is the pragmatic max
 * without changing the release contract.
 */
import type { MachineEnvelope } from "../runtime.js";
import { type StickyWorker } from "./dispatch.js";
import { type StickyWorkerOptions } from "./worker.js";
export type SessionPoolOptions = {
    binary: string;
    env?: NodeJS.ProcessEnv;
    timeoutMs?: number;
};
export type StickyStarter = (options: StickyWorkerOptions) => Promise<StickyWorker>;
export declare class NativeSessionPool {
    #private;
    constructor(startFn?: StickyStarter);
    configure(options: SessionPoolOptions): void;
    configured(): boolean;
    /**
     * Acquire (or start) the sticky worker for a root. Does not take a call-level
     * AbortSignal — killing the session worker on one cancelled tool would thrash
     * every other concurrent call.
     */
    acquire(root: string): Promise<StickyWorker | null>;
    call(root: string, tool: string, args?: Record<string, unknown>, options?: {
        signal?: AbortSignal;
    }): Promise<MachineEnvelope>;
    /** Drop the worker for a root (e..g. after fatal protocol error). */
    invalidate(root: string): Promise<void>;
    shutdown(): Promise<void>;
}
/** Singleton used by the Pi extension for the process lifetime. */
export declare const sharedNativePool: NativeSessionPool;
