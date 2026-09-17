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

/** renderCall component that paints nothing — the result card owns display. */
export const EMPTY_CALL = {
  render: (): string[] => [],
  invalidate(): void {},
};

export type CardModel = {
  command: string;
  title: Array<string | null | undefined>;
  hits?: HitLike[];
  ops?: Array<{ tool: string; target: string; ok: boolean; ms: number }>;
  resultLines?: string[];
  error?: string;
  running?: boolean;
  expanded?: boolean;
};

const TOOL_COL = 12;
const DUR_COL = 7;

function clamp(text: string, width: number): string {
  return truncateToWidth(text, Math.max(1, width));
}

function fitPath(text: string, budget: number): string {
  if (visibleWidth(text) <= budget) return text;
  if (budget <= 1) return "\u2026";
  return "\u2026" + text.slice(Math.max(0, text.length - budget + 1));
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

function describe(model: CardModel): string {
  const bits: string[] = [];
  if (model.ops && model.ops.length > 0) bits.push(model.ops.length + (model.ops.length === 1 ? " call" : " calls"));
  else if (model.hits) bits.push(model.hits.length + (model.hits.length === 1 ? " hit" : " hits"));
  for (const bit of model.title) if (bit) bits.push(bit);
  if (model.error) bits.push("failed");
  else if (model.running) bits.push("running");
  return bits.join(" \u00b7 ");
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
    const lines = bodyLines(theme, model, Math.max(20, width));
    this.cache = { width, lines };
    return lines;
  }
}

function bodyLines(theme: PresentTheme | undefined, model: CardModel, width: number): string[] {
  const lines: string[] = [];
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
  for (const op of ops) lines.push(clamp(opRow(theme, op, width), width));
  if ((model.ops?.length ?? 0) > ops.length) {
    lines.push(paint(theme, "dim", "   \u2026 " + ((model.ops?.length ?? 0) - ops.length) + " more calls"));
  }

  const hits = (model.hits ?? []).slice(0, maxHits);
  hits.forEach((hit, i) => lines.push(hitRow(theme, i + 1, hit, width)));
  if ((model.hits?.length ?? 0) > hits.length) {
    lines.push(paint(theme, "dim", "   \u2026 " + ((model.hits?.length ?? 0) - hits.length) + " more"));
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
  const resultLines = hits ? undefined : resultPreviewLines(details.result);

  const model: CardModel = { command, title, expanded };
  if (ops && ops.length > 0) model.ops = ops;
  if (hits) model.hits = hits;
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
