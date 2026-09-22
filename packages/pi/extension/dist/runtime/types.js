/**
 * Shared leaf module: version constants, wire types, RuntimeError.
 * Imports nothing from sibling modules — any file may depend on it.
 */
export const RUNTIME_VERSION = "2.5.4";
export const MACHINE_SCHEMA_VERSION = "1.0.0";
export const CONFIG_SCHEMA_VERSION = 1;
/** Index format this release ships. Must equal INDEX_SCHEMA_VERSION in crates/ast-sgrep-core (check-contract gates it). */
export const INDEX_FORMAT_VERSION = 16;
export const DEFAULT_TIMEOUT_MS = 30_000;
export const DEFAULT_MAX_OUTPUT_BYTES = 4 * 1024 * 1024;
export const DEFAULT_REFRESH_INTERVAL_MS = 30_000;
/** Max one caller waits on a shared index refresh before serving stale. */
export const DEFAULT_FRESHNESS_WAIT_MS = 10_000;
export const RESOLVED_ROOT = Symbol("resolvedRoot");
export class RuntimeError extends Error {
    code;
    details;
    constructor(code, message, details = {}) {
        super(message);
        this.code = code;
        this.details = details;
        this.name = "AstSgrepRuntimeError";
    }
}
