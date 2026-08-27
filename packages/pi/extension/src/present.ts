/** Neat asgrep tool chrome for the Pi TUI and the model-visible content. */

export type PresentTheme = {
  bold(text: string): string;
  fg(role: string, text: string): string;
};

export type HitLike = {
  file?: unknown;
  path?: unknown;
  ref?: unknown;
  symbol?: unknown;
  kind?: unknown;
  preview?: unknown;
  start_line?: unknown;
  line?: unknown;
  lines?: unknown;
};

export type EnvelopeLike = {
  ok?: unknown;
  hits?: unknown;
  count?: unknown;
  total?: unknown;
  status?: unknown;
  index_status?: unknown;
  counts?: unknown;
  backend?: unknown;
  [key: string]: unknown;
};

export const ASGREP_PROMPT_SNIPPET =
  "Search this repo by intent, symbol, callers, defs, pattern, or chain (in-process asgrep; use without being asked)";

export const ASGREP_PROMPT_GUIDELINES = [
  "For any code lookup (find a function, callers, defs, intent, structural pattern, or imports), call asgrep or asgrep_search immediately. Do not wait for the user to mention ast-sgrep.",
  "Prefer the asgrep Code Mode tool when you need more than one lookup, filtering, or parallel work. Write JavaScript that calls asgrep.search / find / read / edit and return a small shaped value. Independent lookups: Promise.all.",
  "Use grep only for exact log strings, filenames, or config keys. asgrep.edit does unique string replace plus targeted reindex; oldText must match exactly once.",
] as const;

function paint(theme: PresentTheme | undefined, role: string, text: string, bold = false): string {
  const body = bold && theme ? theme.bold(text) : text;
  return theme ? theme.fg(role, body) : body;
}

function hitLocation(hit: HitLike): string {
  const file = String(hit.file ?? hit.path ?? "");
  const line = hit.start_line ?? hit.line ?? hit.lines;
  if (typeof line === "number") return `${file}:${line}`;
  if (typeof line === "string" && line.length > 0) return `${file}:${line}`;
  if (typeof hit.ref === "string" && hit.ref.length > 0) return hit.ref;
  return file || "?";
}

function hitLabel(hit: HitLike): string {
  const symbol = typeof hit.symbol === "string" ? hit.symbol : "";
  const kind = typeof hit.kind === "string" ? hit.kind : "";
  const preview = typeof hit.preview === "string" ? hit.preview.replace(/\s+/g, " ").trim() : "";
  return [symbol, kind, preview && preview.length < 80 ? preview : ""].filter(Boolean).join("  ");
}

function header(theme: PresentTheme | undefined, verb: string, bits: Array<string | null | undefined>): string {
  return [paint(theme, "toolTitle", "asgrep", true), paint(theme, "accent", verb), ...bits.filter((bit): bit is string => Boolean(bit))].join("  ·  ");
}

export function formatSearchCall(
  params: { query?: string; mode?: string; limit?: number; excerptLines?: number },
  theme?: PresentTheme,
): string {
  return header(theme, "search", [
    params.query ? JSON.stringify(params.query) : undefined,
    params.mode ?? "natural",
    params.limit !== undefined ? `limit ${params.limit}` : undefined,
    params.excerptLines ? `excerpt ${params.excerptLines}` : undefined,
  ]);
}

export function formatIndexCall(force: boolean, theme?: PresentTheme): string {
  return header(theme, force ? "reindex" : "index", []);
}

export function formatStatusCall(theme?: PresentTheme): string {
  return header(theme, "status", []);
}

export function formatCodemodeCall(code: string, theme?: PresentTheme): string {
  const preview = code.trim().replace(/\s+/g, " ").slice(0, 80);
  return header(theme, "codemode", [`${preview}${code.trim().length > 80 ? "…" : ""}`]);
}

