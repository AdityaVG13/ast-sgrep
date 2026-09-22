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

import { readFile, writeFile } from "node:fs/promises";
import { isAbsolute, join, relative, sep } from "node:path";
import { pathContained, realpathUtf8 as realpath } from "../runtime/index-health.js";

// Native caps (crates/ast-sgrep-codemode/src/io.rs + core limits).
const MAX_READ_REFS = 32;
const MAX_READ_CHARS = 100_000;
const MAX_EDITS = 16;
const MAX_LINE_CHARS = 2_000;
const MAX_INDEX_FILE_BYTES = 64 * 1024 * 1024;
const MAX_EXCERPT_LINES = 100;
const U32_MAX = 4_294_967_295;

type ReadSpec = { path: string; start: number; end: number };
type EditSpec = { path: string; oldText: string; newText: string };
const editTails = new Map<string, Promise<void>>();

function assertUnicode(text: string): void {
  if (Buffer.from(text, "utf8").toString("utf8") !== text) throw new Error("invalid Unicode: unpaired UTF-16 surrogate");
}

/** Mirror serde as_u64 (integers only, then wrapping `as u32`). */
function u32(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0 || value > Number.MAX_SAFE_INTEGER) return undefined;
  return value % (U32_MAX + 1);
}

/** Mirror serde as_u64 without narrowing (context/max budgets). */
function u64(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0 || value > Number.MAX_SAFE_INTEGER) return undefined;
  return value;
}

/** First present key wins even when its value is unusable (or_else chain). */
function present(args: Record<string, unknown>, ...keys: string[]): unknown {
  for (const key of keys) if (key in args) return args[key];
  return undefined;
}

/** Mirror str::lines: \n or \r\n separators, no phantom trailing line. */
function rustLines(text: string): string[] {
  if (text === "") return [];
  const lines = text.split("\n").map((line, index, all) =>
    index < all.length - 1 && line.endsWith("\r") ? line.slice(0, -1) : line);
  if (text.endsWith("\n")) lines.pop();
  return lines;
}

function charCount(text: string): number {
  return [...text].length;
}

function clampLine(line: string): { line: string; clamped: boolean } {
  if (charCount(line) <= MAX_LINE_CHARS) return { line, clamped: false };
  return { line: [...line].slice(0, MAX_LINE_CHARS).join(""), clamped: true };
}

/** Mirror session root_arg: configured root, optional contained override. */
async function jailRoot(configuredCwd: string, args: Record<string, unknown>): Promise<string> {
  const configured = await realpath(configuredCwd).catch(() => {
    throw new Error(`cannot resolve session root: ${configuredCwd}`);
  });
  const raw = args.root;
  if (typeof raw !== "string") return configured;
  assertUnicode(raw);
  const candidate = isAbsolute(raw) ? raw : join(configured, raw);
  const resolved = await realpath(candidate).catch(() => {
    throw new Error(`cannot resolve requested root: ${candidate}`);
  });
  if (!pathContained(configured, resolved)) {
    throw new Error(`requested root is outside the configured session root: ${resolved}`);
  }
  return resolved;
}

/** Mirror jail_rel_path: explicit '..' rejection, canonicalize, containment. */
async function jailPath(root: string, raw: string): Promise<{ abs: string; display: string }> {
  assertUnicode(raw);
  if (raw.split(sep === "\\" ? /[\\/]/u : "/").includes("..")) throw new Error("path must not contain '..'");
  const candidate = isAbsolute(raw) ? raw : join(root, raw);
  const abs = await realpath(candidate).catch(() => {
    throw new Error(`cannot resolve path ${raw}`);
  });
  if (!pathContained(root, abs)) throw new Error(`path escapes session root: ${raw}`);
  return { abs, display: relative(root, abs).split(sep).join("/") };
}

function parseU32Strict(text: string): number | undefined {
  const digits = /^(?:\+)?(\d+)$/.exec(text)?.[1];
  if (digits === undefined) return undefined;
  const value = Number(digits);
  return Number.isSafeInteger(value) && value <= U32_MAX ? value : undefined;
}

function parseRefString(raw: string): ReadSpec {
  const hash = raw.lastIndexOf("#L");
  if (hash === -1) return { path: raw, start: 1, end: 40 };
  const rest = raw.slice(hash + 2).trim();
  const dash = rest.indexOf("-L");
  const startText = dash === -1 ? rest : rest.slice(0, dash);
  const endText = dash === -1 ? rest : rest.slice(dash + 2);
  const start = parseU32Strict(startText);
  if (start === undefined) throw new Error(`invalid ref start in ${raw}`);
  const end = parseU32Strict(endText);
  if (end === undefined) throw new Error(`invalid ref end in ${raw}`);
  if (start === 0 || end < start) throw new Error(`invalid ref range in ${raw}`);
  return { path: raw.slice(0, hash), start, end };
}

