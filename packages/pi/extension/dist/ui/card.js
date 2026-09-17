/** Pi TUI result card — supernova-style: the call slot is empty; one card
 * owns the whole lifecycle (running → ops ledger → result → error). Rows are
 * fixed-column and theme-painted; nothing here writes to the model channel. */
import { hitLabel, hitLocation, paint, truncateToWidth, visibleWidth, } from "./present.js";
/** renderCall component that paints nothing — the result card owns display. */
export const EMPTY_CALL = {
    render: () => [],
    invalidate() { },
};
const TOOL_COL = 12;
const DUR_COL = 7;
function clamp(text, width) {
    return truncateToWidth(text, Math.max(1, width));
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
function describe(model) {
    const bits = [];
    if (model.ops && model.ops.length > 0)
        bits.push(model.ops.length + (model.ops.length === 1 ? " call" : " calls"));
    else if (model.hits)
        bits.push(model.hits.length + (model.hits.length === 1 ? " hit" : " hits"));
    for (const bit of model.title)
        if (bit)
            bits.push(bit);
    if (model.error)
        bits.push("failed");
    else if (model.running)
        bits.push("running");
    return bits.join(" \u00b7 ");
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
        const lines = bodyLines(theme, model, Math.max(20, width));
        this.cache = { width, lines };
        return lines;
    }
}
function bodyLines(theme, model, width) {
    const lines = [];
    const icon = model.error
        ? paint(theme, "error", "\u2717")
        : model.running
            ? paint(theme, "dim", "\u00b7")
            : paint(theme, "success", "\u2713");
    const head = icon + " " + paint(theme, "accent", "asgrep", true) + " " + paint(theme, "muted", model.command);
    const desc = describe(model);
    lines.push(clamp(desc ? head + " " + paint(theme, "dim", desc) : head, width));
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
        const head = " " + paint(theme, "accent", edit.path ?? "?") + (edit.line ? paint(theme, "dim", ":" + edit.line) : "");
        lines.push(clamp(head, width));
        for (const line of (edit.removed ?? []).slice(0, model.expanded ? 24 : 8)) {
            lines.push("   " + paint(theme, "error", "- " + clamp(line, width - 4)));
        }
        for (const line of (edit.added ?? []).slice(0, model.expanded ? 24 : 8)) {
            lines.push("   " + paint(theme, "success", "+ " + clamp(line, width - 4)));
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
    return lines;
}
function editsOf(value) {
    if (value && typeof value === "object" && Array.isArray(value.edits)) {
        return value.edits.filter((e) => !!e && typeof e === "object" && (Array.isArray(e.removed) || Array.isArray(e.added)));
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
        return value.split("\n").filter((l) => l.length > 0).slice(0, 16);
    if (Array.isArray(value))
        return [value.length + " value" + (value.length === 1 ? "" : "s")];
    if (typeof value === "object") {
        return Object.entries(value).slice(0, 12).map(([k, v]) => {
            const text = typeof v === "string" ? v : JSON.stringify(v);
            return k + ": " + (text.length > 80 ? text.slice(0, 79) + "\u2026" : text);
        });
    }
    return [String(value)];
}
/** Build the card model from the tool result's details payload. */
export function cardModel(result, options) {
    const details = (result.details && typeof result.details === "object" ? result.details : {});
    const command = typeof details.command === "string" ? details.command : "asgrep";
    const expanded = options.expanded === true;
    // In-flight partial updates carry only {command, phase}.
    if (options.isPartial && !("ok" in details)) {
        return { command, title: [typeof details.phase === "string" ? details.phase : undefined], running: true, expanded };
    }
    const title = [];
    if (typeof details.query === "string" && details.query)
        title.push(JSON.stringify(details.query));
    if (typeof details.mode === "string")
        title.push(details.mode);
    const error = details.error;
    if (result.isError || details.ok === false || error) {
        return {
            command,
            title,
            error: typeof error?.message === "string" ? error.message : "tool failed",
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
    // When edits carry diffs they are the interesting part of the result.
    const resultLines = hits || resultEdits ? undefined : resultPreviewLines(details.result);
    const model = { command, title, expanded };
    if (ops && ops.length > 0)
        model.ops = ops;
    if (hits)
        model.hits = hits;
    if (resultEdits)
        model.edits = resultEdits;
    if (resultLines)
        model.resultLines = resultLines;
    return model;
}
/** renderResult entrypoint: bind one card per result slot, feed it details. */
export function renderAsgrepResult(result, options, theme, context) {
    const prev = context?.lastComponent;
    const card = prev instanceof AsgrepCard ? prev : new AsgrepCard();
    card.set(theme, cardModel(result, options));
    if (context)
        context.lastComponent = card;
    return card;
}