export function formatSearchResult(
  response: EnvelopeLike,
  meta: { command: string; query?: string; mode?: string; activationMs?: number; backend?: string },
  theme?: PresentTheme,
): string {
  const hits = Array.isArray(response.hits) ? (response.hits as HitLike[]) : [];
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

export function formatStatusResult(response: EnvelopeLike, theme?: PresentTheme): string {
  const state = typeof response.status === "string" ? response.status
    : typeof response.index_status === "string" ? response.index_status
    : response.ok ? "ok" : "failed";
  const counts = response.counts && typeof response.counts === "object"
    ? Object.entries(response.counts as Record<string, unknown>).map(([key, value]) => `${key}=${String(value)}`).join("  ")
    : "";
  const backend = typeof response.backend === "string" ? response.backend : "";
  const title = header(theme, "status", [state, counts, backend]);
  return title;
}

export function formatIndexResult(command: string, response: EnvelopeLike, theme?: PresentTheme): string {
  const count = typeof response.count === "number" ? response.count
    : typeof response.total === "number" ? response.total
    : undefined;
  const tail = count === undefined ? "done" : `${count} file${count === 1 ? "" : "s"}`;
  return header(theme, command, [tail]);
}

function ansiLengthAt(text: string, index: number): number {
  if (text.charCodeAt(index) !== 0x1b) return 0;
  const next = text[index + 1];
  if (next === "[") {
    let cursor = index + 2;
    while (cursor < text.length) {
      const code = text.charCodeAt(cursor);
      if (code >= 0x40 && code <= 0x7e) return cursor - index + 1;
      cursor += 1;
    }
    return text.length - index;
  }
  if (next === "]") {
    let cursor = index + 2;
    while (cursor < text.length) {
      if (text.charCodeAt(cursor) === 0x07) return cursor - index + 1;
      if (text.charCodeAt(cursor) === 0x1b && text[cursor + 1] === "\\") return cursor - index + 2;
      cursor += 1;
    }
    return text.length - index;
  }
  if (next === "P" || next === "X" || next === "^" || next === "_") {
    let cursor = index + 2;
    while (cursor < text.length) {
      if (text.charCodeAt(cursor) === 0x1b && text[cursor + 1] === "\\") return cursor - index + 2;
      cursor += 1;
    }
    return text.length - index;
  }
  return Math.min(2, text.length - index);
}

function cellWidthAt(text: string, index: number): { width: number; length: number } {
  const code = text.charCodeAt(index);
  if (code === 0x09) return { width: 3, length: 1 };
  if (code >= 0xd800 && code <= 0xdbff) return { width: 2, length: 2 };
  if (code <= 0x1f || (code >= 0x7f && code <= 0x9f)) return { width: 0, length: 1 };
  if (code <= 0x7e) return { width: 1, length: 1 };
  return { width: 2, length: 1 };
}

/** Local stand-in so we do not take a pi-tui dependency. Over-counts wide glyphs rather than under-count. */
export function visibleWidth(text: string): number {
  let width = 0;
  for (let index = 0; index < text.length; ) {
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

export function truncateToWidth(text: string, maxWidth: number, ellipsis = "..."): string {
  const limit = Math.max(0, maxWidth);
  if (limit <= 0) return "";
  if (visibleWidth(text) <= limit) return text;
  const ellipsisWidth = visibleWidth(ellipsis);
  if (ellipsisWidth >= limit) return ellipsis.slice(0, limit);
  const budget = limit - ellipsisWidth;
  let kept = "";
  let width = 0;
  for (let index = 0; index < text.length; ) {
    const ansi = ansiLengthAt(text, index);
    if (ansi > 0) {
      kept += text.slice(index, index + ansi);
      index += ansi;
      continue;
    }
    const cell = cellWidthAt(text, index);
    if (width + cell.width > budget) break;
    kept += text.slice(index, index + cell.length);
    width += cell.width;
    index += cell.length;
  }
  return `${kept}${ellipsis}`;
}

function compactValue(value: unknown): string {
  if (value === null || value === undefined) return String(value);
  if (typeof value !== "object") return String(value);
  if (Array.isArray(value)) return `${value.length} item${value.length === 1 ? "" : "s"}`;
  const json = JSON.stringify(value);
  return json.length <= 80 ? json : `${json.slice(0, 79)}…`;
}

export function formatCodemodeResult(
  value: unknown,
  meta: {
    stats?: { calls: number; batchedCalls: number; parallelSpawnCalls: number; stickyCalls?: number; waves: number };
    wallMs?: number;
    backend?: string;
  } = {},
  theme?: PresentTheme,
): string {
  if (value && typeof value === "object" && Array.isArray((value as EnvelopeLike).hits)) {
    const searchMeta: { command: string; activationMs?: number; backend?: string } = { command: "codemode" };
    if (meta.wallMs !== undefined) searchMeta.activationMs = meta.wallMs;
    if (meta.backend !== undefined) searchMeta.backend = meta.backend;
    return formatSearchResult(value as EnvelopeLike, searchMeta, theme);
  }
  const bits: string[] = [];
  if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    if (typeof record.hit_count === "number") bits.push(`${record.hit_count} hit${record.hit_count === 1 ? "" : "s"}`);
    else if (typeof record.node_count === "number") bits.push(`${record.node_count} node${record.node_count === 1 ? "" : "s"}`);
  }
  if (meta.backend === "napi") bits.push("in-process");
  else if (meta.backend === "cli") bits.push("cli-sticky");
  if (meta.stats && meta.stats.calls > 0) {
    const via =
      (meta.stats.stickyCalls ?? 0) > 0
        ? `native ${meta.stats.stickyCalls}`
        : meta.stats.batchedCalls > 0
          ? `batched ${meta.stats.batchedCalls}`
          : meta.stats.parallelSpawnCalls > 0
            ? `parallel-spawn ${meta.stats.parallelSpawnCalls}`
            : `${meta.stats.calls} call${meta.stats.calls === 1 ? "" : "s"}`;
    bits.push(via);
    if (meta.stats.waves > 1) bits.push(`${meta.stats.waves} waves`);
  }
  if (meta.wallMs !== undefined) bits.push(`${meta.wallMs}ms`);
  const title = header(theme, "codemode", bits);
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const rows = Object.entries(value as Record<string, unknown>).slice(0, 16).map(([key, entry]) =>
      paint(theme, "toolOutput", `  ${key}: ${compactValue(entry)}`),
    );
    return [title, ...rows].join("\n");
  }
  if (Array.isArray(value)) {
    return [title, paint(theme, "toolOutput", `  ${value.length} value${value.length === 1 ? "" : "s"}`)].join("\n");
  }
  return `${title}\n${paint(theme, "toolOutput", `  ${compactValue(value)}`)}`;
}

/** Minimal pi-tui Text stand-in so we do not take a TUI package dependency. */
export class AsgrepText {
  #text: string;
  constructor(text = "") {
    this.#text = text;
  }
  setText(text: string): void {
    this.#text = text;
  }
  invalidate(): void {}
  render(width: number): string[] {
    const maxWidth = Math.max(1, width);
    if (this.#text.length === 0) return [""];
    return this.#text.split("\n").map((line) => truncateToWidth(line, maxWidth));
  }
}

export function presentText(formatted: string, last: unknown): AsgrepText {
  if (last instanceof AsgrepText) {
    last.setText(formatted);
    return last;
  }
  if (last && typeof last === "object" && last !== null && "setText" in last && typeof (last as AsgrepText).setText === "function") {
    (last as AsgrepText).setText(formatted);
    return last as AsgrepText;
  }
  return new AsgrepText(formatted);
}
