/**
 * Session-scoped native Code Mode sessions.
 *
 * Primary path: in-process NAPI (`CodeModeSession` inside Node) — same model as
 * MCP linking core. Zero CLI spawn.
 *
 * Fallback: sticky `codemode-serve` child only when the `.node` addon is missing
 * (unsupported host / incomplete install). Doctor reports that as degraded.
 */
import type { MachineEnvelope } from "../runtime.js";
import { type StickyWorker } from "./dispatch.js";
import { type StickyWorkerOptions } from "./worker.js";
export type SessionPoolOptions = {
    /** Required only for CLI sticky fallback. */
    binary?: string;
    env?: NodeJS.ProcessEnv;
    timeoutMs?: number;
    root?: string;
    indexPath?: string;
    useEmbed?: boolean;
    limit?: number;
};
export type StickyStarter = (options: StickyWorkerOptions) => Promise<StickyWorker>;
export declare class NativeSessionPool {
    #private;
    constructor(startFn?: StickyStarter);
    configure(options: SessionPoolOptions): void;
    configured(): boolean;
    /** Active backend after first successful acquire. */
    backend(): "napi" | "cli" | "none";
    acquire(root: string): Promise<StickyWorker | null>;
    call(root: string, tool: string, args?: Record<string, unknown>, options?: {
        signal?: AbortSignal;
    }): Promise<MachineEnvelope>;
    invalidate(root: string): Promise<void>;
    shutdown(): Promise<void>;
}
/** Singleton for advanced hosts; tools registration uses a local pool. */
export declare const sharedNativePool: NativeSessionPool;
