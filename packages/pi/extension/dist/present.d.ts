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
export declare const ASGREP_PROMPT_SNIPPET = "Search this repo by intent, symbol, callers, defs, pattern, or chain (in-process asgrep; use without being asked)";
export declare const ASGREP_PROMPT_GUIDELINES: readonly ["For any code lookup (find a function, callers, defs, intent, structural pattern, or imports), call asgrep or asgrep_search immediately. Do not wait for the user to mention ast-sgrep.", "Prefer the asgrep Code Mode tool when you need more than one lookup, filtering, or parallel work. Write JavaScript that calls asgrep.search / semantic / chain / defs / callers / imports / indexStatus / indexRepo and return a small shaped value.", "Use grep only for exact log strings, filenames, or config keys. Use Pi write/edit to change files; asgrep does not mutate source."];
export declare function formatSearchCall(params: {
    query?: string;
    mode?: string;
    limit?: number;
    excerptLines?: number;
}, theme?: PresentTheme): string;
export declare function formatIndexCall(force: boolean, theme?: PresentTheme): string;
export declare function formatStatusCall(theme?: PresentTheme): string;
export declare function formatCodemodeCall(code: string, theme?: PresentTheme): string;
export declare function formatSearchResult(response: EnvelopeLike, meta: {
    command: string;
    query?: string;
    mode?: string;
    activationMs?: number;
    backend?: string;
}, theme?: PresentTheme): string;
export declare function formatStatusResult(response: EnvelopeLike, theme?: PresentTheme): string;
export declare function formatIndexResult(command: string, response: EnvelopeLike, theme?: PresentTheme): string;
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
/** Minimal pi-tui Text stand-in so we do not take a TUI package dependency. */
export declare class AsgrepText {
    #private;
    constructor(text?: string);
    setText(text: string): void;
    invalidate(): void;
    render(_width: number): string[];
}
export declare function presentText(formatted: string, last: unknown): AsgrepText;
