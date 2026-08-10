import { access, mkdir, readFile, writeFile } from "node:fs/promises";
import { constants } from "node:fs";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { RuntimeError } from "./runtime.js";
/** Max chars kept per line when scanning text for binary/UTF-8 checks (minified-line defense). */
export const MAX_EDIT_LINE_CHARS = 2_000;
/** Soft byte ceiling for replace targets before we require write mode. */
export const MAX_EDIT_REPLACE_BYTES = 128 * 1024;
const DEVICE_PATHS = new Set([
    "/dev/zero",
    "/dev/urandom",
    "/dev/random",
    "/dev/stdin",
    "/dev/stdout",
    "/dev/stderr",
    "/dev/null",
    "/dev/fd/0",
    "/dev/fd/1",
    "/dev/fd/2",
]);
function containedInRoot(root, candidate) {
    const rel = relative(root, candidate);
    return rel === "" || (!rel.startsWith(`..${sep}`) && rel !== ".." && !isAbsolute(rel));
}
/** Refuse device / proc fd paths before any I/O (cwd=/ is not a workspace boundary). */
export function assertSafeEditTarget(absolutePath) {
    const normalized = absolutePath.replace(/\\/g, "/");
    if (DEVICE_PATHS.has(normalized) || /^\/proc\/\d+\/fd\//.test(normalized)) {
        throw new RuntimeError("EDIT_FORBIDDEN_PATH", `${absolutePath} is a device or process fd path and cannot be edited`, { path: absolutePath });
    }
}
/** Repair common model path mistakes before resolve (validate-then-repair). */
export function repairEditPath(raw) {
    let path = raw.trim()
        .replace(/[\u2018\u2019\u201A\u201B]/g, "'")
        .replace(/[\u201C\u201D\u201E\u201F]/g, '"')
        .replace(/\u202F/g, " ");
    if ((path.startsWith('"') && path.endsWith('"')) ||
        (path.startsWith("'") && path.endsWith("'")) ||
        (path.startsWith("`") && path.endsWith("`"))) {
        path = path.slice(1, -1).trim();
    }
    path = path.replace(/\\ /g, " ");
    try {
        // NFD (macOS) → NFC so resolve matches on-disk names
        path = path.normalize("NFC");
    }
    catch {
        // keep as-is
    }
    return path;
}
function looksBinary(buf) {
    const sample = buf.subarray(0, Math.min(buf.length, 8_192));
    if (sample.includes(0))
        return true;
    try {
        sample.toString("utf8");
        return false;
    }
    catch {
        return true;
    }
}
/** Parse tool params into a root-bounded EditPlan. */
export function planEdit(params, projectRoot) {
    const rawPath = repairEditPath(params.path ?? "");
    if (!rawPath) {
        throw new RuntimeError("INVALID_EDIT", "path is required");
    }
    const absolutePath = resolve(projectRoot, rawPath);
    assertSafeEditTarget(absolutePath);
    if (!containedInRoot(projectRoot, absolutePath)) {
        throw new RuntimeError("EDIT_OUTSIDE_PROJECT", `Edit path resolves outside the project root. Use a path under ${projectRoot}`, { projectRoot, path: rawPath, resolved: absolutePath });
    }
    const displayPath = relative(projectRoot, absolutePath) || rawPath;
    const hasReplace = params.old_string !== undefined || params.new_string !== undefined;
    const hasWrite = params.contents !== undefined;
    if (hasReplace === hasWrite) {
        throw new RuntimeError("INVALID_EDIT", "Provide either old_string+new_string (replace) or contents (write), not both or neither");
    }
    if (hasWrite) {
        if (typeof params.contents !== "string") {
            throw new RuntimeError("INVALID_EDIT", "contents must be a string");
        }
        return { mode: "write", absolutePath, displayPath, contents: params.contents };
    }
    if (typeof params.old_string !== "string" || typeof params.new_string !== "string") {
        throw new RuntimeError("INVALID_EDIT", "replace mode requires old_string and new_string strings");
    }
    if (params.old_string.length === 0) {
        throw new RuntimeError("INVALID_EDIT", "old_string must be non-empty");
    }
    return {
        mode: "replace",
        absolutePath,
        displayPath,
        oldString: params.old_string,
        newString: params.new_string,
        replaceAll: params.replace_all === true,
    };
}
/** Apply a planned edit; returns structured result for the tool details. */
export async function applyEdit(plan) {
    assertSafeEditTarget(plan.absolutePath);
    if (plan.mode === "write") {
        let created = false;
        try {
            await access(plan.absolutePath, constants.F_OK);
        }
        catch {
            created = true;
            await mkdir(dirname(plan.absolutePath), { recursive: true });
        }
        await writeFile(plan.absolutePath, plan.contents, "utf8");
        return { path: plan.displayPath, mode: "write", created };
    }
    let raw;
    try {
        raw = await readFile(plan.absolutePath);
    }
    catch (cause) {
        throw new RuntimeError("EDIT_FILE_MISSING", `File not found: ${plan.displayPath}. Create it with contents=... or fix the path`, {
            path: plan.displayPath,
            cause: cause instanceof Error ? cause.message : String(cause),
        });
    }
    if (raw.length === 0) {
        throw new RuntimeError("EDIT_FILE_EMPTY", `${plan.displayPath} is empty. Use contents=... write mode instead of replace`, { path: plan.displayPath });
    }
    if (raw.length > MAX_EDIT_REPLACE_BYTES) {
        throw new RuntimeError("EDIT_FILE_TOO_LARGE", `${plan.displayPath} is ${raw.length} bytes (>${MAX_EDIT_REPLACE_BYTES}). Prefer a narrower old_string or write mode with intentional full contents`, { path: plan.displayPath, bytes: raw.length, maxBytes: MAX_EDIT_REPLACE_BYTES });
    }
    if (looksBinary(raw)) {
        throw new RuntimeError("BINARY_FILE", `${plan.displayPath} is not valid UTF-8 text`);
    }
    const text = raw.toString("utf8");
    for (const line of text.split("\n")) {
        if (line.length > MAX_EDIT_LINE_CHARS) {
            throw new RuntimeError("EDIT_LINE_TOO_LONG", `${plan.displayPath} has a line over ${MAX_EDIT_LINE_CHARS} chars. Use write mode or edit a non-minified source`, { path: plan.displayPath, maxLineChars: MAX_EDIT_LINE_CHARS });
        }
    }
    const occurrences = text.split(plan.oldString).length - 1;
    if (occurrences === 0) {
        throw new RuntimeError("EDIT_STRING_NOT_FOUND", `old_string was not found in ${plan.displayPath}. Re-read the file and copy an exact unique snippet`, { path: plan.displayPath });
    }
    if (!plan.replaceAll && occurrences > 1) {
        throw new RuntimeError("EDIT_STRING_AMBIGUOUS", `old_string matched ${occurrences} times in ${plan.displayPath}. Pass replace_all=true or provide a longer unique snippet`, { path: plan.displayPath, occurrences });
    }
    const next = plan.replaceAll
        ? text.split(plan.oldString).join(plan.newString)
        : text.replace(plan.oldString, plan.newString);
    await writeFile(plan.absolutePath, next, "utf8");
    return {
        path: plan.displayPath,
        mode: "replace",
        replacements: plan.replaceAll ? occurrences : 1,
    };
}
