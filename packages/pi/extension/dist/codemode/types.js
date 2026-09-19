/** Typed surface the model sees inside a Code Mode program (`asgrep.*`). */
/** Drop undefined keys — replaces spread-conditional arg-building chains. */
export function defined(args) {
    const out = {};
    for (const [key, value] of Object.entries(args))
        if (value !== undefined)
            out[key] = value;
    return out;
}
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
