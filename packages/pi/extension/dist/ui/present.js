/** Neat asgrep tool chrome for the Pi TUI and the model-visible content. */
export const ASGREP_PROMPT_SNIPPET = "Code search by intent, symbol, defs, callers, pattern (use without being asked)";
export const ASGREP_PROMPT_GUIDELINES = [
    "Any code lookup: call asgrep first; read code with asgrep_read, change it with asgrep_edit.",
    "Compose in Code Mode (search/defs/read, Promise.all, small shaped value); grep only exact strings/filenames. Bound with in:\"path\"; on 0 hits use suggested_next.",
];
/**
 * Guideline variant for hosts that already ship read/edit built in: our
 * one-shot file tools are inactive there, so naming them would point the
 * model at tools it cannot see. The host's own builtins describe
 * themselves; this keeps only the asgrep-first routing plus the shared
 * composition guideline.
 */
export const ASGREP_PROMPT_GUIDELINES_HOST_FILES = [
    "Any code lookup: call asgrep first.",
    ASGREP_PROMPT_GUIDELINES[1],
];
export function paint(theme, role, text, bold = false) {
    const body = bold && theme ? theme.bold(text) : text;
    return theme ? theme.fg(role, body) : body;
}
export function hitLocation(hit) {
    const file = sanitizeContent(String(hit.file ?? hit.path ?? ""));
    const line = hit.start_line ?? hit.line ?? hit.lines;
    if (typeof line === "number")
        return `${file}:${line}`;
    if (typeof line === "string" && line.length > 0)
        return `${file}:${sanitizeContent(line)}`;
    if (typeof hit.ref === "string" && hit.ref.length > 0)
        return sanitizeContent(hit.ref);
    return file || "?";
}
/** A caller may return arbitrary data named `hits`; only locations are search hits. */
export function hitsOf(value) {
    const hits = value && typeof value === "object" ? value.hits : undefined;
    return Array.isArray(hits) && hits.every(hit => hit && typeof hit === "object" &&
        (typeof hit.file === "string" || typeof hit.path === "string" || typeof hit.ref === "string"))
        ? hits : undefined;
}
/** Hard caps for one result: rows shown, excerpt lines per hit, longest preview. */
const MAX_HIT_ROWS = 24;
const MAX_EXCERPT_LINES_OUT = 12;
const MAX_PREVIEW_CHARS = 96;
const MAX_RESULT_CHARS = 8_000;
export function hitLabel(hit) {
    const symbol = typeof hit.symbol === "string" ? sanitizeContent(hit.symbol) : "";
    const kind = typeof hit.kind === "string" ? sanitizeContent(hit.kind) : "";
    const raw = typeof hit.preview === "string" ? sanitizeContent(hit.preview).replace(/\s+/g, " ").trim() : "";
    // Truncate instead of dropping: a long preview used to vanish entirely, so a
    // hit could carry no hint at all about what it contained.
    const preview = raw.length > MAX_PREVIEW_CHARS ? raw.slice(0, MAX_PREVIEW_CHARS - 1) + "\u2026" : raw;
    return [symbol, kind, preview].filter(Boolean).join("  ");
}
export function formatEditResult(response, theme) {
    const edits = Array.isArray(response.edits) ? response.edits : [];
    const changed = edits.filter((e) => e.changed === true).length;
    const rows = ["edit: " + changed + "/" + edits.length + " changed"];
    for (const entry of edits.slice(0, 12)) {
        const path = sanitizeContent(typeof entry.path === "string" ? entry.path : "?");
        const line = typeof entry.line === "number" ? ":" + entry.line : "";
        rows.push("  " + path + line);
    }
    return rows.join("\n");
}
/** Model-visible text for a read envelope: the window contents themselves. */
export function formatReadResult(response, theme, maxChars = MAX_RESULT_CHARS) {
    const windows = Array.isArray(response.windows) ? response.windows : [];
    if (windows.length === 0)
        return "read: 0 windows";
    const out = [];
    let displayed = 0;
    const moreWindows = (count) => count > 0 ? [`… ${count} more windows`] : [];
    for (const w of windows.slice(0, 8)) {
        const path = sanitizeContent(typeof w.path === "string" ? w.path : "?");
        const text = sanitizeContent(typeof w.text === "string" ? w.text : "");
        const lines = text.split("\n");
        const start = w.start ?? 1;
        const render = (count) => {
            const end = count < lines.length && typeof start === "number" ? start + Math.max(0, count - 1) : (w.end ?? "");
            return [path + "#L" + start + "-L" + end, ...lines.slice(0, count),
                ...(count < lines.length || w.truncated === true ? ["… (truncated; read a smaller window)"] : [])].join("\n");
        };
        // Reserve the omission notice and fit whole lines before finalizing refs;
        // a later character clip would otherwise overstate the displayed range.
        const suffix = moreWindows(windows.length - displayed - 1);
        let count = Math.min(lines.length, 80);
        const fits = () => [...out, render(count), ...suffix].join("\n").length <= maxChars;
        while (count > 0 && !fits())
            count--;
        if (!fits())
            break;
        out.push(render(count));
        displayed++;
        if (count < Math.min(lines.length, 80))
            break;
    }
    return boundedText([...out, ...moreWindows(windows.length - displayed)].join("\n"), maxChars);
}
/**
 * Model-facing result text is deliberately lean: the tool call already carries
 * the query/mode, the TUI card renders timing and backend for the human, and
 * every token here is re-sent with the whole transcript. The first line is the
 * only chrome: "<command>: <payload summary>".
 */
