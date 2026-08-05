import type { AsgrepConnector } from "./connector.js";
import type { DispatchStats } from "./dispatch.js";
export type CodemodeRunResult = {
    ok: boolean;
    result: unknown;
    logs: string[];
    error?: string;
    code: string;
    stats?: DispatchStats;
    wallMs: number;
};
/** Strip markdown fences and normalize to an async IIFE expression. */
export declare function normalizeCode(raw: string): string;
/**
 * Run model-generated JavaScript with only `asgrep` + safe builtins.
 *
 * Uses the shared microtask queue so host Promises from `asgrep.*` resolve under
 * `Promise.all`. Do not enable `microtaskMode: 'afterEvaluate'` — that isolates
 * queues and breaks cross-context await.
 */
export declare function runCodemode(rawCode: string, asgrep: AsgrepConnector, options?: {
    timeoutMs?: number;
    signal?: AbortSignal;
    stats?: () => DispatchStats;
}): Promise<CodemodeRunResult>;
