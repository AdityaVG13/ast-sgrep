/** Typed surface the model sees inside a Code Mode program (`asgrep.*`). */
/** Host methods the program may invoke. Primary lookup methods first. */
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
    "doctor",
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
type Hit = { file: string; symbol?: string; kind?: string; score?: number; line?: number; ref?: string };
type Hits = { ok: boolean; hits: Hit[]; suggested_next?: string[] };
type Window = { path: string; ref: string; start: number; end: number; truncated: boolean; text: string };
declare const asgrep: {
  search(query: string | { query: string; limit?: number; in?: string; lang?: string; excerptLines?: number }): Promise<Hits>;
  find(query: string | { query: string; limit?: number }): Promise<Hits>;
  defs(symbol: string | { symbol: string; limit?: number }): Promise<Hits>;
  callers(symbol: string | { symbol: string; limit?: number }): Promise<Hits>;
  read(input: { path?: string; start?: number; end?: number; ref?: string; refs?: unknown[]; contextLines?: number }): Promise<{ ok: boolean; count: number; windows: Window[] }>;
  edit(path: string | { path?: string; oldText?: string; newText?: string; edits?: Array<{ path: string; oldText: string; newText: string }> }, oldText?: string, newText?: string): Promise<{ ok: boolean; changed: number }>;
  indexStatus(): Promise<{ ok: boolean }>;
};
/** Positional args work: search("auth"), defs("Foo"). Promise.all independent calls. 0 hits include suggested_next. Return a small value. */
`.trim();
