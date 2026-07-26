import { type AstSgrepRuntime, type MachineEnvelope, type RunOptions, type RuntimeContext } from "./runtime.js";
export type SgrepKind = "asgrep" | "def" | "caller" | "graph" | "anchor" | "import" | "pattern" | "embed";
export type SgrepSignal = "exact" | "structural" | "semantic";
export type SgrepRef = `${string}#L${number}-L${number}`;
export interface SgrepHit {
    kind: SgrepKind;
    signal: SgrepSignal;
    contributors: SgrepKind[];
    score: number;
    margin: number;
    file: string;
    lines: {
        start: number;
        end: number;
    };
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
}
export interface SgrepApi {
    find(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    astFind(pattern: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    semantic(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    read(ids: SgrepRef | Pick<SgrepHit, "ref"> | readonly (SgrepRef | Pick<SgrepHit, "ref">)[], options?: SgrepReadOptions): Promise<SgrepReadResult[]>;
}
export type SgrepPlan<T> = (sgrep: Readonly<SgrepApi>) => T | Promise<T>;
type RuntimeLike = Pick<AstSgrepRuntime, "run" | "resolveRoot">;
export declare class SgrepCodeMode implements SgrepApi {
    #private;
    private readonly runtime;
    private readonly context;
    constructor(runtime: RuntimeLike, context: RuntimeContext);
    execute<T>(plan: SgrepPlan<T>): Promise<T>;
    find(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    astFind(pattern: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    semantic(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
    read(ids: SgrepRef | Pick<SgrepHit, "ref"> | readonly (SgrepRef | Pick<SgrepHit, "ref">)[], options?: SgrepReadOptions): Promise<SgrepReadResult[]>;
}
export declare function createSgrepCodeMode(runtime: RuntimeLike, context: RuntimeContext): SgrepCodeMode;
export {};
