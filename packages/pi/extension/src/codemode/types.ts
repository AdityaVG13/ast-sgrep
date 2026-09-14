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
  edits?: Array<{ path: string; oldText: string; newText: string }>;
};

export type ChainArgs = {
  query: string;
  limit?: number;
  excerptLines?: number;
};

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
] as const;

export type CodemodeHostMethod = (typeof CODEMODE_HOST_METHODS)[number];

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