export function formatSearchResult(response, meta, theme) {
    const hits = Array.isArray(response.hits) ? response.hits : [];
    // Capsules carry an excerpt whether or not the caller asked; render it only
    // when they did, or every defs/imports answer pays for body text nobody
    // requested (measured: 2.5k tokens of unasked-for excerpts before the guard).
    const excerptBudget = Math.max(0, Math.min(meta.excerptLines ?? 0, MAX_EXCERPT_LINES_OUT));
    let excerptLinesLeft = excerptBudget * Math.min(hits.length, MAX_HIT_ROWS);
    const rows = [`${meta.command}: ${hits.length} hit${hits.length === 1 ? "" : "s"}`];
    for (const hit of hits.slice(0, MAX_HIT_ROWS)) {
        const loc = hitLocation(hit);
        const label = hitLabel(hit);
        // Single-space fields: same payload, fewer tokens per row, and every row
        // is re-sent with the transcript on each turn.
        rows.push(label ? `  ${loc} ${label}` : `  ${loc}`);
        // Body excerpts only exist when the caller asked for them (excerptLines);
        // they used to be dropped here, so asking cost nothing and returned less.
        const excerpt = hit.excerpt;
        if (excerptLinesLeft > 0 && typeof excerpt === "string" && excerpt.trim() !== "") {
            for (const line of sanitizeContent(excerpt).split("\n")) {
                if (excerptLinesLeft === 0)
                    break;
                if (line.trim() === "")
                    continue;
                rows.push("    " + line.replace(/\s+$/u, ""));
                excerptLinesLeft -= 1;
            }
        }
    }
    if (hits.length === 0) {
        // A chain answer is nodes+edges, not hits: depth, site, symbol per row.
        const nodes = Array.isArray(response.nodes)
            ? (response.nodes)
            : [];
        if (nodes.length > 0) {
            rows[0] = `${meta.command}: ${nodes.length} nodes`;
            for (const node of nodes.slice(0, 24)) {
                const file = sanitizeContent(typeof node.file === "string" ? node.file : "?");
                const line = typeof node.line_start === "number" ? ":" + node.line_start : "";
                const depth = typeof node.depth === "number" ? "d" + node.depth + " " : "";
                const symbol = typeof node.symbol === "string" ? " " + sanitizeContent(node.symbol) : "";
                rows.push(`  ${depth}${file}${line}${symbol}`);
            }
            return rows.join("\n");
        }
        const next = Array.isArray(response.suggested_next)
            ? response.suggested_next.filter((item) => typeof item === "string")
            : [];
        for (const query of next.slice(0, 3))
            rows.push(`  try: ${sanitizeContent(query)}`);
    }
    if (hits.length > 24)
        rows.push(`  … ${hits.length - 24} more`);
    return rows.join("\n");
}
export function formatStatusResult(response, theme) {
    const state = typeof response.status === "string" ? response.status
        : typeof response.index_status === "string" ? response.index_status
            : response.ok ? "ok" : "failed";
    const counts = response.counts && typeof response.counts === "object"
        ? Object.entries(response.counts).map(([key, value]) => `${key}=${String(value)}`).join(" ")
        : "";
    const backend = typeof response.backend === "string" ? response.backend : "";
    return ["status: " + state, counts, backend].filter(Boolean).join(" ");
}
export function formatIndexResult(command, response, theme) {
    const count = typeof response.count === "number" ? response.count
        : typeof response.total === "number" ? response.total
            : typeof response.files_indexed === "number" ? response.files_indexed
                : undefined;
    return count === undefined ? `${command}: done` : `${command}: ${count} file${count === 1 ? "" : "s"}`;
}
/** Longest escape sequence we will skip as a unit; longer runs are treated as
 * text so an unterminated sequence cannot swallow the rest of a line. */
