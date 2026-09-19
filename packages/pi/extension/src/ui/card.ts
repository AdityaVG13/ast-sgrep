/** Pi TUI result card — supernova-style: the call slot is empty; one card
 * owns the whole lifecycle (running → ops ledger → result → error). Rows are
 * fixed-column and theme-painted; nothing here writes to the model channel. */

import {
  hitLabel,
  hitLocation,
  paint,
  truncateToWidth,
  visibleWidth,
  type EnvelopeLike,
  type HitLike,
  type PresentTheme,
} from "./present.js";

/** Header text that rides the top border: "asgrep search · 4 hits · 12ms · napi". */
function frameLabel(model: CardModel): string {
  const bits = ["asgrep", model.command, ...model.title.filter((b): b is string => Boolean(b))];
  const tail = model.error ? "failed" : model.running ? "running" : "";
  if (tail) bits.push(tail);
  const counts: string[] = [];
  if (model.ops?.length) counts.push(model.ops.length + (model.ops.length === 1 ? " call" : " calls"));
  if (model.hits) counts.push(model.hits.length + (model.hits.length === 1 ? " hit" : " hits"));
  return [...bits.slice(0, 2), ...counts, ...bits.slice(2)].join(" \u00b7 ");
}

/** renderCall component that paints nothing — the result card owns display. */
export const EMPTY_CALL = {
  render: (): string[] => [],
  invalidate(): void {},
};

export type CardModel = {
  command: string;
  title: Array<string | null | undefined>;
  hits?: HitLike[];
  /** Applied edit diffs: path + line + removed/added line arrays. */
  edits?: Array<{ path?: string; line?: number; removed?: string[]; added?: string[]; truncated?: boolean }>;
  ops?: Array<{ tool: string; target: string; ok: boolean; ms: number }>;
  resultLines?: string[];
  error?: string;
  running?: boolean;
  expanded?: boolean;
};

const TOOL_COL = 12;
const DUR_COL = 7;
const MAX_CARD_WIDTH = 100;

/** Rounded frame glyphs — pi themes may provide theme.boxRound; default ASCII-art set. */
const BOX = { tl: "\u256d", tr: "\u256e", bl: "\u2570", br: "\u256f", h: "\u2500", v: "\u2502" };

type BoxGlyphs = { tl: string; tr: string; bl: string; br: string; h: string; v: string };

function boxOf(theme: PresentTheme | undefined): BoxGlyphs {
  const b = (theme as unknown as { boxRound?: Record<string, string> | undefined })?.boxRound;
  if (b && typeof b.topLeft === "string" && typeof b.horizontal === "string" && typeof b.vertical === "string") {
    return { tl: b.topLeft, tr: b.topRight ?? BOX.tr, bl: b.bottomLeft ?? BOX.bl, br: b.bottomRight ?? BOX.br, h: b.horizontal, v: b.vertical };
  }
  return BOX;
}

function borderKey(model: CardModel): string {
  if (model.error) return "error";
  if (model.running) return "accent";
  return "dim";
}

/** Top/bottom bar with an optional label embedded in the rule. Geometry is in
 * display cells (frameW): \u256d + 3 rules on the left, corner on the right. */
function frameBar(theme: PresentTheme | undefined, box: BoxGlyphs, border: (t: string) => string, left: string, right: string, label: string | null, width: number): string {
  const leftRaw = left + box.h.repeat(3);
  const shown = label ? clamp(" " + label + " ", Math.max(0, width - frameW(leftRaw) - 1)) : "";
  const fill = Math.max(0, width - frameW(leftRaw) - frameW(shown) - 1);
  return border(leftRaw) + shown + border(box.h.repeat(fill)) + border(right);
}

/** ANSI-aware cut to `width` display columns on the same scale as frameW
 * (stripped code-point count). Unlike truncateToWidth this never leaves an
 * unclosed SGR color behind: a cut inside a painted span appends \x1b[0m
 * before the ellipsis so the row cannot bleed color into the right border. */
