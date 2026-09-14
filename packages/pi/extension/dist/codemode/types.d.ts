/** Typed surface the model sees inside a Code Mode program (`asgrep.*`). */
export type SearchArgs = {
    query: string;
    limit?: number;
    excerptLines?: number;
    format?: "capsule" | "agent";
    /** Directory or glob; injected as an `in:` query token. */
    in?: string;
    fileFilter?: string;
    file_filter?: string;
    lang?: string;
};
export type FindArgs = SearchArgs;
export type ReadArgs = {
    path?: string;
    start?: number;
    end?: number;
    ref?: string;
    refs?: unknown[];
    contextLines?: number;
    maxChars?: number;
};
export type EditArgs = {
    path?: string;
    oldText?: string;
    newText?: string;
    edits?: Array<{
        path: string;
        oldText: string;
        newText: string;
    }>;
};
export type ChainArgs = {
    query: string;
    limit?: number;
    excerptLines?: number;
};
/** Host methods the program may invoke. Primary lookup methods first. */
export declare const CODEMODE_HOST_METHODS: readonly ["search", "find", "read", "edit", "semantic", "chain", "defs", "callers", "imports", "indexStatus", "indexRepo", "doctor", "catalogSearch", "catalogDescribe"];
export type CodemodeHostMethod = (typeof CODEMODE_HOST_METHODS)[number];
/**
 * Compact TypeScript declarations for the `asgrep` tool description.
 * Four commands only — every token here is paid on every turn.
 * Return shapes are muscle memory (Blacksmith): field names, never values.
 * defs:/callers:/imports:/pattern:/blast: go through find or search prefixes.
 */
export declare const CODEMODE_TYPES_FOR_MODEL: string;
