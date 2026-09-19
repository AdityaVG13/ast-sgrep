/** Neat asgrep tool chrome for the Pi TUI and the model-visible content. */
export const ASGREP_PROMPT_SNIPPET = "Search this repo by intent, symbol, callers, defs, or pattern (asgrep; use without being asked)";
export const ASGREP_PROMPT_GUIDELINES = [
    "For any code lookup (find a function, callers, defs, intent, or structural pattern), call asgrep immediately. Do not wait for the user to mention ast-sgrep.",
    "Prefer the asgrep Code Mode tool. Write JavaScript: asgrep.search(\"query\"), asgrep.defs(\"Symbol\"), asgrep.callers(\"Symbol\"), asgrep.read({ refs }). Independent lookups: Promise.all. Return a small shaped value.",
    "Use grep only for exact log strings, filenames, or config keys. asgrep.edit does unique string replace plus targeted reindex; oldText must match exactly once.",
    "If a search returns 0 hits, use suggested_next or retry with asgrep.find, asgrep.defs, or asgrep.search(query, { in: \"src\" }).",
];
export function paint(theme, role, text, bold = false) {
    const body = bold && theme ? theme.bold(text) : text;
    return theme ? theme.fg(role, body) : body;
}
export function hitLocation(hit) {
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
export function hitLabel(hit) {
    const symbol = typeof hit.symbol === "string" ? hit.symbol : "";
    const kind = typeof hit.kind === "string" ? hit.kind : "";
    const preview = typeof hit.preview === "string" ? hit.preview.replace(/\s+/g, " ").trim() : "";
    return [symbol, kind, preview && preview.length < 80 ? preview : ""].filter(Boolean).join("  ");
}
export function header(theme, verb, bits) {
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
export function formatEditCall(params, theme) {
    const n = Array.isArray(params.edits) ? params.edits.length : 0;
    return header(theme, "edit", [params.path, n > 1 ? n + " edits" : undefined]);
}
export function formatReadCall(params, theme) {
    const target = params.path ?? params.ref;
    const range = params.start !== undefined ? "L" + params.start + "-L" + (params.end ?? "") : undefined;
    return header(theme, "read", [target, range]);
}
/** Model-visible text for an edit envelope: what changed, per file. */
export function formatEditResult(response, theme) {
    const edits = Array.isArray(response.edits) ? response.edits : [];
    const changed = edits.filter((e) => e.changed === true).length;
    const title = header(theme, "edit", [changed + "/" + edits.length + " changed"]);
    const rows = edits.slice(0, 12).map((e) => {
        const path = typeof e.path === "string" ? e.path : "?";
        const line = typeof e.line === "number" ? ":" + e.line : "";
        return paint(theme, "toolOutput", "  " + path + line);
    });
    return [title, ...rows].join("\n");
}
/** Model-visible text for a read envelope: the window contents themselves. */
export function formatReadResult(response, theme) {
    const windows = Array.isArray(response.windows) ? response.windows : [];
    if (windows.length === 0)
        return header(theme, "read", ["0 windows"]);
    const out = [];
    for (const w of windows.slice(0, 8)) {
        const path = typeof w.path === "string" ? w.path : "?";
        out.push(header(theme, "read", [path + "#L" + (w.start ?? 1) + "-L" + (w.end ?? "")]));
        const text = typeof w.text === "string" ? w.text : "";
        for (const line of text.split("\n").slice(0, 80))
            out.push(line);
    }
    if (windows.length > 8)
        out.push("… " + (windows.length - 8) + " more windows");
    return out.join("\n");
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
    if (hits.length === 0) {
        rows.push(paint(theme, "muted", "  0 hits"));
        const next = Array.isArray(response.suggested_next)
            ? response.suggested_next.filter((item) => typeof item === "string")
            : [];
        if (next.length > 0) {
            for (const query of next.slice(0, 4)) {
                rows.push(paint(theme, "toolOutput", `  try  ${query}`));
            }
        }
        else {
            rows.push(paint(theme, "toolOutput", "  try  asgrep.find(query) or asgrep.defs(symbol) or asgrep.search(query, { in: \"src\" })"));
        }
    }
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
function ansiLengthAt(text, index) {
    if (text.charCodeAt(index) !== 0x1b)
        return 0;
    const next = text[index + 1];
    if (next === "[") {
        let cursor = index + 2;
        while (cursor < text.length) {
            const code = text.charCodeAt(cursor);
            if (code >= 0x40 && code <= 0x7e)
                return cursor - index + 1;
            cursor += 1;
        }
        return text.length - index;
    }
    if (next === "]") {
        let cursor = index + 2;
        while (cursor < text.length) {
            if (text.charCodeAt(cursor) === 0x07)
                return cursor - index + 1;
            if (text.charCodeAt(cursor) === 0x1b && text[cursor + 1] === "\\")
                return cursor - index + 2;
            cursor += 1;
        }
        return text.length - index;
    }
    if (next === "P" || next === "X" || next === "^" || next === "_") {
        let cursor = index + 2;
        while (cursor < text.length) {
            if (text.charCodeAt(cursor) === 0x1b && text[cursor + 1] === "\\")
                return cursor - index + 2;
            cursor += 1;
        }
        return text.length - index;
    }
    return Math.min(2, text.length - index);
}
function cellWidthAt(text, index) {
    const code = text.charCodeAt(index);
    if (code === 0x09)
        return { width: 3, length: 1 };
    if (code >= 0xd800 && code <= 0xdbff)
        return { width: 2, length: 2 };
    if (code <= 0x1f || (code >= 0x7f && code <= 0x9f))
        return { width: 0, length: 1 };
    if (code <= 0x7e)
        return { width: 1, length: 1 };
    return { width: 2, length: 1 };
}
/** Local stand-in so we do not take a pi-tui dependency. Over-counts wide glyphs rather than under-count. */
export function visibleWidth(text) {
    let width = 0;
    for (let index = 0; index < text.length;) {
        const ansi = ansiLengthAt(text, index);
        if (ansi > 0) {
            index += ansi;
            continue;
        }
        const cell = cellWidthAt(text, index);
        width += cell.width;
        index += cell.length;
    }
    return width;
}
export function truncateToWidth(text, maxWidth, ellipsis = "...") {
    const limit = Math.max(0, maxWidth);
    if (limit <= 0)
        return "";
    if (visibleWidth(text) <= limit)
        return text;
    const ellipsisWidth = visibleWidth(ellipsis);
    if (ellipsisWidth >= limit)
        return ellipsis.slice(0, limit);
    const budget = limit - ellipsisWidth;
    let kept = "";
    let width = 0;
    for (let index = 0; index < text.length;) {
        const ansi = ansiLengthAt(text, index);
        if (ansi > 0) {
            kept += text.slice(index, index + ansi);
            index += ansi;
            continue;
        }
        const cell = cellWidthAt(text, index);
        if (width + cell.width > budget)
            break;
        kept += text.slice(index, index + cell.length);
        width += cell.width;
        index += cell.length;
    }
    // The cut can land inside a painted span — its closing SGR is past the
    // budget, so the ellipsis and everything after would inherit the open color.
    // Close it explicitly; harmless when the kept spans were already balanced.
    return kept + (kept.includes("\u001b[") ? "\u001b[0m" : "") + ellipsis;
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
    if (value === undefined) {
        return `${title}\n${paint(theme, "muted", "  (no return statement; add `return` to send a value to the model)")}`;
    }
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
    render(width) {
        const maxWidth = Math.max(1, width);
        if (this.#text.length === 0)
            return [""];
        return this.#text.split("\n").map((line) => truncateToWidth(line, maxWidth));
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
