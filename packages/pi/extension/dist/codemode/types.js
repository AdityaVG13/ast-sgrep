/** Typed surface the model sees inside a Code Mode program (`asgrep.*`). */
/** Host methods the program may invoke. Primary four first; the rest stay for tests and catalog tools. */
export const CODEMODE_HOST_METHODS = [
    "search",
    "find",
    "read",
    "edit",
    "semantic",
    "chain",
    "defs",
    "callers",
    "imports",
    "indexStatus",
    "indexRepo",
    "catalogSearch",
    "catalogDescribe",
];
/**
 * Compact TypeScript declarations for the `asgrep` tool description.
 * Four commands only — every token here is paid on every turn.
 * Return shapes are muscle memory (Blacksmith): field names, never values.
 * defs:/callers:/imports:/pattern:/blast: go through find or search prefixes.
 */
export const CODEMODE_TYPES_FOR_MODEL = `
type Hit = { file: string; symbol?: string; kind?: string; score?: number; line?: number; ref?: string; excerpt?: string };
type Hits = { ok: boolean; hits: Hit[] };
type Window = { path: string; ref: string; start: number; end: number; truncated: boolean; text: string };
declare const asgrep: {
  search(input: { query: string; limit?: number; excerptLines?: number }): Promise<Hits>;
  find(input: { query: string; limit?: number; excerptLines?: number }): Promise<Hits>;
  read(input: { path?: string; start?: number; end?: number; ref?: string; refs?: unknown[]; contextLines?: number }): Promise<{ ok: boolean; count: number; windows: Window[] }>;
  edit(input: { path?: string; oldText?: string; newText?: string; edits?: Array<{ path: string; oldText: string; newText: string }> }): Promise<{ ok: boolean; changed: number; edits: Array<{ path: string; changed: boolean }> }>;
};
/** Promise.all independent calls. Stage1 find (lexical/blast:); Stage2 search/read survivors. edit unique replace. */
`.trim();