function parseRefValue(value: unknown): ReadSpec {
  if (typeof value === "string") return parseRefString(value);
  if (value === null || typeof value !== "object" || Array.isArray(value)) throw new Error("ref must be a string or object");
  const obj = value as Record<string, unknown>;
  if (typeof obj.ref === "string") return parseRefString(obj.ref);
  const path = present(obj, "path", "file");
  if (typeof path !== "string") throw new Error("ref.path is required");
  const start = Math.max(1, u32(present(obj, "start", "line_start")) ?? 1);
  const end = u32(present(obj, "end", "line_end")) ?? start;
  return { path, start, end: Math.max(start, end) };
}

function collectRefs(args: Record<string, unknown>): ReadSpec[] {
  if (Array.isArray(args.refs)) return args.refs.map(parseRefValue);
  if (args.ref !== undefined) return [parseRefValue(args.ref)];
  const path = present(args, "path", "file");
  if (typeof path !== "string") throw new Error("path is required");
  const start = Math.max(1, u32(present(args, "start", "line_start")) ?? 1);
  const end = u32(present(args, "end", "line_end")) ?? start;
  return [{ path, start, end: Math.max(start, end) }];
}

function sliceWindow(numbered: Array<[number, string]>, start: number, end: number, maxChars: number): { text: string; actualStart: number; actualEnd: number; truncated: boolean } {
  let out = "";
  let actualStart = start;
  let actualEnd = start;
  let first = true;
  let truncated = false;
  let chars = 0;
  for (const [no, content] of numbered) {
    if (no < start) continue;
    if (no > end) break;
    const { line, clamped } = clampLine(content);
    if (clamped) truncated = true;
    const add = (first ? 0 : 1) + charCount(line);
    if (chars + add > maxChars) {
      truncated = true;
      break;
    }
    if (first) {
      actualStart = no;
      first = false;
    } else {
      out += "\n";
    }
    out += line;
    actualEnd = no;
    chars += add;
  }
  if (first) return { text: "", actualStart: start, actualEnd: start, truncated };
  return { text: out, actualStart, actualEnd, truncated };
}

async function readFileCapped(abs: string, display: string, signal?: AbortSignal): Promise<string> {
  const bytes = await readFile(abs, { signal }).catch(() => {
    signal?.throwIfAborted();
    throw new Error(`cannot read ${display}`);
  });
  if (bytes.length > MAX_INDEX_FILE_BYTES) throw new Error(`${display} exceeds max ${MAX_INDEX_FILE_BYTES} bytes`);
  signal?.throwIfAborted();
  try {
    // Match Rust's strict UTF-8 reads. Replacement decoding would corrupt bytes
    // outside the requested edit; ignoreBOM retains a literal source BOM.
    return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
  } catch {
    throw new Error(`cannot read ${display}: invalid UTF-8`);
  }
}

/** Mirror read_windows (disk-backed; see module note). */
export async function readWindowsFallback(
  configuredCwd: string,
  args: Record<string, unknown>,
  signal?: AbortSignal,
): Promise<{ ok: true; count: number; windows: Array<{ path: string; ref: string; start: number; end: number; truncated: boolean; text: string }> }> {
  signal?.throwIfAborted();
  const root = await jailRoot(configuredCwd, args);
  const contextLines = Math.min(u64(present(args, "context_lines", "contextLines")) ?? 0, MAX_EXCERPT_LINES);
  const maxChars = Math.min(Math.max(u64(present(args, "max_chars", "maxChars")) ?? MAX_READ_CHARS, 1), MAX_READ_CHARS);
  const refs = collectRefs(args);
  if (refs.length === 0) throw new Error("read requires path, ref, or refs");
  if (refs.length > MAX_READ_REFS) throw new Error(`read exceeds max ${MAX_READ_REFS} windows`);
  const windows: Array<{ path: string; ref: string; start: number; end: number; truncated: boolean; text: string }> = [];
  for (const spec of refs) {
    signal?.throwIfAborted();
    const { abs, display } = await jailPath(root, spec.path);
    const text = await readFileCapped(abs, display, signal);
    const numbered = rustLines(text).map((line, index): [number, string] => [index + 1, line]);
    const start = Math.max(1, spec.start - contextLines);
    const end = Math.min(spec.end + contextLines, U32_MAX);
    const slice = sliceWindow(numbered, start, end, maxChars);
    windows.push({
      path: display,
      ref: `${display}#L${slice.actualStart}-L${slice.actualEnd}`,
      start: slice.actualStart,
      end: slice.actualEnd,
      truncated: slice.truncated,
      text: slice.text,
    });
  }
  return { ok: true, count: windows.length, windows };
}