const MAX_ESCAPE_LENGTH = 512;
/**
 * Length of the terminal escape starting at `index`, or 0 when there is none.
 *
 * A lone or unterminated ESC measures zero and everything after it is text,
 * which is what pi does (measured: "\u001b[31" is three cells, ESC included as
 * zero). Skipping unterminated sequences whole would under-count, and pi kills
 * the process for any rendered line wider than the terminal.
 */
function ansiLengthAt(text, index) {
    if (text.charCodeAt(index) !== 0x1b)
        return 0;
    const limit = Math.min(text.length, index + MAX_ESCAPE_LENGTH);
    const next = text[index + 1];
    if (next === "[") {
        for (let cursor = index + 2; cursor < limit; cursor += 1) {
            const code = text.charCodeAt(cursor);
            if (code >= 0x40 && code <= 0x7e)
                return cursor - index + 1;
        }
        return 1;
    }
    if (next === "]") {
        for (let cursor = index + 2; cursor < limit; cursor += 1) {
            if (text.charCodeAt(cursor) === 0x07)
                return cursor - index + 1;
            if (text.charCodeAt(cursor) === 0x1b && text[cursor + 1] === "\\")
                return cursor - index + 2;
        }
        return 1;
    }
    if (next === "P" || next === "X" || next === "^" || next === "_") {
        for (let cursor = index + 2; cursor < limit; cursor += 1) {
            if (text.charCodeAt(cursor) === 0x1b && text[cursor + 1] === "\\")
                return cursor - index + 2;
        }
        return 1;
    }
    return 1;
}
/**
 * Strip terminal control sequences and C0/C1 controls from untrusted content
 * (tool output, code windows, paths) before this extension paints it. Without
 * this a stray ESC in a file would sit inside our own SGR span, leaving an
 * unterminated color and mis-measuring the row's width.
 */
export function sanitizeContent(text) {
    let out = "";
    for (let index = 0; index < text.length;) {
        const ansi = ansiLengthAt(text, index);
        if (ansi > 0) {
            index += ansi;
            continue;
        }
        const code = text.charCodeAt(index);
        if (code === 0x09 || code === 0x0a || (code >= 0x20 && code !== 0x7f && !(code >= 0x80 && code <= 0x9f))) {
            out += text[index];
        }
        index += 1;
    }
    return out;
}
/**
 * Chrome glyphs this extension draws itself. Pi measures every one of them as a
 * single cell (verified against `visibleWidth` from @earendil-works/pi-tui), so
 * frame geometry can stay exact while everything else rounds UP.
 */
const ONE_CELL_CHROME = /^[\u00b7\u00d7\u2022\u2026\u2192\u23f5\u23f8\u2500-\u257f\u25a0-\u25cf\u2591-\u2593\u26d3\u2713\u2714\u2717\u276f\u2588]$/u;
/**
 * Conservative display width on pi's scale: never under-counts what pi measures.
 *
 * Pi expands tabs to three spaces and measures grapheme clusters with East Asian
 * Width (CJK/fullwidth/emoji = 2); lone combining marks, variation selectors and
 * joiners are zero there. Re-deriving that table here would be a second source of
 * truth that can drift, so this counts only what the card itself draws exactly
 * (one cell) and rounds every other non-ASCII code point UP to two. A line that
 * measures `width` here is therefore at most `width` in the terminal: over-counting
 * can only leave slack before the right border, while under-counting is what pi
 * kills the process for ("Rendered line N exceeds terminal width").
 */
