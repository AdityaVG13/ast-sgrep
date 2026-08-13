/** Typed surface the model sees inside a Code Mode program (`asgrep.*`). */
export type SearchArgs = {
    query: string;
    limit?: number;
    excerptLines?: number;
    format?: "capsule" | "agent";
};
export type ChainArgs = {
    query: string;
    limit?: number;
    excerptLines?: number;
};
/**
 * Compact TypeScript declarations for the `asgrep` tool description.
 * Keep short — every token here is paid on every turn (schema landfill lesson
 * from pi-codex-conversion: compose inside Code Mode, don't dump 17 schemas).
 */
export declare const CODEMODE_TYPES_FOR_MODEL: string;
