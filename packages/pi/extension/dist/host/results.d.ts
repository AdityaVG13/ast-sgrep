import { type MachineEnvelope, type RunOptions } from "../runtime/types.js";
import type { FreshnessCoordinator } from "../runtime/freshness.js";
export declare const MAX_CONTENT_CHARS = 8000;
export type RuntimeLike = {
    run(args: readonly string[], context: {
        cwd: string;
    }, options?: RunOptions): Promise<MachineEnvelope>;
    resolveRoot?(context: {
        cwd: string;
    }): Promise<string>;
    resolveBinaryPath?(options?: {
        env?: NodeJS.ProcessEnv;
    }): string;
    nativeEnv?(options?: {
        env?: NodeJS.ProcessEnv;
    }): NodeJS.ProcessEnv;
    binaryWarning?(): string | undefined;
    diagnostics?(context: {
        cwd: string;
    }): Promise<Record<string, unknown>>;
    config?: {
        timeoutMs?: number;
        maxOutputBytes?: number;
        refreshIntervalMs?: number;
    };
    inspectIndexCompatibility?(context: {
        cwd: string;
    }): Promise<"ready" | "missing" | "incompatible">;
    rebuildIncompatibleIndex?(context: {
        cwd: string;
    }, options?: RunOptions): Promise<MachineEnvelope>;
    resolveIndexPath?(root: string): string;
    watchExternalChanges?: boolean;
};
export type FreshnessLike = Pick<FreshnessCoordinator, "ensureFresh" | "markAffectedPath"> & {
    markRootDirty?(root: string): void;
    shutdown?(): void;
};
export type ToolContext = {
    cwd: string;
};
export type CommandContext = ToolContext & {
    hasUI: boolean;
    ui: {
        notify(message: string, type?: "info" | "warning" | "error"): void;
    };
};
export type CommandResult = {
    diagnostics?: Record<string, unknown>;
} & ({
    ok: true;
    command: string;
    response: MachineEnvelope;
} | {
    ok: false;
    command: string;
    error: {
        code: string;
        message: string;
        details: Readonly<Record<string, unknown>>;
    };
});
export type Update = (result: {
    content: Array<{
        type: "text";
        text: string;
    }>;
    details: Record<string, unknown>;
}) => void;
export declare function bounded(text: string, maxChars?: number): string;
export declare function withNotes(text: string, notes?: string[]): string;
export declare function success(command: string, response: MachineEnvelope, extra?: {
    query?: string;
    mode?: string;
    activationMs?: number;
    backend?: string;
    freshness?: "stale";
    indexState?: "empty" | "ready";
    excerptLines?: number;
    notes?: string[];
}): {
    content: {
        type: "text";
        text: string;
    }[];
    details: {
        query?: string;
        mode?: string;
        activationMs?: number;
        backend?: string;
        freshness?: "stale";
        indexState?: "empty" | "ready";
        excerptLines?: number;
        notes?: string[];
        ok: boolean;
        command: string;
        response: {
            command: string;
            tool: "asgrep";
            schema_version: string;
            ok: boolean;
            version?: string;
            machine_schema_version?: string;
        };
    };
};
export declare function errorDetails(cause: unknown, signal?: AbortSignal): {
    code: string;
    message: string;
    details: Readonly<Record<string, unknown>>;
};
export declare function isFreshnessTimeout(cause: unknown, userSignal?: AbortSignal): boolean;
/** Leading or mid-query `in:path` scope used to bound a fresh-directory index. */
export declare function extractInPath(query: string): string | undefined;
export declare function failure(command: string, cause: unknown, signal?: AbortSignal): {
    content: {
        type: "text";
        text: string;
    }[];
    details: {
        ok: boolean;
        command: string;
        error: {
            code: string;
            message: string;
            details: Readonly<Record<string, unknown>>;
        };
    };
};
export declare function report(onUpdate: Update | undefined, command: string, phase: "started" | "completed"): void;
