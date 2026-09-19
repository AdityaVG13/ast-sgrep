/** Neat asgrep tool chrome for the Pi TUI and the model-visible content. */
export type PresentTheme = {
    bold(text: string): string;
    fg(role: string, text: string): string;
    /** Optional: pi themes expose tool box backgrounds (toolSuccessBg/toolErrorBg/toolPendingBg). */
    bg?(role: string, text: string): string;
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
export declare const ASGREP_PROMPT_SNIPPET = "Code search by intent, symbol, defs, callers, pattern (asgrep; use without being asked)";
export declare const ASGREP_PROMPT_GUIDELINES: readonly ["Any code lookup (function, def, caller, intent, pattern): call asgrep first.", "Compose in Code Mode (search/defs/read with Promise.all, return a small shaped value); grep only for exact strings or filenames. Bound with in: \"path\"; on 0 hits use suggested_next."];
export declare function paint(theme: PresentTheme | undefined, role: string, text: string, bold?: boolean): string;
export declare function hitLocation(hit: HitLike): string;
export declare function hitLabel(hit: HitLike): string;
export declare function formatEditResult(response: EnvelopeLike, theme?: PresentTheme): string;
/** Model-visible text for a read envelope: the window contents themselves. */
export declare function formatReadResult(response: EnvelopeLike, theme?: PresentTheme): string;
/**
 * Model-facing result text is deliberately lean: the tool call already carries
 * the query/mode, the TUI card renders timing and backend for the human, and
 * every token here is re-sent with the whole transcript. The first line is the
 * only chrome: "<command>: <payload summary>".
 */
export declare function formatSearchResult(response: EnvelopeLike, meta: {
    command: string;
    excerptLines?: number;
}, theme?: PresentTheme): string;
export declare function formatStatusResult(response: EnvelopeLike, theme?: PresentTheme): string;
export declare function formatIndexResult(command: string, response: EnvelopeLike, theme?: PresentTheme): string;
/**
 * Strip terminal control sequences and C0/C1 controls from untrusted content
 * (tool output, code windows, paths) before this extension paints it. Without
 * this a stray ESC in a file would sit inside our own SGR span, leaving an
 * unterminated color and mis-measuring the row's width.
 */
export declare function sanitizeContent(text: string): string;
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
export declare function displayWidth(text: string): number;
/** Local stand-in so we do not take a pi-tui dependency. Over-counts wide glyphs rather than under-count. */
export declare function visibleWidth(text: string): number;
export declare function truncateToWidth(text: string, maxWidth: number, ellipsis?: string): string;
/**
 * One summary line per interesting entry of a shaped value: known shapes get a
 * sentence ("3 windows · path:1-340"), everything else `key: value` with the
 * value compacted. Transport fields are dropped rather than rendered.
 */
export declare function summarizeValue(value: unknown, limit?: number): string[];
export declare function formatCodemodeResult(value: unknown, meta?: {
    stats?: {
        calls: number;
        batchedCalls: number;
        parallelSpawnCalls: number;
        stickyCalls?: number;
        waves: number;
    };
    wallMs?: number;
    backend?: string;
}, theme?: PresentTheme): string;
