/** Pi TUI result card — supernova-style: the call slot is empty; one card
 * owns the whole lifecycle (running → ops ledger → result → error). Rows are
 * fixed-column and theme-painted; nothing here writes to the model channel. */
import { displayWidth, hitLabel, hitLocation, paint, sanitizeContent, summarizeValue, truncateToWidth, visibleWidth, } from "./present.js";
/** Header text that rides the top border: "asgrep search · 4 hits · 12ms · napi". */
function frameLabel(model) {
    const bits = ["asgrep", model.command, ...model.title.filter((b) => Boolean(b))];
    const tail = model.error ? "failed" : model.running ? "running" : "";
    if (tail)
        bits.push(tail);
    const counts = [];
    if (model.ops?.length)
        counts.push(model.ops.length + (model.ops.length === 1 ? " call" : " calls"));
    if (model.hits)
        counts.push(model.hits.length + (model.hits.length === 1 ? " hit" : " hits"));
    return [...bits.slice(0, 2), ...counts, ...bits.slice(2)].join(" \u00b7 ");
}
/** renderCall component that paints nothing — the result card owns display. */
export const EMPTY_CALL = {
    render: () => [],
    invalidate() { },
};
const TOOL_COL = 12;
const DUR_COL = 7;
/** Rounded frame glyphs — pi themes may provide theme.boxRound; default ASCII-art set. */
const BOX = { tl: "\u256d", tr: "\u256e", bl: "\u2570", br: "\u256f", h: "\u2500", v: "\u2502" };
function boxOf(theme) {
    const b = theme?.boxRound;
    if (b && typeof b.topLeft === "string" && typeof b.horizontal === "string" && typeof b.vertical === "string") {
        return { tl: b.topLeft, tr: b.topRight ?? BOX.tr, bl: b.bottomLeft ?? BOX.bl, br: b.bottomRight ?? BOX.br, h: b.horizontal, v: b.vertical };
    }
    return BOX;
}
function borderKey(model) {
    if (model.error)
        return "error";
    if (model.running)
        return "accent";
    return "dim";
}
/** Tool box background per state — the same keys pi themes tint native tool rows with. */
function backgroundKey(model) {
    if (model.error)
        return "toolErrorBg";
    if (model.running)
        return "toolPendingBg";
    return "toolSuccessBg";
}
/**
 * Paint the tool box background across the full row, when the theme has one.
 * Without this the host's message background shows through the card, which
 * reads as a dark band beside the border.
 */
function backgroundPaint(theme, model) {
    const bg = theme?.bg;
    if (typeof bg !== "function")
        return undefined;
    try {
        const key = backgroundKey(model);
        if (typeof bg.call(theme, key, "x") !== "string")
            return undefined;
        return (text) => {
            const painted = bg.call(theme, key, text);
            return typeof painted === "string" ? painted : text;
        };
    }
    catch {
        return undefined;
    }
}
/** Top/bottom bar with an optional label embedded in the rule. Geometry is in
 * display cells (displayWidth): \u256d + 3 rules on the left, corner on the right. */
function frameBar(theme, box, border, left, right, label, width) {
    const leftRaw = left + box.h.repeat(3);
    const shown = label ? clamp(" " + label + " ", Math.max(0, width - displayWidth(leftRaw) - 1)) : "";
    const fill = Math.max(0, width - displayWidth(leftRaw) - displayWidth(shown) - 1);
    return border(leftRaw) + shown + border(box.h.repeat(fill)) + border(right);
}
/**
 * Cut a row to `width` display columns (pi's cell scale: tab = 3, wide = 2).
 *
 * Delegates to the shared cell-aware truncator so every surface measures with
 * one rule. Counting code units here instead (the previous shape) under-counted
 * tabs threefold and made tab-indented code rows overflow the card — the crash
 * pi reports as "Rendered line N exceeds terminal width".
 */
