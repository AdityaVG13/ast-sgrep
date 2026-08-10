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
 * Run model-generated JavaScript against the typed `asgrep` connector.
 *
 * Trust model (OpenCode-style): Code Mode is an orchestration pattern, not an OS
 * jail. The Pi package already runs with the installing user's privileges. Authority
 * is the explicit `asgrep.*` surface passed into the program — same idea as
 * OpenCode CodeMode exposing only host-supplied tools. We intentionally do **not**
 * use `node:vm` / isolates: same-realm `AsyncFunction` is faster and enough for
 * composition (`Promise.all`, filter, shape).
 */
export declare function runCodemode(rawCode: string, asgrep: AsgrepConnector, options?: {
    timeoutMs?: number;
    signal?: AbortSignal;
    stats?: () => DispatchStats;
}): Promise<CodemodeRunResult>;
