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
/**
 * Run model-generated JavaScript against the typed `asgrep` connector.
 *
 * Model-generated code is not trusted with the extension host's ambient Node
 * authority. A dedicated worker contains CPU/microtask denial of service; its
 * VM hides `process`, module loading, and host constructors, with a JSON bridge
 * as the only exposed capability. This is not an OS sandbox, so deployments
 * requiring adversarial-code isolation should still restrict the Pi process.
 */
export declare function runCodemode(rawCode: string, asgrep: AsgrepConnector, options?: {
    timeoutMs?: number;
    signal?: AbortSignal;
    stats?: () => DispatchStats;
}): Promise<CodemodeRunResult>;
