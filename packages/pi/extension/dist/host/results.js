/**
 * Tool-result plumbing shared by tools.ts and commands.ts: bounded text,
 * success/failure envelopes, freshness-timeout classification, reporting.
 */
import { isClosedWorkerError } from "../codemode/index.js";
import { RuntimeError } from "../runtime/types.js";
import { formatEditResult, formatIndexResult, formatReadResult, formatSearchResult, formatStatusResult } from "../ui/present.js";
export const MAX_CONTENT_CHARS = 8_000;
export function bounded(text) {
    return text.length <= MAX_CONTENT_CHARS ? text : `${text.slice(0, MAX_CONTENT_CHARS - 1)}…`;
}
export function success(command, response, extra = {}) {
    const text = command === "status"
        ? formatStatusResult(response)
        : command === "index" || command === "reindex"
            ? formatIndexResult(command, response)
            : command === "edit"
                ? formatEditResult(response)
                : command === "read"
                    ? formatReadResult(response)
                    : formatSearchResult(response, { command, ...extra });
    // Notes qualify the answer the agent is about to trust (stale index, empty
    // index): they stay in the model-visible text, not only in the details bag.
    const body = (extra.notes ?? []).length > 0
        ? `${text}\n${(extra.notes ?? []).map((note) => `  ! ${note}`).join("\n")}`
        : text;
    return {
        content: [{ type: "text", text: bounded(body) }],
        // The tool execute owns its machine command: normalize the envelope's
        // command (native catalog names like index_status/index_repo must surface
        // as the machine commands status/index/reindex).
        details: { ok: true, command, response: { ...response, command }, ...extra },
    };
}
/**
 * Failure families the native session raises as *text*: the NAPI boundary
 * carries `err.to_string()`, so the code has to be reconstructed from the
 * message instead of being read off a struct. Everything here maps to
 * OPERATIONAL_ERROR — a real answer about the index, not a mystery failure.
 *
 * Keep this list bounded and message-precise: an unrecognised failure stays
 * UNEXPECTED_ERROR, which is honest, while a false positive would hide one.
 */
const OPERATIONAL_FAILURES = [
    {
        pattern: /database is locked|database table is locked/i,
        hint: "another process holds the index write lock (a concurrent asgrep index build); retry in a few seconds, or scope the search with in:/fileFilter",
    },
    {
        pattern: /index changed while preparing|retry the rebuild/i,
        hint: "the index changed while this query was preparing; retry once the running index build finishes",
    },
    {
        pattern: /index is empty|index does not exist|failed to open index|failed to resolve index path|index schema version|unsupported schema|newer than supported/i,
    },
    {
        pattern: /database disk image is malformed|file is not a database|not a database/i,
        hint: "the index file is damaged; run /asgrep-reindex to rebuild it",
    },
    { pattern: /unable to open database file|no such table/i },
];
export function errorDetails(cause, signal) {
    if (signal?.aborted) {
        return { code: "CANCELLED", message: "cancelled", details: {} };
    }
    if (cause instanceof RuntimeError) {
        return { code: cause.code, message: cause.message, details: cause.details };
    }
    const message = cause instanceof Error ? cause.message : String(cause);
    if (/timed out after \d+ms|timeout after \d+ms|exceeded \d+ms/i.test(message)) {
        return { code: "TIMEOUT", message, details: {} };
    }
    if (isClosedWorkerError(cause)) {
        return {
            code: "SESSION_CLOSED",
            message: "asgrep session closed; retry the search",
            details: {},
        };
    }
    // An aborted call is not a mystery failure: the caller's deadline or cancel
    // fired. (Checked after timeout so a timed-out abort still reads as TIMEOUT.)
    if ((cause instanceof Error && cause.name === "AbortError") || /aborted|was cancelled|operation cancelled/i.test(message)) {
        return { code: "CANCELLED", message: "cancelled", details: {} };
    }
    for (const family of OPERATIONAL_FAILURES) {
        if (!family.pattern.test(message))
            continue;
        return {
            code: "OPERATIONAL_ERROR",
            message,
            details: family.hint ? { hint: family.hint } : {},
        };
    }
    return { code: "UNEXPECTED_ERROR", message, details: {} };
}
export function isFreshnessTimeout(cause, userSignal) {
    if (userSignal?.aborted)
        return false;
    if (cause instanceof RuntimeError && (cause.code === "TIMEOUT" || cause.code === "CANCELLED"))
        return true;
    if (isClosedWorkerError(cause))
        return true;
    const message = cause instanceof Error ? cause.message : String(cause);
    return /timed out after \d+ms|timeout after \d+ms|exceeded \d+ms/i.test(message);
}
/** Leading or mid-query `in:path` scope used to bound a fresh-directory index. */
export function extractInPath(query) {
    const match = /(?:^|\s)in:([^\s]+)/.exec(query);
    const path = match?.[1];
    if (!path || path.split(/[/\\]/u).includes(".."))
        return undefined;
    return path;
}
export function failure(command, cause, signal) {
    const error = errorDetails(cause, signal);
    return {
        content: [{ type: "text", text: bounded(`${command} failed [${error.code}]: ${error.message}`) }],
        details: { ok: false, command, error },
    };
}
export function report(onUpdate, command, phase) {
    onUpdate?.({
        content: [{ type: "text", text: `${command} ${phase}` }],
        details: { command, phase },
    });
}
