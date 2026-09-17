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
    return {
        content: [{ type: "text", text: bounded(text) }],
        // The tool execute owns its machine command: normalize the envelope's
        // command (native catalog names like index_status/index_repo must surface
        // as the machine commands status/index/reindex).
        details: { ok: true, command, response: { ...response, command }, ...extra },
    };
}
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
