/** Typed surface the model sees inside a Code Mode program (`asgrep.*`). */

/** Drop undefined keys — replaces spread-conditional arg-building chains. */
export function defined<T extends Record<string, unknown>>(args: T): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(args)) if (value !== undefined) out[key] = value;
  return out;
}

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
  edits?: Array<{ path?: string; oldText: string; newText: string }>;
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
/**
 * Always-on API cheat sheet for the Code Mode tool description.
 *
 * Deliberately minimal: every token here rides in the system prompt of every
 * request. The full per-method schema is one call away through
 * `asgrep.catalogSearch(query)` / `asgrep.catalogDescribe(name)`, which returns
 * the same shapes from the native catalog, so the model pays for the reference
 * only when it needs it.
 */
export const CODEMODE_TYPES_FOR_MODEL = `
asgrep.search(query|{query,in,lang,limit,excerptLines}) | find(q) | semantic(q) | defs(sym) | callers(sym)
 | imports(mod) | chain(q) | read({path|ref|refs,start,end}) | edit({path,oldText,newText}|{edits}) | indexStatus()
hits[{file,ref,symbol,kind,preview}] | windows[{path,start,end,text}] | catalogDescribe("name") for schemas
`.trim();
