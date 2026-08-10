import type { MachineEnvelope } from "../runtime.js";
import type { ChainArgs, SearchArgs } from "./types.js";
import { type BatchCapableHost, type DispatchStats } from "./dispatch.js";
/**
 * Spawn/CLI transport. Hosts provide argv `run` only — never a typed twin.
 * Typed entry lives solely on {@link DispatchSurface} (dispatcher output).
 */
export type ConnectorHost = {
    run(args: readonly string[], context: {
        cwd: string;
    }, options?: {
        signal?: AbortSignal;
    }): Promise<MachineEnvelope>;
};
/**
 * Trusted typed dispatch after coalescing. `call` is required; no argv peer
 * that can disagree with tool+args.
 */
export type DispatchSurface = {
    call(tool: string, args: Record<string, unknown>, context: {
        cwd: string;
    }, options?: {
        signal?: AbortSignal;
    }): Promise<MachineEnvelope>;
};
export type AsgrepConnector = {
    search(input: SearchArgs): Promise<MachineEnvelope>;
    semantic(input: SearchArgs): Promise<MachineEnvelope>;
    chain(input: ChainArgs): Promise<MachineEnvelope>;
    defs(input: {
        symbol: string;
        limit?: number;
        excerptLines?: number;
    }): Promise<MachineEnvelope>;
    callers(input: {
        symbol: string;
        limit?: number;
        excerptLines?: number;
    }): Promise<MachineEnvelope>;
    imports(input: {
        module: string;
        limit?: number;
        excerptLines?: number;
    }): Promise<MachineEnvelope>;
    indexStatus(): Promise<MachineEnvelope>;
    indexRepo(input?: {
        force?: boolean;
    }): Promise<MachineEnvelope>;
    /** Progressive discovery (like deferred tools) — list/filter available asgrep tools. */
    catalogSearch(input: {
        query: string;
    }): Promise<MachineEnvelope>;
    catalogDescribe(input: {
        name: string;
    }): Promise<MachineEnvelope>;
};
export type ConnectorBundle = {
    asgrep: AsgrepConnector;
    stats: () => DispatchStats;
    resetStats: () => void;
};
/**
 * Host-side connector: typed methods the Code Mode program calls.
 *
 * Same-tick calls (Promise.all) are coalesced by CodemodeDispatcher so N
 * lookups share sticky serve / one warm batch process when available.
 */
export declare function createAsgrepConnector(host: BatchCapableHost, context: {
    cwd: string;
}, options?: {
    signal?: AbortSignal;
}): ConnectorBundle;
