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
export const CODEMODE_TYPES_FOR_MODEL = `
declare const asgrep: {
  search(input: { query: string; limit?: number; excerptLines?: number }): Promise<unknown>;
  semantic(input: { query: string; limit?: number; excerptLines?: number }): Promise<unknown>;
  chain(input: { query: string; limit?: number }): Promise<unknown>;
  defs(input: { symbol: string; limit?: number }): Promise<unknown>;
  callers(input: { symbol: string; limit?: number }): Promise<unknown>;
  imports(input: { module: string; limit?: number }): Promise<unknown>;
  indexStatus(): Promise<unknown>;
  indexRepo(input?: { force?: boolean }): Promise<unknown>;
  catalogSearch(input: { query: string }): Promise<unknown>;
  catalogDescribe(input: { name: string }): Promise<unknown>;
};
/** JS: Promise, JSON, Array, Object, Map, Set, Math. No require/process/fetch/fs. */
`.trim();
