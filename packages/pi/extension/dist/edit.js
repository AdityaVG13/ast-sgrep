import { access, mkdir, readFile, writeFile } from "node:fs/promises";
import { constants } from "node:fs";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { RuntimeError } from "./runtime.js";
function containedInRoot(root, candidate) {
    const rel = relative(root, candidate);
    return rel === "" || (!rel.startsWith(`..${sep}`) && rel !== ".." && !isAbsolute(rel));
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
    const rawPath = params.path?.trim();
    if (!rawPath) {
        throw new RuntimeError("INVALID_EDIT", "path is required");
    }
    const absolutePath = resolve(projectRoot, rawPath);
    if (!containedInRoot(projectRoot, absolutePath)) {
        throw new RuntimeError("EDIT_OUTSIDE_PROJECT", "Edit path resolves outside the project root", {
            projectRoot,
            path: rawPath,
            resolved: absolutePath,
        });
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
        throw new RuntimeError("EDIT_FILE_MISSING", `File not found: ${plan.displayPath}`, {
            path: plan.displayPath,
            cause: cause instanceof Error ? cause.message : String(cause),
        });
    }
    if (looksBinary(raw)) {
        throw new RuntimeError("BINARY_FILE", `${plan.displayPath} is not valid UTF-8 text`);
    }
    const text = raw.toString("utf8");
    const occurrences = text.split(plan.oldString).length - 1;
    if (occurrences === 0) {
        throw new RuntimeError("EDIT_STRING_NOT_FOUND", "old_string was not found in the file", {
            path: plan.displayPath,
        });
    }
    if (!plan.replaceAll && occurrences > 1) {
        throw new RuntimeError("EDIT_STRING_AMBIGUOUS", `old_string matched ${occurrences} times; pass replace_all=true or provide a unique snippet`, { path: plan.displayPath, occurrences });
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
