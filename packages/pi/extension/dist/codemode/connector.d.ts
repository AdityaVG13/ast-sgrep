import type { MachineEnvelope } from "../runtime.js";
import type { ChainArgs, SearchArgs } from "./types.js";
import { type BatchCapableHost, type DispatchStats } from "./dispatch.js";
export type ConnectorHost = {
    run(args: readonly string[], context: {
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
};
export type ConnectorBundle = {
    asgrep: AsgrepConnector;
    stats: () => DispatchStats;
    resetStats: () => void;
};
/**
 * Host-side connector: typed methods the sandbox calls.
 *
 * Same-tick calls (Promise.all) are coalesced by CodemodeDispatcher so N
 * lookups share one warm `codemode-batch` process when available, otherwise
 * overlapped CLI spawns.
 */
export declare function createAsgrepConnector(host: BatchCapableHost, context: {
    cwd: string;
}, options?: {
    signal?: AbortSignal;
}): ConnectorBundle;
