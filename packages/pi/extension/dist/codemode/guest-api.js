/** Guest-call packing so the first shape a model tries actually works. */
import { CODEMODE_HOST_METHODS } from "./types.js";
const QUERY_METHODS = new Set([
    "search",
    "find",
    "semantic",
    "chain",
    "catalogSearch",
]);
const SYMBOL_METHODS = new Set(["defs", "callers"]);
/** Catalog / common-misname aliases. Resolved before packing so defs vs search keys stay correct. */
const METHOD_ALIASES = {
    index_status: "indexStatus",
    index_repo: "indexRepo",
    catalog_search: "catalogSearch",
    catalog_describe: "catalogDescribe",
    define: "defs",
    definition: "defs",
    definitions: "defs",
    grep: "find",
    keyword: "find",
    keywordSearch: "find",
    references: "callers",
};
function isPlainObject(value) {
    return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}
function mergeRest(base, rest) {
    if (!isPlainObject(rest))
        return base;
    return { ...rest, ...base };
}
function scopeToken(value) {
    if (typeof value !== "string")
        return undefined;
    const path = value.trim();
    if (!path || path.split(/[/\\]/u).includes(".."))
        return undefined;
    return path;
}
function editDistance(a, b) {
    const rows = a.length + 1;
    const cols = b.length + 1;
    const prev = new Array(cols);
    const cur = new Array(cols);
    for (let j = 0; j < cols; j++)
        prev[j] = j;
    for (let i = 1; i < rows; i++) {
        cur[0] = i;
        for (let j = 1; j < cols; j++) {
            const cost = a.charCodeAt(i - 1) === b.charCodeAt(j - 1) ? 0 : 1;
            cur[j] = Math.min((prev[j] ?? 0) + 1, (cur[j - 1] ?? 0) + 1, (prev[j - 1] ?? 0) + cost);
        }
        for (let j = 0; j < cols; j++)
            prev[j] = cur[j] ?? 0;
    }
    return prev[b.length] ?? a.length;
}
function suggestMethod(name) {
    const needle = name.toLowerCase();
    if (!needle)
        return undefined;
    const maxDistance = Math.max(1, Math.floor(needle.length / 3));
    let best;
    for (const candidate of CODEMODE_HOST_METHODS) {
        const lower = candidate.toLowerCase();
        const distance = lower.includes(needle) || needle.includes(lower)
            ? Math.min(1, editDistance(needle, lower))
            : editDistance(needle, lower);
        if (distance > maxDistance)
            continue;
        if (!best || distance < best.distance || (distance === best.distance && candidate.length < best.candidate.length)) {
            best = { candidate, distance };
        }
    }
    return best?.candidate;
}
export function resolveHostMethod(method) {
    if (CODEMODE_HOST_METHODS.includes(method)) {
        return method;
    }
    const aliased = METHOD_ALIASES[method] ?? METHOD_ALIASES[method.toLowerCase()];
    if (aliased)
        return aliased;
    const camel = method.replace(/_([a-z])/gu, (_all, letter) => letter.toUpperCase());
    if (camel !== method && CODEMODE_HOST_METHODS.includes(camel)) {
        return camel;
    }
    return undefined;
}
/** Turn positional guest calls into the host object shape. */
export function packGuestCall(method, args) {
    if (args.length === 0)
        return {};
    const first = args[0];
    const rest = args[1];
    if (typeof first === "string") {
        if (SYMBOL_METHODS.has(method))
            return mergeRest({ symbol: first }, rest);
        if (method === "imports")
            return mergeRest({ module: first }, rest);
        if (method === "read") {
            if (typeof args[1] === "number") {
                const packed = { path: first, start: args[1] };
                if (typeof args[2] === "number")
                    packed.end = args[2];
                return packed;
            }
            return mergeRest({ path: first }, rest);
        }
        if (method === "edit" && typeof args[1] === "string") {
            return { path: first, oldText: args[1], newText: typeof args[2] === "string" ? args[2] : "" };
        }
        if (method === "catalogDescribe")
            return mergeRest({ name: first }, rest);
        if (method === "indexRepo" || method === "indexStatus" || method === "doctor") {
            return isPlainObject(rest) ? { ...rest } : {};
        }
        return mergeRest({ query: first }, rest);
    }
    if (isPlainObject(first))
        return { ...first };
    throw new Error(`asgrep.${method}: pass a string or object (got ${typeof first}). Example: asgrep.search("auth") or asgrep.search({ query: "auth" })`);
}
/** Accept query as a symbol alias and fold in:/fileFilter into the query string. */
export function coerceHostArgs(method, input) {
    const args = { ...input };
    if (SYMBOL_METHODS.has(method) && typeof args.symbol !== "string") {
        const alias = args.query;
        if (typeof alias === "string" && alias.trim())
            args.symbol = alias.trim();
    }
    if (QUERY_METHODS.has(method)) {
        const query = typeof args.query === "string" ? args.query : "";
        const scoped = applyQueryScope(query, args);
        if (scoped)
            args.query = scoped;
    }
    return args;
}
export function applyQueryScope(query, args) {
    const scope = scopeToken(args.in) ?? scopeToken(args.fileFilter) ?? scopeToken(args.file_filter);
    if (!scope)
        return query.trim() ? query : undefined;
    if (query.split(/\s+/u).some((token) => token.startsWith("in:")))
        return query;
    const trimmed = query.trim();
    return trimmed ? `in:${scope} ${trimmed}` : `in:${scope}`;
}
export function unknownMethodError(method) {
    const close = suggestMethod(method);
    const hint = close ? ` Did you mean ${close}?` : "";
    return `unknown asgrep method '${method}'.${hint} Use search, find, defs, callers, read, edit, indexStatus. Example: asgrep.search("auth")`;
}
export function timeoutHint(message) {
    if (!/timeout after \d+ms|timed out after \d+ms/i.test(message))
        return message;
    return `${message}; narrow with asgrep.search(query, { in: "src" }), lower limit, or split the program`;
}
function looksLikeBareExpression(code) {
    const trimmed = code.trim().replace(/;\s*$/u, "");
    if (!trimmed || /\breturn\b/.test(trimmed))
        return false;
    if (/^(?:const|let|var|function|class|if|for|while|switch|try|async|import|export)\b/u.test(trimmed))
        return false;
    if (/;\s*\S/u.test(trimmed))
        return false;
    return true;
}
/**
 * Strip fences and invoke a function expression, including non-async arrows.
 * A single expression with no `return` is returned automatically.
 * Bare statements still wrap in an async IIFE.
 */
export function normalizeCode(raw) {
    let code = raw.trim();
    if (code.startsWith("```")) {
        code = code.replace(/^```(?:javascript|js|typescript|ts)?\s*/i, "").replace(/\s*```$/u, "").trim();
    }
    const trimmed = code.replace(/;\s*$/u, "");
    if (/^async\s*(?:function\b|\()/u.test(trimmed)) {
        return `(${trimmed})()`;
    }
    if (/^function\b/u.test(trimmed)) {
        return `(async ${trimmed})()`;
    }
    if (/^(?:\([^)]*\)|[A-Za-z_$][\w$]*)\s*=>/u.test(trimmed)) {
        return `(async ${trimmed})()`;
    }
    if (looksLikeBareExpression(code)) {
        return `(async () => {\nreturn ${trimmed}\n})()`;
    }
    return `(async () => {\n${code}\n})()`;
}
