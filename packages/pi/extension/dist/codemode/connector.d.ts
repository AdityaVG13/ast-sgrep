import type { MachineEnvelope } from "../runtime.js";
import type { ChainArgs, SearchArgs } from "./types.js";
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
/**
 * Host-side connector: typed methods the sandbox calls. Each method maps to one
 * native CLI invocation. Independent methods may run concurrently via Promise.all.
 */
export declare function createAsgrepConnector(host: ConnectorHost, context: {
    cwd: string;
}, options?: {
    signal?: AbortSignal;
}): AsgrepConnector;