function parseEditValue(value: unknown): EditSpec {
  if (value === null || typeof value !== "object" || Array.isArray(value)) throw new Error("edit must be an object");
  const obj = value as Record<string, unknown>;
  const path = present(obj, "path", "file");
  if (typeof path !== "string") throw new Error("path is required");
  const oldText = present(obj, "oldText", "old_string", "old");
  if (typeof oldText !== "string") throw new Error("oldText is required");
  const newText = present(obj, "newText", "new_string", "new");
  if (typeof newText !== "string") throw new Error("newText is required");
  if (oldText === "") throw new Error("oldText must not be empty");
  assertUnicode(oldText);
  assertUnicode(newText);
  return { path, oldText, newText };
}

function collectEdits(args: Record<string, unknown>): EditSpec[] {
  if (Array.isArray(args.edits)) return args.edits.map(parseEditValue);
  if (typeof args.path === "string") return [parseEditValue(args)];
  return [];
}

function diffLines(text: string): { lines: string[]; more: boolean } {
  const all = rustLines(text);
  return { lines: all.slice(0, 24), more: all.length > 24 };
}

function matchLine(buffer: string, offset: number): number {
  return buffer.slice(0, offset).split("\n").length;
}

/** Mirror edit_files (validate-all-then-write; no targeted reindex — freshness covers). */
export async function editFilesFallback(
  configuredCwd: string,
  args: Record<string, unknown>,
  signal?: AbortSignal,
): Promise<{ ok: true; changed: number; edits: Array<{ path: string; changed: boolean; line: number; removed: string[]; added: string[]; truncated?: true }> }> {
  signal?.throwIfAborted();
  const root = await jailRoot(configuredCwd, args);
  // Native mutations serialize per session. Fallbacks can be reached through
  // different connectors, so their read/modify/write queue belongs here.
  const operation = (editTails.get(root) ?? Promise.resolve()).then(() => applyEdits(root, args, signal));
  const tail = operation.then(() => undefined, () => undefined);
  editTails.set(root, tail);
  try {
    return await operation;
  } finally {
    if (editTails.get(root) === tail) editTails.delete(root);
  }
}

async function applyEdits(root: string, args: Record<string, unknown>, signal?: AbortSignal) {
  signal?.throwIfAborted();
  const edits = collectEdits(args);
  if (edits.length === 0) throw new Error("edit requires path+oldText+newText or edits[]");
  if (edits.length > MAX_EDITS) throw new Error(`edit exceeds max ${MAX_EDITS} replacements`);
  // Phase 1: resolve, read, and compute every rewrite before any write.
  const plan = new Map<string, { abs: string; display: string; original: string; rewritten: string }>();
  const applied: Array<{ path: string; changed: boolean; line: number; removed: string[]; added: string[]; truncated?: true }> = [];
  for (const edit of edits) {
    signal?.throwIfAborted();
    const { abs, display } = await jailPath(root, edit.path);
    let entry = plan.get(display);
    if (!entry) {
      const original = await readFileCapped(abs, display, signal);
      entry = { abs, display, original, rewritten: original };
      plan.set(display, entry);
    }
    const first = entry.rewritten.indexOf(edit.oldText);
    if (first === -1) throw new Error("oldText must match exactly once (found 0)");
    if (entry.rewritten.indexOf(edit.oldText, first + edit.oldText.length) !== -1) {
      throw new Error("oldText must match exactly once (found 2+)");
    }
    const line = matchLine(entry.rewritten, first);
    const next = entry.rewritten.slice(0, first) + edit.newText + entry.rewritten.slice(first + edit.oldText.length);
    const changed = next !== entry.rewritten;
    entry.rewritten = next;
    const removed = diffLines(edit.oldText);
    const added = diffLines(edit.newText);
    applied.push({
      path: display,
      changed,
      line,
      removed: removed.lines,
      added: added.lines,
      ...(removed.more || added.more ? { truncated: true as const } : {}),
    });
  }
  // Phase 2: every edit validated — write each touched file once.
  for (const entry of plan.values()) {
    if (entry.rewritten === entry.original) continue;
    signal?.throwIfAborted();
    await writeFile(entry.abs, entry.rewritten).catch(() => {
      throw new Error(`cannot write ${entry.display}`);
    });
  }
  return { ok: true as const, changed: applied.filter((row) => row.changed).length, edits: applied };
}
