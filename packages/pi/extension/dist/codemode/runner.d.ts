import type { AsgrepConnector } from "./connector.js";
import type { DispatchStats } from "./dispatch.js";
import { normalizeCode } from "./guest-api.js";
export { normalizeCode };
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
/** Spawn the warm standby isolate (session_start / pre-call). */
export declare function warmCodemodeSandbox(): Promise<void>;
/** Drop the standby isolate (tests / session shutdown). */
export declare function resetCodemodeSandboxForTests(): Promise<void>;
/**
 * Run model-generated JavaScript against the typed `asgrep` connector.
 *
 * Execution happens in a single-use `worker_threads` isolate: the guest gets a
 * `node:vm` context inside the worker; `asgrep`/`console` are built there; the
 * only host channel is a JSON postMessage bridge. Timeout and abort call
 * `worker.terminate()`, which is the only mechanism that actually stops a
 * detached guest microtask or a runaway heap (each worker is heap-capped).
 */
export declare function runCodemode(rawCode: string, asgrep: AsgrepConnector, options?: {
    timeoutMs?: number;
    signal?: AbortSignal;
    stats?: () => DispatchStats;
}): Promise<CodemodeRunResult>;
