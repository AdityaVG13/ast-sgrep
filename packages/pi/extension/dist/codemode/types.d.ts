/** Typed surface the model sees inside the Code Mode sandbox. */
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
 * TypeScript declarations embedded in the `asgrep_codemode` tool description
 * so the model can write correct calls (Cloudflare createCodeTool style).
 */
export declare const CODEMODE_TYPES_FOR_MODEL: string;
