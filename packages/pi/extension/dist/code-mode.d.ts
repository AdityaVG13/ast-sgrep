import { type AstSgrepRuntime, type MachineEnvelope, type RunOptions, type RuntimeContext } from "./runtime.js";
export type SgrepKind = "asgrep" | "def" | "caller" | "graph" | "anchor" | "import" | "pattern" | "embed";
export type SgrepSignal = "exact" | "structural" | "semantic";
export type SgrepRef = `${string}#L${number}-L${number}`;
/**
 * Trusted search hit. Location is solely `ref` (parsed once at the CLI/JSON boundary).
 * Wire may still dual-encode file/lines; those are not live fields on this type.
 */
export interface SgrepHit {
    kind: SgrepKind;
    signal: SgrepSignal;
    contributors: SgrepKind[];
    score: number;
    margin: number;
    ref: SgrepRef;
    preview: string;
    symbol?: string | null;
    caller?: string | null;
    callee?: string | null;
    language?: string | null;
    excerpt?: string;
}
export interface SgrepSearchResponse extends MachineEnvelope {
    hits: SgrepHit[];
    query?: string;
    hit_count?: number;
}
export interface SgrepSearchOptions extends RunOptions {
    limit?: number;
    excerptLines?: number;
}
export interface SgrepReadOptions {
    contextLines?: number;
    /** Aggregate character budget across all refs. */
    maxChars?: number;
    signal?: AbortSignal;
}
export interface SgrepReadResult {
    ref: SgrepRef;
    file: string;
    lines: {
        start: number;
        end: number;
    };
    content: string;
    truncated: boolean;
    /** Present when truncated: 1-indexed line to resume from (on the last shown line). */
    resumeOffset?: number;
    /** Named recovery hint for the model (empty/past-EOF/truncation). */
    note?: string;
}
export interface SgrepApi {
    keywordSearch(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    astSearch(pattern: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    semanticSearch(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    codeRead(ids: SgrepRef | Pick<SgrepHit, "ref"> | readonly (SgrepRef | Pick<SgrepHit, "ref">)[], options?: SgrepReadOptions): Promise<SgrepReadResult[]>;
    /** Alias for keywordSearch. */
    find(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    /** Alias for astSearch. */
    astFind(pattern: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    /** Alias for semanticSearch. */
    semantic(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    /** Alias for codeRead. */
    read(ids: SgrepRef | Pick<SgrepHit, "ref"> | readonly (SgrepRef | Pick<SgrepHit, "ref">)[], options?: SgrepReadOptions): Promise<SgrepReadResult[]>;
}
export type SgrepPlan<T> = (sgrep: Readonly<SgrepApi>) => T | Promise<T>;
type RuntimeLike = Pick<AstSgrepRuntime, "run" | "resolveRoot">;
/** Derive file/lines from a branded ref (sole location encoding on SgrepHit). */
export declare function parseSgrepRef(ref: SgrepRef): {
    file: string;
    start: number;
    end: number;
};
export declare class SgrepCodeMode implements SgrepApi {
    #private;
    private readonly runtime;
    private readonly context;
    constructor(runtime: RuntimeLike, context: RuntimeContext);
    execute<T>(plan: SgrepPlan<T>): Promise<T>;
    keywordSearch(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    astSearch(pattern: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    semanticSearch(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    find(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    astFind(pattern: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    semantic(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    codeRead(ids: SgrepRef | Pick<SgrepHit, "ref"> | readonly (SgrepRef | Pick<SgrepHit, "ref">)[], options?: SgrepReadOptions): Promise<SgrepReadResult[]>;
    read(ids: SgrepRef | Pick<SgrepHit, "ref"> | readonly (SgrepRef | Pick<SgrepHit, "ref">)[], options?: SgrepReadOptions): Promise<SgrepReadResult[]>;
}
export declare function createSgrepCodeMode(runtime: RuntimeLike, context: RuntimeContext): SgrepCodeMode;
export {};
