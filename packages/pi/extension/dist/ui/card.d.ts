/** Pi TUI result card — supernova-style: the call slot is empty; one card
 * owns the whole lifecycle (running → ops ledger → result → error). Rows are
 * fixed-column and theme-painted; nothing here writes to the model channel. */
import { type HitLike, type PresentTheme } from "./present.js";
/** renderCall component that paints nothing — the result card owns display. */
export declare const EMPTY_CALL: {
    render: () => string[];
    invalidate(): void;
};
export type CardModel = {
    command: string;
    title: Array<string | null | undefined>;
    hits?: HitLike[];
    /** Applied edit diffs: path + line + removed/added line arrays. */
    edits?: Array<{
        path?: string;
        line?: number;
        removed?: string[];
        added?: string[];
        truncated?: boolean;
    }>;
    ops?: Array<{
        tool: string;
        target: string;
        ok: boolean;
        ms: number;
    }>;
    resultLines?: string[];
    /** Warnings that qualify the answer (stale index, unindexed repo). */
    notes?: string[];
    error?: string;
    running?: boolean;
    expanded?: boolean;
};
export declare class AsgrepCard {
    theme: PresentTheme | undefined;
    model: CardModel | undefined;
    cache: {
        width: number;
        lines: string[];
    } | undefined;
    set(theme: PresentTheme | undefined, model: CardModel): void;
    invalidate(): void;
    render(width?: number): string[];
}
type ResultLike = {
    isError?: boolean;
    content?: Array<{
        type: string;
        text?: string;
    }>;
    details?: unknown;
};
type RenderOptions = {
    expanded?: boolean;
    isPartial?: boolean;
};
type RenderContext = {
    lastComponent?: unknown;
    args?: object;
};
/** Build the card model from the tool result's details payload. */
export declare function cardModel(result: ResultLike, options: RenderOptions, callArgs?: Record<string, unknown>): CardModel;
/** renderResult entrypoint: bind one card per result slot, feed it details. */
export declare function renderAsgrepResult(result: ResultLike, options: RenderOptions, theme: PresentTheme, context?: RenderContext): AsgrepCard;
export {};
