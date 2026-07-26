import { constants } from "node:fs";
import { lstat, open, realpath } from "node:fs/promises";
import { isAbsolute, relative, resolve, sep } from "node:path";
import { RuntimeError } from "./runtime.js";
const DEFAULT_LIMIT = 20;
const MAX_LIMIT = 100;
const MAX_EXCERPT_LINES = 100;
const DEFAULT_MAX_READ_CHARS = 100_000;
const MAX_READ_CHARS = 1_000_000;
const MAX_READ_REFS = 20;
const MAX_SCAN_BYTES = 64 * 1024 * 1024;
const MAX_LINE_NUMBER = 0xffff_ffff;
const REF_PATTERN = /^(.+?)#L([1-9]\d*)-L([1-9]\d*)$/;
const KINDS = new Set(["asgrep", "def", "caller", "graph", "anchor", "import", "pattern", "embed"]);
const SIGNALS = new Set(["exact", "structural", "semantic"]);
function boundedInteger(value, fallback, minimum, maximum, name) {
    if (value === undefined)
        return fallback;
    if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
        throw new RuntimeError("INVALID_ARGUMENT", `${name} must be an integer from ${minimum} to ${maximum}`);
    }
    return value;
}
function requiredText(value, name) {
    if (typeof value !== "string" || value.trim().length === 0 || value.length > 4_096) {
        throw new RuntimeError("INVALID_ARGUMENT", `${name} must contain 1 to 4096 characters`);
    }
    return value.trim();
}
function outputArgs(options) {
    return [
        "--json",
        "--format",
        "agent-capsule",
        "--limit",
        String(boundedInteger(options.limit, DEFAULT_LIMIT, 1, MAX_LIMIT, "limit")),
        "--excerpt-lines",
        String(boundedInteger(options.excerptLines, 0, 0, MAX_EXCERPT_LINES, "excerptLines")),
    ];
}
function asSearchResponse(value) {
    if (value.ok !== true || !Array.isArray(value.hits)) {
        throw new RuntimeError("PROTOCOL_MISMATCH", "ast-sgrep search response is missing hits");
    }
    if (value.query !== undefined && typeof value.query !== "string") {
        throw new RuntimeError("PROTOCOL_MISMATCH", "ast-sgrep search response has an invalid query");
    }
    if (value.hit_count !== undefined
        && (typeof value.hit_count !== "number" || !Number.isSafeInteger(value.hit_count)
            || value.hit_count < 0 || value.hit_count !== value.hits.length)) {
        throw new RuntimeError("PROTOCOL_MISMATCH", "ast-sgrep search response has an invalid hit_count");
    }
    const optionalText = (field) => field === undefined || field === null || typeof field === "string";
    for (const candidate of value.hits) {
        if (!candidate || typeof candidate !== "object") {
            throw new RuntimeError("PROTOCOL_MISMATCH", "ast-sgrep returned an invalid search hit");
        }
        const hit = candidate;
        const lines = hit.lines;
        const validLines = !!lines && typeof lines === "object"
            && Number.isSafeInteger(lines.start)
            && Number.isSafeInteger(lines.end)
            && Number(lines.start) > 0
            && Number(lines.end) >= Number(lines.start);
        const valid = typeof hit.kind === "string" && KINDS.has(hit.kind)
            && typeof hit.signal === "string" && SIGNALS.has(hit.signal)
            && Array.isArray(hit.contributors) && hit.contributors.length > 0
            && hit.contributors.every((kind) => typeof kind === "string" && KINDS.has(kind))
            && typeof hit.score === "number" && Number.isFinite(hit.score)
            && typeof hit.margin === "number" && Number.isFinite(hit.margin) && hit.margin >= 0
            && typeof hit.file === "string" && hit.file.length > 0 && !isAbsolute(hit.file)
            && validLines
            && typeof hit.ref === "string"
            && typeof hit.preview === "string"
            && optionalText(hit.symbol) && optionalText(hit.caller) && optionalText(hit.callee)
            && optionalText(hit.language) && optionalText(hit.excerpt);
        if (!valid)
            throw new RuntimeError("PROTOCOL_MISMATCH", "ast-sgrep returned an invalid search hit");
        const parsed = parseRef(hit.ref);
        const hitLines = lines;
        if (parsed.file !== hit.file || parsed.start !== hitLines.start || parsed.end !== hitLines.end) {
            throw new RuntimeError("PROTOCOL_MISMATCH", "ast-sgrep hit ref does not match its file and lines");
        }
    }
    return value;
}
function refValue(value) {
    return typeof value === "string" ? value : value.ref;
}
function parseRef(ref) {
    const match = REF_PATTERN.exec(ref);
    if (!match)
        throw new RuntimeError("INVALID_REF", `Invalid ast-sgrep ref: ${ref}`);
    const file = match[1];
    const start = Number(match[2]);
    const end = Number(match[3]);
    if (isAbsolute(file) || !Number.isSafeInteger(start) || !Number.isSafeInteger(end)
        || start > MAX_LINE_NUMBER || end > MAX_LINE_NUMBER || end < start) {
        throw new RuntimeError("INVALID_REF", `Invalid ast-sgrep ref: ${ref}`);
    }
    return { file, start, end };
}
function inside(root, path) {
    const rel = relative(root, path);
    return rel === "" || (rel !== ".." && !rel.startsWith(`..${sep}`) && !isAbsolute(rel));
}
function checkAbort(signal) {
    if (signal?.aborted)
        throw new RuntimeError("CANCELLED", "ast-sgrep read was cancelled");
}
function boundedPrefix(value, maxChars) {
    let chars = 0;
    let end = 0;
    for (const codePoint of value) {
        if (chars >= maxChars)
            return { text: value.slice(0, end), chars, truncated: true };
        end += codePoint.length;
        chars += 1;
    }
    return { text: value, chars, truncated: false };
}
async function readLineWindow(handle, parsed, contextLines, maxChars, signal) {
    const stat = await handle.stat();
    if (!stat.isFile())
        throw new RuntimeError("READ_FAILED", `${parsed.file} is not a regular file`);
    const wantedStart = Math.max(1, parsed.start - contextLines);
    const wantedEnd = Math.min(MAX_LINE_NUMBER, parsed.end + contextLines);
    const decoder = new TextDecoder("utf-8", { fatal: true });
    const stream = handle.createReadStream({
        autoClose: false,
        highWaterMark: 64 * 1024,
        ...(signal ? { signal } : {}),
    });
    let pending = "";
    let lineNumber = 1;
    let selectedStart;
    let selectedEnd;
    let selectedLines = 0;
    let content = "";
    let contentChars = 0;
    let truncated = false;
    let rangeComplete = false;
    let scannedBytes = 0;
    const consumeLine = (line) => {
        if (lineNumber >= wantedStart && lineNumber <= wantedEnd) {
            selectedStart ??= lineNumber;
            selectedEnd = lineNumber;
            if (!truncated) {
                const addition = `${selectedLines > 0 ? "\n" : ""}${line.endsWith("\r") ? line.slice(0, -1) : line}`;
                const bounded = boundedPrefix(addition, maxChars - contentChars);
                content += bounded.text;
                contentChars += bounded.chars;
                truncated = bounded.truncated;
            }
            selectedLines += 1;
        }
        if (lineNumber >= wantedEnd)
            rangeComplete = true;
        lineNumber += 1;
    };
    try {
        for await (const chunk of stream) {
            checkAbort(signal);
            const bytes = chunk;
            const remainingScan = MAX_SCAN_BYTES - scannedBytes;
            const scanned = bytes.length > remainingScan + 1
                ? bytes.subarray(0, remainingScan + 1)
                : bytes;
            scannedBytes += scanned.length;
            try {
                pending += decoder.decode(scanned, { stream: true });
            }
            catch {
                throw new RuntimeError("BINARY_FILE", `${parsed.file} is not valid UTF-8 text`);
            }
            let newline = pending.indexOf("\n");
            while (newline >= 0) {
                consumeLine(pending.slice(0, newline));
                pending = pending.slice(newline + 1);
                if (rangeComplete)
                    break;
                newline = pending.indexOf("\n");
            }
            if (rangeComplete)
                break;
            if (scannedBytes > MAX_SCAN_BYTES || scanned.length < bytes.length) {
                throw new RuntimeError("READ_SCAN_LIMIT", `${parsed.file} exceeds the ${MAX_SCAN_BYTES}-byte scan limit`);
            }
            if (newline < 0 && lineNumber < wantedStart)
                pending = "";
        }
        if (!rangeComplete) {
            try {
                pending += decoder.decode();
            }
            catch {
                throw new RuntimeError("BINARY_FILE", `${parsed.file} is not valid UTF-8 text`);
            }
            if (pending.length > 0 || lineNumber === 1)
                consumeLine(pending);
        }
    }
    catch (cause) {
        if (signal?.aborted)
            throw new RuntimeError("CANCELLED", "ast-sgrep read was cancelled");
        throw cause;
    }
    finally {
        stream.destroy();
    }
    checkAbort(signal);
    if (parsed.start >= lineNumber || parsed.end >= lineNumber) {
        throw new RuntimeError("RANGE_OUT_OF_BOUNDS", `${parsed.file} has fewer than ${parsed.end} lines`);
    }
    return {
        file: parsed.file,
        lines: { start: selectedStart ?? wantedStart, end: selectedEnd ?? Math.max(wantedStart, lineNumber - 1) },
        content,
        truncated,
    };
}
export class SgrepCodeMode {
    runtime;
    context;
    #api;
    constructor(runtime, context) {
        this.runtime = runtime;
        this.context = context;
        this.#api = Object.freeze({
            keywordSearch: this.keywordSearch.bind(this),
            astSearch: this.astSearch.bind(this),
            semanticSearch: this.semanticSearch.bind(this),
            codeRead: this.codeRead.bind(this),
            find: this.find.bind(this),
            astFind: this.astFind.bind(this),
            semantic: this.semantic.bind(this),
            read: this.read.bind(this),
        });
    }
    async execute(plan) {
        if (typeof plan !== "function")
            throw new RuntimeError("INVALID_PLAN", "Code Mode plan must be a function");
        return await plan(this.#api);
    }
    async keywordSearch(query, options = {}) {
        const value = await this.runtime.run([...outputArgs(options), "keyword", "--", requiredText(query, "query"), "."], this.context, options);
        return asSearchResponse(value);
    }
    async astSearch(pattern, options = {}) {
        const query = `pattern: ${requiredText(pattern, "pattern")}`;
        const value = await this.runtime.run([...outputArgs(options), "--", query, "."], this.context, options);
        return asSearchResponse(value);
    }
    async semanticSearch(query, options = {}) {
        const value = await this.runtime.run([...outputArgs(options), "semantic", "--", requiredText(query, "query"), "."], this.context, options);
        return asSearchResponse(value);
    }
    async find(query, options) {
        return await this.keywordSearch(query, options);
    }
    async astFind(pattern, options) {
        return await this.astSearch(pattern, options);
    }
    async semantic(query, options) {
        return await this.semanticSearch(query, options);
    }
    async codeRead(ids, options = {}) {
        const values = Array.isArray(ids) ? ids : [ids];
        if (values.length === 0 || values.length > MAX_READ_REFS) {
            throw new RuntimeError("INVALID_ARGUMENT", `read requires 1 to ${MAX_READ_REFS} refs`);
        }
        const contextLines = boundedInteger(options.contextLines, 0, 0, 100, "contextLines");
        const maxChars = boundedInteger(options.maxChars, DEFAULT_MAX_READ_CHARS, 1, MAX_READ_CHARS, "maxChars");
        const perRefChars = Math.floor(maxChars / values.length);
        const remainder = maxChars % values.length;
        checkAbort(options.signal);
        const root = await realpath(await this.runtime.resolveRoot(this.context));
        const results = [];
        for (const [index, value] of values.entries()) {
            checkAbort(options.signal);
            const ref = refValue(value);
            const parsed = parseRef(ref);
            const unresolved = resolve(root, parsed.file);
            if (!inside(root, unresolved))
                throw new RuntimeError("PATH_OUTSIDE_ROOT", `Ref escapes the project root: ${ref}`);
            let filePath;
            let expectedStat;
            try {
                filePath = await realpath(unresolved);
                expectedStat = await lstat(filePath);
            }
            catch (cause) {
                throw new RuntimeError("READ_FAILED", `Unable to resolve ${parsed.file}`, {
                    ref,
                    cause: cause instanceof Error ? cause.message : String(cause),
                });
            }
            if (!inside(root, filePath))
                throw new RuntimeError("PATH_OUTSIDE_ROOT", `Ref escapes the project root: ${ref}`);
            if (!expectedStat.isFile())
                throw new RuntimeError("READ_FAILED", `${parsed.file} is not a regular file`);
            let handle;
            try {
                const noFollow = typeof constants.O_NOFOLLOW === "number" ? constants.O_NOFOLLOW : 0;
                handle = await open(filePath, constants.O_RDONLY | noFollow);
            }
            catch (cause) {
                throw new RuntimeError("READ_FAILED", `Unable to open ${parsed.file}`, {
                    ref,
                    cause: cause instanceof Error ? cause.message : String(cause),
                });
            }
            try {
                const [actualStat, openedPath] = await Promise.all([handle.stat(), realpath(unresolved)]);
                if (!inside(root, openedPath) || openedPath !== filePath
                    || actualStat.dev !== expectedStat.dev || actualStat.ino !== expectedStat.ino) {
                    throw new RuntimeError("PATH_CHANGED", `Ref changed while opening: ${ref}`);
                }
                const budget = perRefChars + (index < remainder ? 1 : 0);
                results.push({ ref, ...await readLineWindow(handle, parsed, contextLines, budget, options.signal) });
            }
            finally {
                await handle.close();
            }
        }
        return results;
    }
    async read(ids, options) {
        return await this.codeRead(ids, options);
    }
}
export function createSgrepCodeMode(runtime, context) {
    return new SgrepCodeMode(runtime, context);
}
