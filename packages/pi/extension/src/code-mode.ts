import { constants, type Stats } from "node:fs";
import { lstat, open, realpath, type FileHandle } from "node:fs/promises";
import { isAbsolute, relative, resolve, sep } from "node:path";
import { RuntimeError, type AstSgrepRuntime, type MachineEnvelope, type RunOptions, type RuntimeContext } from "./runtime.js";

export type SgrepKind = "asgrep" | "def" | "caller" | "graph" | "anchor" | "import" | "pattern" | "embed";
export type SgrepSignal = "exact" | "structural" | "semantic";
export type SgrepRef = `${string}#L${number}-L${number}`;

export interface SgrepHit {
  kind: SgrepKind;
  signal: SgrepSignal;
  contributors: SgrepKind[];
  score: number;
  margin: number;
  file: string;
  lines: { start: number; end: number };
  ref: SgrepRef;
  preview: string;
  symbol?: string | null;
  caller?: string | null;
  callee?: string | null;
  language?: string | null;
  excerpt?: string;
}

export interface SgrepSearchResponse extends MachineEnvelope {
  hits: SgrepHit[];
  query?: string;
  hit_count?: number;
}

export interface SgrepSearchOptions extends RunOptions {
  limit?: number;
  excerptLines?: number;
}

export interface SgrepReadOptions {
  contextLines?: number;
  /** Aggregate character budget across all refs. */
  maxChars?: number;
  signal?: AbortSignal;
}

export interface SgrepReadResult {
  ref: SgrepRef;
  file: string;
  lines: { start: number; end: number };
  content: string;
  truncated: boolean;
}

