/** Typed surface the model sees inside the Code Mode sandbox. */
/**
 * TypeScript declarations embedded in the `asgrep_codemode` tool description
 * so the model can write correct calls (Cloudflare createCodeTool style).
 */
export const CODEMODE_TYPES_FOR_MODEL = `
declare const asgrep: {
  /** Hybrid search (lexical + symbols + graph + semantic). Capsule JSON by default. */
  search(input: { query: string; limit?: number; excerptLines?: number }): Promise<unknown>;
  /** Semantic / embed pass only. */
  semantic(input: { query: string; limit?: number; excerptLines?: number }): Promise<unknown>;
  /** Expand callers/callees/imports neighborhood from a seed query. */
  chain(input: { query: string; limit?: number }): Promise<unknown>;
  /** Definition lookup for a known symbol. */
  defs(input: { symbol: string; limit?: number }): Promise<unknown>;
  /** Caller lookup for a known symbol. */
  callers(input: { symbol: string; limit?: number }): Promise<unknown>;
  /** Import / module lookup. */
  imports(input: { module: string; limit?: number }): Promise<unknown>;
  /** Index statistics. */
  indexStatus(): Promise<unknown>;
  /** Build or rebuild the project index. force=true for full rebuild. */
  indexRepo(input?: { force?: boolean }): Promise<unknown>;
};

/** Standard JS: Promise, JSON, Array, Object, Map, Set, Math. No require/process/fetch/fs. */
`.trim();