export function displayWidth(text) {
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
function cellWidthAt(text, index) {
    const code = text.charCodeAt(index);
    if (code === 0x09)
        return { width: 3, length: 1 };
    if (code <= 0x1f || (code >= 0x7f && code <= 0x9f))
        return { width: 0, length: 1 };
    if (code <= 0x7e)
        return { width: 1, length: 1 };
    if (code >= 0xd800 && code <= 0xdbff)
        return { width: 2, length: 2 };
    if (ONE_CELL_CHROME.test(text[index] ?? ""))
        return { width: 1, length: 1 };
    return { width: 2, length: 1 };
}
/** Local stand-in so we do not take a pi-tui dependency. Over-counts wide glyphs rather than under-count. */
export function visibleWidth(text) {
    return displayWidth(text);
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
/** Wire-envelope fields that describe the transport, not the answer. Showing
 * them turns a one-line answer into a JSON dump of our own protocol. */
const TRANSPORT_KEYS = new Set([
    "tool",
    "command",
    "schema_version",
    "ok",
    "ref",
    "refs",
    "index_path",
    "expand_hint",
    "snapshot",
    "backend",
    "wall_ms",
    "exit_code",
    "prevented_read_bytes",
    "read_bytes_estimate",
    "returned_excerpt_bytes",
]);
/**
 * One summary line per interesting entry of a shaped value: known shapes get a
 * sentence ("3 windows · path:1-340"), everything else `key: value` with the
 * value compacted. Transport fields are dropped rather than rendered.
 */
export function summarizeValue(value, limit = 4) {
    if (value === null || value === undefined || typeof value !== "object" || Array.isArray(value)) {
        return [compactValue(value)];
    }
    const recordValue = value;
    const rows = [];
    const hits = hitsOf(value);
    const windows = Array.isArray(recordValue.windows)
        ? recordValue.windows
        : undefined;
    if (hits)
        rows.push(hits.length + " hit" + (hits.length === 1 ? "" : "s"));
    if (windows && windows.length > 0) {
        const first = windows[0] ?? {};
        const where = typeof first.path === "string" ? sanitizeContent(first.path) : "";
        const range = typeof first.start === "number"
            ? ":" + first.start + (typeof first.end === "number" ? "-" + first.end : "")
            : "";
        rows.push(windows.length + " window" + (windows.length === 1 ? "" : "s") + (where ? " · " + where + range : ""));
    }
    for (const [key, entry] of Object.entries(recordValue)) {
        if (rows.length >= limit)
            break;
        if (TRANSPORT_KEYS.has(key))
            continue;
        if (hits && key === "hits")
            continue;
        if (windows && (key === "windows" || key === "count"))
            continue;
        rows.push(key + ": " + compactValue(entry));
    }
    return rows;
}
function compactValue(value) {
    if (value === null || value === undefined)
        return String(value);
    if (typeof value !== "object")
        return sanitizeContent(String(value));
    if (Array.isArray(value))
        return `${value.length} item${value.length === 1 ? "" : "s"}`;
    const json = JSON.stringify(value);
    return json.length <= 80 ? json : `${json.slice(0, 79)}…`;
}
/** Model output is data, not the TUI's lossy summary. Keep a visible size bound. */
function codemodePayload(value) {
    return sanitizeContent(typeof value === "string" ? value : JSON.stringify(value, null, 2) ?? String(value));
}
export function boundedText(text, maxChars = MAX_RESULT_CHARS, notice = "\n… (truncated)") {
    if (text.length <= maxChars)
        return text;
    if (maxChars <= notice.length)
        return notice.slice(0, Math.max(0, maxChars));
    let end = maxChars - notice.length;
    const last = text.charCodeAt(end - 1);
    if (last >= 0xd800 && last <= 0xdbff)
        end--;
    return text.slice(0, end) + notice;
}
function boundedCodemode(text) {
    return boundedText(text, MAX_RESULT_CHARS, "\n… (truncated; return fewer items or smaller windows)");
}
export function formatCodemodeResult(value, meta = {}, theme) {
    const result = value && typeof value === "object" ? value : undefined;
    const hits = hitsOf(value);
    const envelope = result?.tool === "asgrep" && typeof result.schema_version === "string" && typeof result.ok === "boolean";
    const bits = [];
    if (hits)
        bits.push(`${hits.length} hit${hits.length === 1 ? "" : "s"}`);
    if (value && typeof value === "object") {
        const record = value;
        if (!hits && typeof record.hit_count === "number")
            bits.push(`${record.hit_count} hit${record.hit_count === 1 ? "" : "s"}`);
        else if (typeof record.node_count === "number")
            bits.push(`${record.node_count} node${record.node_count === 1 ? "" : "s"}`);
    }
    if (meta.stats && meta.stats.calls > 0) {
        bits.push(`${meta.stats.calls} call${meta.stats.calls === 1 ? "" : "s"}`);
        if (meta.stats.waves > 1)
            bits.push(`${meta.stats.waves} waves`);
    }
    // Backend and wall time are display concerns: the card renders them, the
    // transcript does not need them repeated on every call.
    const title = "codemode" + (bits.length > 0 ? ": " + bits.join(" ") : "");
    if (value === undefined) {
        return `${title}\n${paint(theme, "muted", "  (no return statement; add `return` to send a value to the model)")}`;
    }
    if (value && typeof value === "object" && !Array.isArray(value)) {
        // The chosen return value is data. Lossy hit summaries belong only to the
        // TUI or one-shot search, never this explicit model-facing return channel.
        const rows = Object.entries(value)
            .filter(([key]) => !envelope || !TRANSPORT_KEYS.has(key))
            .map(([key, entry]) => paint(theme, "toolOutput", `  ${sanitizeContent(key)}: ${codemodePayload(entry)}`));
        return boundedCodemode([title, ...rows].join("\n"));
    }
    return boundedCodemode(`${title}\n${paint(theme, "toolOutput", `  ${codemodePayload(value)}`)}`);
}
