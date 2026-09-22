/**
 * Extension-side read/edit for launchers that predate the native tools.
 *
 * Severed-lane contract: when native answers `unknown tool: <name>` for
 * read/edit/find, the connector serves the call here instead of failing, so
 * an extension tracking main keeps working on an official launcher. Shapes,
 * defaults, caps, and confinement follow crates/ast-sgrep-codemode/src/io.rs.
 * Errors are plain Errors; adapter-specific filesystem messages can differ.
 *
 * Deliberate divergences from native, both strictly safer:
 * - Reads come from disk, never the index (the extension has no row reader
 *   here). Disk is fresher than a possibly stale index row.
 * - Edits skip the native targeted reindex. A fallback edit is exactly like
 *   an external editor write, which the freshness layer already reconciles.
 */
/** Mirror read_windows (disk-backed; see module note). */
export declare function readWindowsFallback(configuredCwd: string, args: Record<string, unknown>, signal?: AbortSignal): Promise<{
    ok: true;
    count: number;
    windows: Array<{
        path: string;
        ref: string;
        start: number;
        end: number;
        truncated: boolean;
        text: string;
    }>;
}>;
/** Mirror edit_files (validate-all-then-write; no targeted reindex — freshness covers). */
export declare function editFilesFallback(configuredCwd: string, args: Record<string, unknown>, signal?: AbortSignal): Promise<{
    ok: true;
    changed: number;
    edits: Array<{
        path: string;
        changed: boolean;
        line: number;
        removed: string[];
        added: string[];
        truncated?: true;
    }>;
}>;