function clamp(text, width) {
    return truncateToWidth(text, Math.max(1, width), "\u2026");
}
function fitPath(text, budget) {
    if (visibleWidth(text) <= budget)
        return text;
    if (budget <= 1)
        return "\u2026";
    return "\u2026" + text.slice(Math.max(0, text.length - budget + 1));
}
function fmtMs(ms) {
    if (!Number.isFinite(ms) || ms < 0)
        return "";
    return ms < 1000 ? Math.round(ms) + "ms" : (ms / 1000).toFixed(1) + "s";
}
/** "✓ search      12ms  \"query\"" — one aligned op row. */
function opRow(theme, op, width) {
    const marker = op.ok ? paint(theme, "success", "\u2713") : paint(theme, "error", "\u00d7");
    const tool = paint(theme, "syntaxFunction", op.tool.slice(0, TOOL_COL).padEnd(TOOL_COL));
    const dur = paint(theme, "dim", fmtMs(op.ms).padStart(DUR_COL));
    const target = op.target ? "  " + paint(theme, "muted", fitPath(op.target, Math.max(1, width - TOOL_COL - DUR_COL - 9))) : "";
    return " " + marker + " " + tool + " " + dur + target;
}
/** " 1. path:line   symbol · kind" — one numbered hit row. */
function hitRow(theme, n, hit, width) {
    const num = paint(theme, "dim", String(n).padStart(2) + ".");
    const loc = hitLocation(hit);
    const locBudget = Math.max(8, Math.floor(width * 0.62));
    const locText = paint(theme, "accent", fitPath(loc, locBudget));
    const label = hitLabel(hit);
    const row = " " + num + " " + locText + (label ? "  " + paint(theme, "muted", label) : "");
    return clamp(row, width);
}
export class AsgrepCard {
    theme;
    model;
    cache;
    set(theme, model) {
        this.theme = theme;
        this.model = model;
        this.cache = undefined;
    }
    invalidate() {
        this.cache = undefined;
    }
    render(width = 80) {
        const theme = this.theme;
        const model = this.model;
        if (!model || width <= 0)
            return [];
        if (this.cache?.width === width)
            return this.cache.lines;
        // Pi hands us the full terminal width and renders this card with
        // renderShell "self", so the frame must span every column the host gave
        // us. Capping it left the host's message background showing as a dark
        // band to the right of the border.
        const lines = framedLines(theme, model, Math.max(8, width));
        this.cache = { width, lines };
        return lines;
    }
}
/** Rounded card: header embedded in the top rule, body rows in \u2502 gutters. */
function framedLines(theme, model, width) {
    const box = boxOf(theme);
    const key = borderKey(model);
    const border = (text) => paint(theme, key, text);
    const inner = Math.max(1, width - 4); // "\u2502 " + content + " \u2502"
    const label = frameLabel(model);
    const rows = bodyLines(theme, model, inner);
    const background = backgroundPaint(theme, model);
    const fill = (line) => (background ? background(line) : line);
    const out = [fill(frameBar(theme, box, border, box.tl, box.tr, label, width))];
    for (const row of rows) {
        const body = clamp(row, inner);
        const pad = Math.max(0, inner - displayWidth(body));
        out.push(fill(border(box.v) + " " + body + " ".repeat(pad) + " " + border(box.v)));
    }
    out.push(fill(frameBar(theme, box, border, box.bl, box.br, null, width)));
    return out;
}
function bodyLines(theme, model, width) {
    const lines = [];
    const maxOps = model.expanded ? 24 : 8;
    const maxHits = model.expanded ? 24 : 12;
    const ops = (model.ops ?? []).slice(0, maxOps);
    for (const op of ops)
        lines.push(clamp(opRow(theme, op, width), width));
    if ((model.ops?.length ?? 0) > ops.length) {
        lines.push(paint(theme, "dim", "   \u2026 " + ((model.ops?.length ?? 0) - ops.length) + " more calls"));
    }
    const hits = (model.hits ?? []).slice(0, maxHits);
    hits.forEach((hit, i) => lines.push(hitRow(theme, i + 1, hit, width)));
    if ((model.hits?.length ?? 0) > hits.length) {
        lines.push(paint(theme, "dim", "   \u2026 " + ((model.hits?.length ?? 0) - hits.length) + " more"));
    }
    for (const edit of (model.edits ?? []).slice(0, model.expanded ? 12 : 4)) {
        const head = " " + paint(theme, "accent", sanitizeContent(edit.path ?? "?")) + (edit.line ? paint(theme, "dim", ":" + edit.line) : "");
        lines.push(clamp(head, width));
        for (const line of (edit.removed ?? []).slice(0, model.expanded ? 24 : 8)) {
            lines.push("   " + paint(theme, "error", "- " + clamp(line, width - 5)));
        }
        for (const line of (edit.added ?? []).slice(0, model.expanded ? 24 : 8)) {
            lines.push("   " + paint(theme, "success", "+ " + clamp(line, width - 5)));
        }
        if (edit.truncated)
            lines.push(paint(theme, "dim", "   \u2026"));
    }
    if (model.error) {
        for (const line of model.error.split("\n").slice(0, model.expanded ? 12 : 4)) {
            lines.push(" " + paint(theme, "error", clamp(line, width - 1)));
        }
    }
    else if (model.resultLines && model.resultLines.length > 0) {
        const shown = model.expanded ? model.resultLines : model.resultLines.slice(0, 8);
        for (const line of shown)
            lines.push(" " + paint(theme, "muted", clamp(line, width - 1)));
        if (!model.expanded && model.resultLines.length > shown.length) {
            lines.push(paint(theme, "dim", "   \u2026 " + (model.resultLines.length - shown.length) + " more result lines"));
        }
    }
    for (const note of model.notes ?? []) {
        lines.push(" " + paint(theme, "warning", "! " + clamp(note, width - 3)));
    }
    return lines;
}
/** Target bits taken from the call arguments, so a running card can name what
 * the call is about before any result exists. Never duplicated on completion:
 * once details land they own the label. */
