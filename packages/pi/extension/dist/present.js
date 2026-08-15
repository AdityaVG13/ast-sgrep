/** Neat asgrep tool chrome for the Pi TUI and the model-visible content. */
export const ASGREP_PROMPT_SNIPPET = "Search this repo by intent, symbol, callers, defs, pattern, or chain (in-process asgrep; use without being asked)";
export const ASGREP_PROMPT_GUIDELINES = [
    "For any code lookup (find a function, callers, defs, intent, structural pattern, or imports), call asgrep or asgrep_search immediately. Do not wait for the user to mention ast-sgrep.",
    "Prefer the asgrep Code Mode tool when you need more than one lookup, filtering, or parallel work. Write JavaScript that calls asgrep.search / semantic / chain / defs / callers / imports / indexStatus / indexRepo and return a small shaped value.",
    "Use grep only for exact log strings, filenames, or config keys. Use Pi write/edit to change files; asgrep does not mutate source.",
];
function paint(theme, role, text, bold = false) {
    const body = bold && theme ? theme.bold(text) : text;
    return theme ? theme.fg(role, body) : body;
}
function hitLocation(hit) {
    const file = String(hit.file ?? hit.path ?? "");
    const line = hit.start_line ?? hit.line ?? hit.lines;
    if (typeof line === "number")
        return `${file}:${line}`;
    if (typeof line === "string" && line.length > 0)
        return `${file}:${line}`;
    if (typeof hit.ref === "string" && hit.ref.length > 0)
        return hit.ref;
    return file || "?";
}
function hitLabel(hit) {
    const symbol = typeof hit.symbol === "string" ? hit.symbol : "";
    const kind = typeof hit.kind === "string" ? hit.kind : "";
    const preview = typeof hit.preview === "string" ? hit.preview.replace(/\s+/g, " ").trim() : "";
    return [symbol, kind, preview && preview.length < 80 ? preview : ""].filter(Boolean).join("  ");
}
function header(theme, verb, bits) {
    return [paint(theme, "toolTitle", "asgrep", true), paint(theme, "accent", verb), ...bits.filter((bit) => Boolean(bit))].join("  ·  ");
}
export function formatSearchCall(params, theme) {
    return header(theme, "search", [
        params.query ? JSON.stringify(params.query) : undefined,
        params.mode ?? "natural",
        params.limit !== undefined ? `limit ${params.limit}` : undefined,
        params.excerptLines ? `excerpt ${params.excerptLines}` : undefined,
    ]);
}
export function formatIndexCall(force, theme) {
    return header(theme, force ? "reindex" : "index", []);
}
export function formatStatusCall(theme) {
    return header(theme, "status", []);
}
export function formatCodemodeCall(code, theme) {
    const preview = code.trim().replace(/\s+/g, " ").slice(0, 80);
    return header(theme, "codemode", [`${preview}${code.trim().length > 80 ? "…" : ""}`]);
}
export function formatSearchResult(response, meta, theme) {
    const hits = Array.isArray(response.hits) ? response.hits : [];
    const title = header(theme, meta.command, [
        meta.query ? JSON.stringify(meta.query) : undefined,
        meta.mode,
        `${hits.length} hit${hits.length === 1 ? "" : "s"}`,
        meta.activationMs !== undefined ? `${meta.activationMs < 10 ? meta.activationMs.toFixed(2) : meta.activationMs.toFixed(1)}ms` : undefined,
        meta.backend,
    ]);
    const rows = hits.slice(0, 24).map((hit) => {
        const loc = hitLocation(hit);
        const label = hitLabel(hit);
        return paint(theme, "toolOutput", label ? `  ${loc}  ${label}` : `  ${loc}`);
    });
    if (hits.length > 24) {
        rows.push(paint(theme, "muted", `  … ${hits.length - 24} more`));
    }
    return [title, ...rows].join("\n");
}
export function formatStatusResult(response, theme) {
    const state = typeof response.status === "string" ? response.status
        : typeof response.index_status === "string" ? response.index_status
            : response.ok ? "ok" : "failed";
    const counts = response.counts && typeof response.counts === "object"
        ? Object.entries(response.counts).map(([key, value]) => `${key}=${String(value)}`).join("  ")
        : "";
    const backend = typeof response.backend === "string" ? response.backend : "";
    const title = header(theme, "status", [state, counts, backend]);
    return title;
}
export function formatIndexResult(command, response, theme) {
    const count = typeof response.count === "number" ? response.count
        : typeof response.total === "number" ? response.total
            : undefined;
    const tail = count === undefined ? "done" : `${count} file${count === 1 ? "" : "s"}`;
    return header(theme, command, [tail]);
}
function compactValue(value) {
    if (value === null || value === undefined)
        return String(value);
    if (typeof value !== "object")
        return String(value);
    if (Array.isArray(value))
        return `${value.length} item${value.length === 1 ? "" : "s"}`;
    const json = JSON.stringify(value);
    return json.length <= 80 ? json : `${json.slice(0, 79)}…`;
}
export function formatCodemodeResult(value, meta = {}, theme) {
    if (value && typeof value === "object" && Array.isArray(value.hits)) {
        const searchMeta = { command: "codemode" };
        if (meta.wallMs !== undefined)
            searchMeta.activationMs = meta.wallMs;
        if (meta.backend !== undefined)
            searchMeta.backend = meta.backend;
        return formatSearchResult(value, searchMeta, theme);
    }
    const bits = [];
    if (value && typeof value === "object") {
        const record = value;
        if (typeof record.hit_count === "number")
            bits.push(`${record.hit_count} hit${record.hit_count === 1 ? "" : "s"}`);
        else if (typeof record.node_count === "number")
            bits.push(`${record.node_count} node${record.node_count === 1 ? "" : "s"}`);
    }
    if (meta.backend === "napi")
        bits.push("in-process");
    else if (meta.backend === "cli")
        bits.push("cli-sticky");
    if (meta.stats && meta.stats.calls > 0) {
        const via = (meta.stats.stickyCalls ?? 0) > 0
            ? `native ${meta.stats.stickyCalls}`
            : meta.stats.batchedCalls > 0
                ? `batched ${meta.stats.batchedCalls}`
                : meta.stats.parallelSpawnCalls > 0
                    ? `parallel-spawn ${meta.stats.parallelSpawnCalls}`
                    : `${meta.stats.calls} call${meta.stats.calls === 1 ? "" : "s"}`;
        bits.push(via);
        if (meta.stats.waves > 1)
            bits.push(`${meta.stats.waves} waves`);
    }
    if (meta.wallMs !== undefined)
        bits.push(`${meta.wallMs}ms`);
    const title = header(theme, "codemode", bits);
    if (value && typeof value === "object" && !Array.isArray(value)) {
        const rows = Object.entries(value).slice(0, 16).map(([key, entry]) => paint(theme, "toolOutput", `  ${key}: ${compactValue(entry)}`));
        return [title, ...rows].join("\n");
    }
    if (Array.isArray(value)) {
        return [title, paint(theme, "toolOutput", `  ${value.length} value${value.length === 1 ? "" : "s"}`)].join("\n");
    }
    return `${title}\n${paint(theme, "toolOutput", `  ${compactValue(value)}`)}`;
}
/** Minimal pi-tui Text stand-in so we do not take a TUI package dependency. */
export class AsgrepText {
    #text;
    constructor(text = "") {
        this.#text = text;
    }
    setText(text) {
        this.#text = text;
    }
    invalidate() { }
    render(_width) {
        return this.#text.length === 0 ? [""] : this.#text.split("\n");
    }
}
export function presentText(formatted, last) {
    if (last instanceof AsgrepText) {
        last.setText(formatted);
        return last;
    }
    if (last && typeof last === "object" && last !== null && "setText" in last && typeof last.setText === "function") {
        last.setText(formatted);
        return last;
    }
    return new AsgrepText(formatted);
}
