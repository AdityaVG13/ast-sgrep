/**
 * Index-file health: format probing, completion classification,
 * rebuild-failure reporting, containment helper. Imports types + sqlite.
 */
import { existsSync, readdirSync } from "node:fs";
import { basename, dirname, extname, isAbsolute, join, relative, resolve } from "node:path";
import { INDEX_FORMAT_VERSION, RuntimeError } from "./types.js";
import { openIndexDatabase } from "./sqlite.js";
export function pathContained(parent, child) {
    const rel = relative(parent, child);
    const first = rel.split(/[\\/]/u, 1)[0];
    return rel === "" || (!isAbsolute(rel) && first !== "..");
}
export function record(value) {
    return value !== null && typeof value === "object" && !Array.isArray(value) ? value : undefined;
}
export function indexHealth(status) {
    const index = record(status.index);
    const state = typeof index?.status === "string" ? index.status :
        typeof status.index_status === "string" ? status.index_status : undefined;
    if (state === "incompatible" || index?.compatible === false || status.index_compatible === false)
        return "incompatible";
    if (state === "missing" || index?.exists === false || status.indexed === false)
        return "missing";
    if (state === "ready" || state === "current" || index?.exists === true || status.indexed === true)
        return "ready";
    if (typeof status.index_path === "string" && typeof status.file_count === "number") {
        // A present-but-empty index answers nothing: report it as unindexed so the
        // caller indexes instead of reading zero rows as a legitimate no-match.
        return status.file_count > 0 ? "ready" : "missing";
    }
    throw new RuntimeError("INDEX_STATUS_UNKNOWN", "ast-sgrep status did not report index freshness", { index: status.index, index_status: status.index_status });
}
export function incompatibleStatusFailure(cause) {
    // RuntimeError (CLI envelope) or a plain sticky/NAPI error — both carry the
    // native version-window message as text. The class gate is intentionally
    // dropped: this classifier only feeds the rebuild decision.
    const details = cause instanceof RuntimeError ? cause.details : undefined;
    const text = `${cause instanceof Error ? cause.message : String(cause)} ${JSON.stringify(details ?? {})}`;
    return /incompatib|unsupported.{0,24}schema|schema.{0,24}(version|mismatch)|(newer|older) than supported/i.test(text);
}
export function indexCompletion(response, requireWalkErrors) {
    const stats = record(response.stats) ?? response;
    const failed = stats.files_failed;
    const walkErrors = stats.walk_errors;
    if (!Number.isSafeInteger(failed) || failed < 0
        || (requireWalkErrors ? typeof walkErrors !== "boolean" : walkErrors !== undefined && typeof walkErrors !== "boolean")) {
        throw new RuntimeError("INDEX_RESPONSE_INVALID", "ast-sgrep index response omitted valid completion status", { filesFailed: failed, walkErrors, requireWalkErrors });
    }
    return {
        failed: failed,
        walkErrors: walkErrors === true,
    };
}
export function indexPathFor(root, env) {
    const configured = env.ASGREP_INDEX_PATH;
    if (!configured)
        return join(root, ".asgrep", "index.db");
    const resolved = resolve(root, configured);
    return extname(resolved) === ".db" ? resolved : join(resolved, "index.db");
}
export function indexQuarantines(indexPath) {
    const quarantinePrefix = `${basename(indexPath)}.corrupt`;
    try {
        return readdirSync(dirname(indexPath), { withFileTypes: true })
            .filter((entry) => entry.isFile() && (entry.name === quarantinePrefix || entry.name.startsWith(`${quarantinePrefix}.`)))
            .map((entry) => join(dirname(indexPath), entry.name))
            .sort();
    }
    catch {
        return [];
    }
}
/** Classify a rebuild failure and identify recovery copies made by this attempt. */
export function throwIndexRebuildFailed(cause, indexPath, quarantinesBefore) {
    const newQuarantines = indexQuarantines(indexPath).filter((path) => !quarantinesBefore.has(path));
    const recoveryPaths = [
        ...newQuarantines,
        ...(existsSync(indexPath) ? [indexPath] : []),
    ];
    const causeText = cause instanceof Error ? cause.message : String(cause);
    throw new RuntimeError("INDEX_REBUILD_FAILED", "Incompatible index rebuild failed: " + causeText + "; the prior index remains recoverable", {
        indexPath,
        recoveryPath: recoveryPaths[0] ?? indexPath,
        recoveryPaths,
        priorIndexPreserved: recoveryPaths.length > 0,
        expectedIndexFormat: INDEX_FORMAT_VERSION,
        cause: causeText,
        repair: "run /asgrep-reindex, or remove the project's .asgrep directory and run /asgrep-index",
    });
}
/** Read the on-disk index format marker. The binary is the authority on what it can read. */
export function inspectIndexFile(path) {
    if (!existsSync(path))
        return "missing";
    let database;
    try {
        database = openIndexDatabase(path, { readOnly: true });
        const row = database.prepare("PRAGMA user_version").get();
        const version = Number(Object.values(row ?? {})[0]);
        if (!Number.isSafeInteger(version) || version <= 0)
            return "incompatible";
        return version;
    }
    catch {
        return "incompatible";
    }
    finally {
        database?.close();
    }
}