function callTargetBits(args) {
    if (!args)
        return [];
    const bits = [];
    if (typeof args.query === "string" && args.query)
        bits.push(JSON.stringify(sanitizeContent(args.query)));
    else if (typeof args.symbol === "string" && args.symbol)
        bits.push(sanitizeContent(args.symbol));
    else if (typeof args.code === "string" && args.code)
        bits.push(sanitizeContent(args.code.trim().replace(/\s+/gu, " ")).slice(0, 60));
    if (typeof args.path === "string" && args.path)
        bits.push(sanitizeContent(args.path));
    else if (typeof args.ref === "string" && args.ref)
        bits.push(sanitizeContent(args.ref));
    return bits;
}
function editsOf(value) {
    if (value && typeof value === "object" && Array.isArray(value.edits)) {
        return value.edits
            .filter((e) => !!e && typeof e === "object")
            .filter((e) => Array.isArray(e.removed) || Array.isArray(e.added))
            .map((e) => {
            const entry = {
                truncated: e.truncated === true,
            };
            if (typeof e.path === "string")
                entry.path = sanitizeContent(e.path);
            if (typeof e.line === "number")
                entry.line = e.line;
            if (Array.isArray(e.removed))
                entry.removed = e.removed.map((line) => sanitizeContent(String(line)));
            if (Array.isArray(e.added))
                entry.added = e.added.map((line) => sanitizeContent(String(line)));
            return entry;
        });
    }
    return undefined;
}
function hitsOf(value) {
    if (value && typeof value === "object" && Array.isArray(value.hits)) {
        return value.hits;
    }
    return undefined;
}
function resultPreviewLines(value) {
    if (value === undefined || value === null)
        return undefined;
    if (typeof value === "string")
        return value.split("\n").filter((line) => line.length > 0).slice(0, 16).map(sanitizeContent);
    if (Array.isArray(value))
        return [value.length + " value" + (value.length === 1 ? "" : "s")];
    if (typeof value === "object") {
        // Shaped summary only: transport fields (tool/command/schema_version/ok)
        // describe the wire envelope, not the answer, and a raw JSON dump of them
        // is noise in the transcript.
        const rows = summarizeValue(value);
        return rows.length > 0 ? rows : undefined;
    }
    return [sanitizeContent(String(value))];
}
/** Build the card model from the tool result's details payload. */
export function cardModel(result, options, callArgs) {
    const details = (result.details && typeof result.details === "object" ? result.details : {});
    const command = typeof details.command === "string" ? details.command : "asgrep";
    const expanded = options.expanded === true;
    // In-flight partial updates carry only {command, phase}.
    if (options.isPartial && !("ok" in details)) {
        const phase = typeof details.phase === "string" && details.phase !== "started" ? details.phase : undefined;
        return {
            command,
            title: [...callTargetBits(callArgs), phase],
            running: true,
            expanded,
        };
    }
    const title = [];
    if (typeof details.query === "string" && details.query)
        title.push(JSON.stringify(sanitizeContent(details.query)));
    if (typeof details.mode === "string")
        title.push(sanitizeContent(details.mode));
    const error = details.error;
    if (result.isError || details.ok === false || error) {
        return {
            command,
            title,
            error: typeof error?.message === "string" ? sanitizeContent(error.message) : "tool failed",
            expanded,
        };
    }
    const response = details.response;
    if (command === "status" || command === "index" || command === "reindex") {
        const state = typeof response?.status === "string" ? response.status
            : typeof response?.index_status === "string" ? response.index_status
                : response?.ok === true ? "ok" : undefined;
        if (state)
            title.push(state);
        const counts = response?.counts;
        if (counts && typeof counts === "object") {
            title.push(Object.entries(counts).slice(0, 4).map(([k, v]) => k + "=" + String(v)).join(" "));
        }
        else if (response) {
            // Flat status envelope: file_count/symbol_count/caller_count + embed info.
            const flat = [];
            for (const k of ["file_count", "symbol_count", "caller_count"]) {
                const v = response[k];
                if (typeof v === "number" || typeof v === "bigint")
                    flat.push(k.replace(/_count$/, "s") + "=" + String(v));
            }
            if (typeof response.embed_backend === "string")
                flat.push(response.embed_backend);
            if (flat.length > 0)
                title.push(flat.join(" "));
        }
    }
    const ms = typeof details.wallMs === "number" ? details.wallMs : typeof details.activationMs === "number" ? details.activationMs : undefined;
    if (ms !== undefined)
        title.push(fmtMs(ms));
    if (typeof details.backend === "string")
        title.push(details.backend);
    const trace = Array.isArray(details.trace) ? details.trace : undefined;
    const ops = trace?.map((t) => ({ tool: t.tool, target: t.target ?? "", ok: t.ok !== false, ms: typeof t.ms === "number" ? t.ms : 0 }));
    const hits = hitsOf(response) ?? hitsOf(details.result);
    const resultEdits = editsOf(details.result) ?? editsOf(response);
    // read envelopes carry windows: preview each window's first lines.
    const windows = command === "read" && response && Array.isArray(response.windows)
        ? response.windows
        : undefined;
    const readLines = windows?.flatMap((w) => [
        sanitizeContent((w.path ?? "?") + ":" + (w.start ?? 1) + "-" + (w.end ?? "")),
        ...(typeof w.text === "string" ? sanitizeContent(w.text).split("\n").slice(0, expanded ? 20 : 6).map((l) => "  " + l) : []),
    ]);
    // When edits carry diffs they are the interesting part of the result.
    const resultLines = hits || resultEdits ? undefined : (readLines ?? resultPreviewLines(details.result));
    const model = { command, title, expanded };
    if (ops && ops.length > 0)
        model.ops = ops;
    if (hits)
        model.hits = hits;
    if (resultEdits)
        model.edits = resultEdits;
    if (resultLines)
        model.resultLines = resultLines;
    // Warnings travel with the model-visible text; keep them visible in the TUI
    // too, or the transcript looks clean while the answer is qualified.
    if (Array.isArray(details.notes)) {
        const notes = details.notes.filter((note) => typeof note === "string" && note.length > 0);
        if (notes.length > 0)
            model.notes = notes.map((note) => sanitizeContent(note));
    }
    return model;
}
/** renderResult entrypoint: bind one card per result slot, feed it details. */
export function renderAsgrepResult(result, options, theme, context) {
    const prev = context?.lastComponent;
    const card = prev instanceof AsgrepCard ? prev : new AsgrepCard();
    const args = context?.args;
    card.set(theme, cardModel(result, options, args && !Array.isArray(args) ? args : undefined));
    if (context)
        context.lastComponent = card;
    return card;
}
