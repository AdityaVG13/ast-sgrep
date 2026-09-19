/** Guest-call packing so the first shape a model tries actually works. */
import { type CodemodeHostMethod } from "./types.js";
export declare function resolveHostMethod(method: string): CodemodeHostMethod | undefined;
/** Turn positional guest calls into the host object shape. */
export declare function packGuestCall(method: string, args: unknown[]): Record<string, unknown>;
/** Accept query as a symbol alias and fold in:/fileFilter into the query string. */
export declare function coerceHostArgs(method: string, input: Record<string, unknown>): Record<string, unknown>;
export declare function applyQueryScope(query: string, args: Record<string, unknown>): string | undefined;
export declare function unknownMethodError(method: string): string;
export declare function timeoutHint(message: string): string;
/**
 * Strip fences and invoke a function expression, including non-async arrows.
 * A single expression with no `return` is returned automatically.
 * Bare statements still wrap in an async IIFE.
 */
export declare function normalizeCode(raw: string): string;
