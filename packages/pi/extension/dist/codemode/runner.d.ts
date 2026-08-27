import type { AsgrepConnector } from "./connector.js";
import type { DispatchStats } from "./dispatch.js";
/** Closed sum: success|failure — `ok:true` with `error` (or `ok:false` without) is unrepresentable. */
export type CodemodeRunSuccess = {
    ok: true;
    result: unknown;
    logs: string[];
    code: string;
    stats?: DispatchStats;
    wallMs: number;
};
export type CodemodeRunFailure = {
    ok: false;
    result: null;
    error: string;
    logs: string[];
    code: string;
    stats?: DispatchStats;
    wallMs: number;
};
export type CodemodeRunResult = CodemodeRunSuccess | CodemodeRunFailure;
/** Strip markdown fences and normalize to an async IIFE expression. */
export declare function normalizeCode(raw: string): string;
/** No-op: programs run in-process. Kept so session_start / tests stay stable. */
export declare function warmCodemodeSandbox(): Promise<void>;
/** No-op: there is no sticky Worker isolate to drop. */
export declare function resetCodemodeSandboxForTests(): Promise<void>;
/**
 * Run model-generated JavaScript against the typed `asgrep` connector.
 *
 * In-process `node:vm` (OpenCode/nicknisi: no Worker, no OS sandbox). `asgrep`
 * and `console` are built inside the context; the only host objects are a
 * JSON bridge and a log sink. Same trust as Pi `bash`.
 */
export declare function runCodemode(rawCode: string, asgrep: AsgrepConnector, options?: {
    timeoutMs?: number;
    signal?: AbortSignal;
    stats?: () => DispatchStats;
}): Promise<CodemodeRunResult>;