function clamp(text: string, width: number): string {
  const limit = Math.max(1, width);
  if (frameW(text) <= limit) return text;
  const budget = Math.max(1, limit - 1); // room for the ellipsis
  const ansi = /\u001b\[[0-9;]*m/gu;
  const stops: Array<[number, number]> = [];
  for (let match = ansi.exec(text); match !== null; match = ansi.exec(text)) {
    stops.push([match.index, match.index + match[0].length]);
  }
  let kept = "";
  let visible = 0;
  let index = 0;
  let stopIndex = 0;
  while (index < text.length && visible < budget) {
    if (stopIndex < stops.length && index === stops[stopIndex]![0]) {
      const [, end] = stops[stopIndex]!;
      kept += text.slice(index, end);
      index = end;
      stopIndex += 1;
      continue;
    }
    kept += text[index];
    visible += 1;
    index += 1;
  }
  return kept + (stops.length > 0 ? "\u001b[0m" : "") + "\u2026";
}

function fitPath(text: string, budget: number): string {
  if (visibleWidth(text) <= budget) return text;
  if (budget <= 1) return "\u2026";
  return "\u2026" + text.slice(Math.max(0, text.length - budget + 1));
}

/** Display columns for frame geometry: ANSI-stripped code-point count.
 * Box glyphs/·/✓ render width-1 in real terminals; the conservative
 * visibleWidth() over-counts them (width 2) which would ragged the box. */
function frameW(text: string): number {
  return text.replace(/\u001b\[[0-9;]*m/g, "").length;
}

function fmtMs(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "";
  return ms < 1000 ? Math.round(ms) + "ms" : (ms / 1000).toFixed(1) + "s";
}

/** "✓ search      12ms  \"query\"" — one aligned op row. */
function opRow(theme: PresentTheme | undefined, op: { tool: string; target: string; ok: boolean; ms: number }, width: number): string {
  const marker = op.ok ? paint(theme, "success", "\u2713") : paint(theme, "error", "\u00d7");
  const tool = paint(theme, "syntaxFunction", op.tool.slice(0, TOOL_COL).padEnd(TOOL_COL));
  const dur = paint(theme, "dim", fmtMs(op.ms).padStart(DUR_COL));
  const target = op.target ? "  " + paint(theme, "muted", fitPath(op.target, Math.max(1, width - TOOL_COL - DUR_COL - 9))) : "";
  return " " + marker + " " + tool + " " + dur + target;
}

/** " 1. path:line   symbol · kind" — one numbered hit row. */
function hitRow(theme: PresentTheme | undefined, n: number, hit: HitLike, width: number): string {
  const num = paint(theme, "dim", String(n).padStart(2) + ".");
  const loc = hitLocation(hit);
  const locBudget = Math.max(8, Math.floor(width * 0.62));
  const locText = paint(theme, "accent", fitPath(loc, locBudget));
  const label = hitLabel(hit);
  const row = " " + num + " " + locText + (label ? "  " + paint(theme, "muted", label) : "");
  return clamp(row, width);
}

export class AsgrepCard {
  theme: PresentTheme | undefined;
  model: CardModel | undefined;
  cache: { width: number; lines: string[] } | undefined;

  set(theme: PresentTheme | undefined, model: CardModel): void {
    this.theme = theme;
    this.model = model;
    this.cache = undefined;
  }

  invalidate(): void {
    this.cache = undefined;
  }

  render(width = 80): string[] {
    const theme = this.theme;
    const model = this.model;
    if (!model || width <= 0) return [];
    if (this.cache?.width === width) return this.cache.lines;
    // Pi hands us the full terminal width — a hollow frame at 200+ cols is a
    // wall of empty border. Cap at a readable card width; pi pads the rest.
    const lines = framedLines(theme, model, Math.min(Math.max(8, width), MAX_CARD_WIDTH));
    this.cache = { width, lines };
    return lines;
  }
}

/** Rounded card: header embedded in the top rule, body rows in \u2502 gutters. */
function framedLines(theme: PresentTheme | undefined, model: CardModel, width: number): string[] {
  const box = boxOf(theme);
  const key = borderKey(model);
  const border = (text: string): string => paint(theme, key, text);
  const inner = Math.max(1, width - 4); // "\u2502 " + content + " \u2502"
  const label = frameLabel(model);
  const rows = bodyLines(theme, model, inner);
  const out = [frameBar(theme, box, border, box.tl, box.tr, label, width)];
  for (const row of rows) {
    const body = clamp(row, inner);
    const pad = Math.max(0, inner - frameW(body));
    out.push(border(box.v) + " " + body + " ".repeat(pad) + " " + border(box.v));
  }
  out.push(frameBar(theme, box, border, box.bl, box.br, null, width));
  return out;
}

function bodyLines(theme: PresentTheme | undefined, model: CardModel, width: number): string[] {
  const lines: string[] = [];

  const maxOps = model.expanded ? 24 : 8;
  const maxHits = model.expanded ? 24 : 12;
  const ops = (model.ops ?? []).slice(0, maxOps);
  for (const op of ops) lines.push(clamp(opRow(theme, op, width), width));
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
    if (edit.truncated) lines.push(paint(theme, "dim", "   \u2026"));
  }

  if (model.error) {
    for (const line of model.error.split("\n").slice(0, model.expanded ? 12 : 4)) {
      lines.push(" " + paint(theme, "error", clamp(line, width - 1)));
    }
  } else if (model.resultLines && model.resultLines.length > 0) {
    const shown = model.expanded ? model.resultLines : model.resultLines.slice(0, 8);
    for (const line of shown) lines.push(" " + paint(theme, "muted", clamp(line, width - 1)));
    if (!model.expanded && model.resultLines.length > shown.length) {
      lines.push(paint(theme, "dim", "   \u2026 " + (model.resultLines.length - shown.length) + " more result lines"));
    }
  }
  return lines;
}

type ResultLike = {
  isError?: boolean;
  content?: Array<{ type: string; text?: string }>;
  details?: unknown;
};

type RenderOptions = { expanded?: boolean; isPartial?: boolean };
type RenderContext = { lastComponent?: unknown };

function editsOf(value: unknown): Array<{ path?: string; line?: number; removed?: string[]; added?: string[]; truncated?: boolean }> | undefined {
  if (value && typeof value === "object" && Array.isArray((value as { edits?: unknown }).edits)) {
    return (value as { edits: unknown[] }).edits.filter(
      (e): e is { path?: string; line?: number; removed?: string[]; added?: string[]; truncated?: boolean } =>
        !!e && typeof e === "object" && (Array.isArray((e as { removed?: unknown }).removed) || Array.isArray((e as { added?: unknown }).added)),
    );
  }
  return undefined;
}

function hitsOf(value: unknown): HitLike[] | undefined {
  if (value && typeof value === "object" && Array.isArray((value as EnvelopeLike).hits)) {
    return (value as EnvelopeLike).hits as HitLike[];
  }
  return undefined;
}

function resultPreviewLines(value: unknown): string[] | undefined {
  if (value === undefined || value === null) return undefined;
  if (typeof value === "string") return value.split("\n").filter((l) => l.length > 0).slice(0, 16);
  if (Array.isArray(value)) return [value.length + " value" + (value.length === 1 ? "" : "s")];
  if (typeof value === "object") {
    return Object.entries(value as Record<string, unknown>).slice(0, 12).map(([k, v]) => {
      const text = typeof v === "string" ? v : JSON.stringify(v);
      return k + ": " + (text.length > 80 ? text.slice(0, 79) + "\u2026" : text);
    });
  }
  return [String(value)];
}

/** Build the card model from the tool result's details payload. */
export function cardModel(result: ResultLike, options: RenderOptions): CardModel {
  const details = (result.details && typeof result.details === "object" ? result.details : {}) as Record<string, unknown>;
  const command = typeof details.command === "string" ? details.command : "asgrep";
  const expanded = options.expanded === true;

  // In-flight partial updates carry only {command, phase}.
  if (options.isPartial && !("ok" in details)) {
    return { command, title: [typeof details.phase === "string" ? details.phase : undefined], running: true, expanded };
  }

  const title: Array<string | undefined> = [];
  if (typeof details.query === "string" && details.query) title.push(JSON.stringify(details.query));
  if (typeof details.mode === "string") title.push(details.mode);

  const error = details.error as { message?: unknown } | undefined;
  if (result.isError || details.ok === false || error) {
    return {
      command,
      title,
      error: typeof error?.message === "string" ? error.message : "tool failed",
      expanded,
    };
  }
  const response = details.response as EnvelopeLike | undefined;
  if (command === "status" || command === "index" || command === "reindex") {
    const state = typeof response?.status === "string" ? response.status
      : typeof response?.index_status === "string" ? response.index_status
      : response?.ok === true ? "ok" : undefined;
    if (state) title.push(state);
    const counts = response?.counts;
    if (counts && typeof counts === "object") {
      title.push(Object.entries(counts as Record<string, unknown>).slice(0, 4).map(([k, v]) => k + "=" + String(v)).join(" "));
    } else if (response) {
      // Flat status envelope: file_count/symbol_count/caller_count + embed info.
      const flat: string[] = [];
      for (const k of ["file_count", "symbol_count", "caller_count"]) {
        const v = (response as Record<string, unknown>)[k];
        if (typeof v === "number" || typeof v === "bigint") flat.push(k.replace(/_count$/, "s") + "=" + String(v));
      }
      if (typeof response.embed_backend === "string") flat.push(response.embed_backend);
      if (flat.length > 0) title.push(flat.join(" "));
    }
  }
  const ms = typeof details.wallMs === "number" ? details.wallMs : typeof details.activationMs === "number" ? details.activationMs : undefined;
  if (ms !== undefined) title.push(fmtMs(ms));
  if (typeof details.backend === "string") title.push(details.backend);

  const trace = Array.isArray(details.trace) ? details.trace as Array<{ tool: string; target?: string; ok: boolean; ms: number }> : undefined;
  const ops = trace?.map((t) => ({ tool: t.tool, target: t.target ?? "", ok: t.ok !== false, ms: typeof t.ms === "number" ? t.ms : 0 }));

  const hits = hitsOf(response) ?? hitsOf(details.result);
  const resultEdits = editsOf(details.result) ?? editsOf(response);
  // read envelopes carry windows: preview each window's first lines.
  const windows = command === "read" && response && Array.isArray((response as { windows?: unknown }).windows)
    ? (response as { windows: Array<{ path?: string; start?: number; end?: number; text?: string }> }).windows
    : undefined;
  const readLines = windows?.flatMap((w) => [
    (w.path ?? "?") + ":" + (w.start ?? 1) + "-" + (w.end ?? ""),
    ...(typeof w.text === "string" ? w.text.split("\n").slice(0, expanded ? 20 : 6).map((l) => "  " + l) : []),
  ]);
  // When edits carry diffs they are the interesting part of the result.
  const resultLines = hits || resultEdits ? undefined : (readLines ?? resultPreviewLines(details.result));

  const model: CardModel = { command, title, expanded };
  if (ops && ops.length > 0) model.ops = ops;
  if (hits) model.hits = hits;
  if (resultEdits) model.edits = resultEdits;
  if (resultLines) model.resultLines = resultLines;
  return model;
}

/** renderResult entrypoint: bind one card per result slot, feed it details. */
export function renderAsgrepResult(result: ResultLike, options: RenderOptions, theme: PresentTheme, context?: RenderContext): AsgrepCard {
  const prev = context?.lastComponent;
  const card = prev instanceof AsgrepCard ? prev : new AsgrepCard();
  card.set(theme, cardModel(result, options));
  if (context) context.lastComponent = card;
  return card;
}
