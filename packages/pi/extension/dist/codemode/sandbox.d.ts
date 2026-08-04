import type { AsgrepConnector } from "./connector.js";
export type CodemodeRunResult = {
    ok: boolean;
    result: unknown;
    logs: string[];
    error?: string;
    code: string;
};
/** Strip markdown fences and normalize to an async IIFE expression. */
export declare function normalizeCode(raw: string): string;
/**
 * Run model-generated JavaScript with only `asgrep` + safe builtins.
 *
 * This is a capability sandbox (no require/process/fetch), not an OS security
 * boundary — same trust model as the Pi package itself.
 */
export declare function runCodemode(rawCode: string, asgrep: AsgrepConnector, options?: {
    timeoutMs?: number;
}): Promise<CodemodeRunResult>;
