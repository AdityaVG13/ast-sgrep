/**
 * Load the in-process Code Mode NAPI addon.
 *
 * Same model as MCP: Rust `CodeModeSession` runs inside the Node process.
 * No `asgrep` CLI spawn on the hot path.
 *
 * Resolution order:
 * 1. `ASGREP_CODEMODE_NAPI_PATH` (dev override)
 * 2. `@ast-sgrep/<platform>/ast-sgrep-codemode.node` via launcher (release install)
 * 3. Local `extension/native/` / cargo `target/release` (dev builds)
 */
export declare const CODEMODE_BINDING_VERSION = "2.0.0";
export type NativeSessionConfig = {
    root?: string;
    indexPath?: string;
    limit?: number;
    useEmbed?: boolean;
};
export type NativeBatchCall = {
    id: string;
    tool: string;
    args?: Record<string, unknown>;
};
export type NativeBatchResult = {
    id: string;
    ok: boolean;
    value?: unknown;
    error?: string;
};
export type NativeBatchResponse = {
    allOk: boolean;
    results: NativeBatchResult[];
    callCount: number;
    wallMs: number;
    mode: string;
};
export type NativeSession = {
    call(tool: string, args?: Record<string, unknown>, signal?: AbortSignal): Promise<unknown>;
    /** Sync bounded metadata/symbol lookup; omitted on older addons. Throws if busy. */
    callNow?(tool: string, args?: Record<string, unknown>): unknown;
    batch(calls: NativeBatchCall[], signal?: AbortSignal): Promise<NativeBatchResponse>;
    readonly callCount: number;
    readonly root: string;
};
export type CodemodeNativeBinding = {
    Session: new (config?: NativeSessionConfig) => NativeSession;
    bindingVersion(): string;
    isNative(): boolean;
    asyncApiVersion(): number;
};
/** Load the NAPI binding once. Returns null if unavailable on this host. */
export declare function loadCodemodeNative(): CodemodeNativeBinding | null;
export declare function nativeAvailable(): boolean;
/** Reset cache (tests). */
export declare function resetNativeCache(): void;