export interface SgrepApi {
  find(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
  astFind(pattern: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
  semantic(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
  read(
    ids: SgrepRef | Pick<SgrepHit, "ref"> | readonly (SgrepRef | Pick<SgrepHit, "ref">)[],
    options?: SgrepReadOptions,
  ): Promise<SgrepReadResult[]>;
}

export type SgrepPlan<T> = (sgrep: Readonly<SgrepApi>) => T | Promise<T>;

type RuntimeLike = Pick<AstSgrepRuntime, "run" | "resolveRoot">;

const DEFAULT_LIMIT = 20;
const MAX_LIMIT = 100;
const MAX_EXCERPT_LINES = 100;
const DEFAULT_MAX_READ_CHARS = 100_000;
const MAX_READ_CHARS = 1_000_000;
const MAX_READ_REFS = 20;
const MAX_SCAN_BYTES = 64 * 1024 * 1024;
const MAX_LINE_NUMBER = 0xffff_ffff;
const REF_PATTERN = /^(.+?)#L([1-9]\d*)-L([1-9]\d*)$/;
const KINDS = new Set<SgrepKind>(["asgrep", "def", "caller", "graph", "anchor", "import", "pattern", "embed"]);
const SIGNALS = new Set<SgrepSignal>(["exact", "structural", "semantic"]);

function boundedInteger(value: number | undefined, fallback: number, minimum: number, maximum: number, name: string): number {
  if (value === undefined) return fallback;
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    throw new RuntimeError("INVALID_ARGUMENT", `${name} must be an integer from ${minimum} to ${maximum}`);
  }
  return value;
}

function requiredText(value: string, name: string): string {
  if (typeof value !== "string" || value.trim().length === 0 || value.length > 4_096) {
    throw new RuntimeError("INVALID_ARGUMENT", `${name} must contain 1 to 4096 characters`);
  }
  return value.trim();
}

function outputArgs(options: SgrepSearchOptions): string[] {
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

function asSearchResponse(value: MachineEnvelope): SgrepSearchResponse {
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
  const optionalText = (field: unknown): boolean => field === undefined || field === null || typeof field === "string";
  for (const candidate of value.hits) {
    if (!candidate || typeof candidate !== "object") {
      throw new RuntimeError("PROTOCOL_MISMATCH", "ast-sgrep returned an invalid search hit");
    }
    const hit = candidate as Record<string, unknown>;
    const lines = hit.lines;
    const validLines = !!lines && typeof lines === "object"
      && Number.isSafeInteger((lines as Record<string, unknown>).start)
      && Number.isSafeInteger((lines as Record<string, unknown>).end)
      && Number((lines as Record<string, unknown>).start) > 0
      && Number((lines as Record<string, unknown>).end) >= Number((lines as Record<string, unknown>).start);
    const valid = typeof hit.kind === "string" && KINDS.has(hit.kind as SgrepKind)
      && typeof hit.signal === "string" && SIGNALS.has(hit.signal as SgrepSignal)
      && Array.isArray(hit.contributors) && hit.contributors.length > 0
      && hit.contributors.every((kind) => typeof kind === "string" && KINDS.has(kind as SgrepKind))
      && typeof hit.score === "number" && Number.isFinite(hit.score)
      && typeof hit.margin === "number" && Number.isFinite(hit.margin) && hit.margin >= 0
      && typeof hit.file === "string" && hit.file.length > 0 && !isAbsolute(hit.file)
      && validLines
      && typeof hit.ref === "string"
      && typeof hit.preview === "string"
      && optionalText(hit.symbol) && optionalText(hit.caller) && optionalText(hit.callee)
      && optionalText(hit.language) && optionalText(hit.excerpt);
    if (!valid) throw new RuntimeError("PROTOCOL_MISMATCH", "ast-sgrep returned an invalid search hit");
    const parsed = parseRef(hit.ref as SgrepRef);
    const hitLines = lines as { start: number; end: number };
    if (parsed.file !== hit.file || parsed.start !== hitLines.start || parsed.end !== hitLines.end) {
      throw new RuntimeError("PROTOCOL_MISMATCH", "ast-sgrep hit ref does not match its file and lines");
    }
  }
  return value as SgrepSearchResponse;
}

function refValue(value: SgrepRef | Pick<SgrepHit, "ref">): SgrepRef {
  return typeof value === "string" ? value : value.ref;
}

function parseRef(ref: SgrepRef): { file: string; start: number; end: number } {
  const match = REF_PATTERN.exec(ref);
  if (!match) throw new RuntimeError("INVALID_REF", `Invalid ast-sgrep ref: ${ref}`);
  const file = match[1]!;
  const start = Number(match[2]);
  const end = Number(match[3]);
  if (isAbsolute(file) || !Number.isSafeInteger(start) || !Number.isSafeInteger(end)
    || start > MAX_LINE_NUMBER || end > MAX_LINE_NUMBER || end < start) {
    throw new RuntimeError("INVALID_REF", `Invalid ast-sgrep ref: ${ref}`);
  }
  return { file, start, end };
}

function inside(root: string, path: string): boolean {
  const rel = relative(root, path);
  return rel === "" || (rel !== ".." && !rel.startsWith(`..${sep}`) && !isAbsolute(rel));
}

function checkAbort(signal: AbortSignal | undefined): void {
  if (signal?.aborted) throw new RuntimeError("CANCELLED", "ast-sgrep read was cancelled");
}

function safePrefix(value: string, maxChars: number): string {
  if (value.length <= maxChars) return value;
  let end = maxChars;
  if (end > 0 && /[\uD800-\uDBFF]/.test(value[end - 1]!)) end -= 1;
  return value.slice(0, end);
}

async function readLineWindow(
  handle: FileHandle,
  parsed: { file: string; start: number; end: number },
  contextLines: number,
  maxChars: number,
  signal: AbortSignal | undefined,
): Promise<Omit<SgrepReadResult, "ref">> {
  const stat = await handle.stat();
  if (!stat.isFile()) throw new RuntimeError("READ_FAILED", `${parsed.file} is not a regular file`);
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
  let selectedStart: number | undefined;
  let selectedEnd: number | undefined;
  let selectedLines = 0;
  let content = "";
  let truncated = false;
  let complete = false;
  let scannedBytes = 0;

  const consumeLine = (line: string): void => {
    if (lineNumber >= wantedStart && lineNumber <= wantedEnd) {
      selectedStart ??= lineNumber;
      selectedEnd = lineNumber;
      const addition = `${selectedLines > 0 ? "\n" : ""}${line.endsWith("\r") ? line.slice(0, -1) : line}`;
      selectedLines += 1;
      const remaining = maxChars - content.length;
      if (addition.length > remaining) {
        content += safePrefix(addition, remaining);
        truncated = true;
        complete = true;
      } else {
        content += addition;
      }
    }
    if (lineNumber >= wantedEnd) complete = true;
    lineNumber += 1;
  };

  try {
    for await (const chunk of stream) {
      checkAbort(signal);
      const bytes = chunk as Buffer;
      scannedBytes += bytes.length;
      if (scannedBytes > MAX_SCAN_BYTES) {
        throw new RuntimeError("READ_SCAN_LIMIT", `${parsed.file} exceeds the ${MAX_SCAN_BYTES}-byte scan limit`);
      }
      try {
        pending += decoder.decode(bytes, { stream: true });
      } catch {
        throw new RuntimeError("BINARY_FILE", `${parsed.file} is not valid UTF-8 text`);
      }
      let newline = pending.indexOf("\n");
      while (newline >= 0) {
        consumeLine(pending.slice(0, newline));
        pending = pending.slice(newline + 1);
        if (complete) break;
        newline = pending.indexOf("\n");
      }
      if (complete) break;
      if (newline < 0 && lineNumber < wantedStart) pending = "";
      if (lineNumber >= wantedStart && pending.length > maxChars - content.length) {
        consumeLine(pending);
        pending = "";
      }
    }
    if (!complete) {
      try {
        pending += decoder.decode();
      } catch {
        throw new RuntimeError("BINARY_FILE", `${parsed.file} is not valid UTF-8 text`);
      }
      if (pending.length > 0 || lineNumber === 1) consumeLine(pending);
    }
  } catch (cause) {
    if (signal?.aborted) throw new RuntimeError("CANCELLED", "ast-sgrep read was cancelled");
    throw cause;
  } finally {
    stream.destroy();
  }

  checkAbort(signal);
  if (parsed.start >= lineNumber && selectedStart === undefined) {
    throw new RuntimeError("RANGE_OUT_OF_BOUNDS", `${parsed.file} has fewer than ${parsed.start} lines`);
  }
  return {
    file: parsed.file,
    lines: { start: selectedStart ?? wantedStart, end: selectedEnd ?? Math.max(wantedStart, lineNumber - 1) },
    content,
    truncated,
  };
}

export class SgrepCodeMode implements SgrepApi {
  readonly #api: Readonly<SgrepApi>;

  constructor(private readonly runtime: RuntimeLike, private readonly context: RuntimeContext) {
    this.#api = Object.freeze({
      find: this.find.bind(this),
      astFind: this.astFind.bind(this),
      semantic: this.semantic.bind(this),
      read: this.read.bind(this),
    });
  }

  async execute<T>(plan: SgrepPlan<T>): Promise<T> {
    if (typeof plan !== "function") throw new RuntimeError("INVALID_PLAN", "Code Mode plan must be a function");
    return await plan(this.#api);
  }

  async find(query: string, options: SgrepSearchOptions = {}): Promise<SgrepSearchResponse> {
    const value = await this.runtime.run([...outputArgs(options), "--", requiredText(query, "query"), "."], this.context, options);
    return asSearchResponse(value);
  }

  async astFind(pattern: string, options: SgrepSearchOptions = {}): Promise<SgrepSearchResponse> {
    const query = `pattern: ${requiredText(pattern, "pattern")}`;
    const value = await this.runtime.run([...outputArgs(options), "--", query, "."], this.context, options);
    return asSearchResponse(value);
  }

  async semantic(query: string, options: SgrepSearchOptions = {}): Promise<SgrepSearchResponse> {
    const value = await this.runtime.run([...outputArgs(options), "semantic", "--", requiredText(query, "query"), "."], this.context, options);
    return asSearchResponse(value);
  }

  async read(
    ids: SgrepRef | Pick<SgrepHit, "ref"> | readonly (SgrepRef | Pick<SgrepHit, "ref">)[],
    options: SgrepReadOptions = {},
  ): Promise<SgrepReadResult[]> {
    const values = Array.isArray(ids) ? ids : [ids];
    if (values.length === 0 || values.length > MAX_READ_REFS) {
      throw new RuntimeError("INVALID_ARGUMENT", `read requires 1 to ${MAX_READ_REFS} refs`);
    }
    const contextLines = boundedInteger(options.contextLines, 0, 0, 100, "contextLines");
    const maxChars = boundedInteger(options.maxChars, DEFAULT_MAX_READ_CHARS, values.length, MAX_READ_CHARS, "maxChars");
    const perRefChars = Math.floor(maxChars / values.length);
    checkAbort(options.signal);
    const root = await realpath(await this.runtime.resolveRoot(this.context));
    const results: SgrepReadResult[] = [];
    for (const value of values) {
      checkAbort(options.signal);
      const ref = refValue(value);
      const parsed = parseRef(ref);
      const unresolved = resolve(root, parsed.file);
      if (!inside(root, unresolved)) throw new RuntimeError("PATH_OUTSIDE_ROOT", `Ref escapes the project root: ${ref}`);
      let filePath: string;
      let expectedStat: Stats;
      try {
        filePath = await realpath(unresolved);
        expectedStat = await lstat(filePath);
      } catch (cause) {
        throw new RuntimeError("READ_FAILED", `Unable to resolve ${parsed.file}`, {
          ref,
          cause: cause instanceof Error ? cause.message : String(cause),
        });
      }
      if (!inside(root, filePath)) throw new RuntimeError("PATH_OUTSIDE_ROOT", `Ref escapes the project root: ${ref}`);
      if (!expectedStat.isFile()) throw new RuntimeError("READ_FAILED", `${parsed.file} is not a regular file`);
      let handle: FileHandle;
      try {
        const noFollow = typeof constants.O_NOFOLLOW === "number" ? constants.O_NOFOLLOW : 0;
        handle = await open(filePath, constants.O_RDONLY | noFollow);
      } catch (cause) {
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
        results.push({ ref, ...await readLineWindow(handle, parsed, contextLines, perRefChars, options.signal) });
      } finally {
        await handle.close();
      }
    }
    return results;
  }
}

export function createSgrepCodeMode(runtime: RuntimeLike, context: RuntimeContext): SgrepCodeMode {
  return new SgrepCodeMode(runtime, context);
}
