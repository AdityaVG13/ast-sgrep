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
  /** Present when truncated: 1-indexed line to resume from (on the last shown line). */
  resumeOffset?: number;
  /** Named recovery hint for the model (empty/past-EOF/truncation). */
  note?: string;
}

export interface SgrepApi {
  keywordSearch(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
  astSearch(pattern: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
  semanticSearch(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
  codeRead(
    ids: SgrepRef | Pick<SgrepHit, "ref"> | readonly (SgrepRef | Pick<SgrepHit, "ref">)[],
    options?: SgrepReadOptions,
  ): Promise<SgrepReadResult[]>;
  /** Alias for keywordSearch. */
  find(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
  /** Alias for astSearch. */
  astFind(pattern: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
  /** Alias for semanticSearch. */
  semantic(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse>;
  /** Alias for codeRead. */
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
const MAX_LINE_CHARS = 2_000;
const DEVICE_PATHS = new Set([
  "/dev/zero", "/dev/urandom", "/dev/random", "/dev/stdin",
  "/dev/stdout", "/dev/stderr", "/dev/null", "/dev/fd/0", "/dev/fd/1", "/dev/fd/2",
]);

function assertSafeReadPath(absolutePath: string): void {
  const normalized = absolutePath.replace(/\\/g, "/");
  if (DEVICE_PATHS.has(normalized) || /^\/proc\/\d+\/fd\//.test(normalized)) {
    throw new RuntimeError(
      "READ_FORBIDDEN_PATH",
      `${absolutePath} is a device or process fd path and cannot be read`,
      { path: absolutePath },
    );
  }
}

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

function boundedPrefix(value: string, maxChars: number): { text: string; chars: number; truncated: boolean } {
  let chars = 0;
  let end = 0;
  for (const codePoint of value) {
    if (chars >= maxChars) return { text: value.slice(0, end), chars, truncated: true };
    end += codePoint.length;
    chars += 1;
  }
  return { text: value, chars, truncated: false };
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
  let contentChars = 0;
  let truncated = false;
  let rangeComplete = false;
  let scannedBytes = 0;

  const consumeLine = (line: string): void => {
    if (lineNumber >= wantedStart && lineNumber <= wantedEnd) {
      selectedStart ??= lineNumber;
      selectedEnd = lineNumber;
      if (!truncated) {
        const rawLine = line.endsWith("\r") ? line.slice(0, -1) : line;
        const clamped = rawLine.length > MAX_LINE_CHARS ? `${rawLine.slice(0, MAX_LINE_CHARS)}…` : rawLine;
        const addition = `${selectedLines > 0 ? "\n" : ""}${clamped}`;
        const bounded = boundedPrefix(addition, maxChars - contentChars);
        content += bounded.text;
        contentChars += bounded.chars;
        truncated = bounded.truncated;
      }
      selectedLines += 1;
    }
    if (lineNumber >= wantedEnd) rangeComplete = true;
    lineNumber += 1;
  };

  try {
    for await (const chunk of stream) {
      checkAbort(signal);
      const bytes = chunk as Buffer;
      const remainingScan = MAX_SCAN_BYTES - scannedBytes;
      const scanned = bytes.length > remainingScan + 1
        ? bytes.subarray(0, remainingScan + 1)
        : bytes;
      scannedBytes += scanned.length;
      try {
        pending += decoder.decode(scanned, { stream: true });
      } catch {
        throw new RuntimeError("BINARY_FILE", `${parsed.file} is not valid UTF-8 text`);
      }
      let newline = pending.indexOf("\n");
      while (newline >= 0) {
        consumeLine(pending.slice(0, newline));
        pending = pending.slice(newline + 1);
        if (rangeComplete) break;
        newline = pending.indexOf("\n");
      }
      if (rangeComplete) break;
      if (scannedBytes > MAX_SCAN_BYTES || scanned.length < bytes.length) {
        throw new RuntimeError("READ_SCAN_LIMIT", `${parsed.file} exceeds the ${MAX_SCAN_BYTES}-byte scan limit`);
      }
      if (newline < 0 && lineNumber < wantedStart) pending = "";
    }
    if (!rangeComplete) {
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
  const totalLines = Math.max(0, lineNumber - 1);
  if (totalLines === 0) {
    return {
      file: parsed.file,
      lines: { start: 1, end: 0 },
      content: "",
      truncated: false,
      note: `${parsed.file} is empty`,
    };
  }
  if (parsed.start > totalLines || parsed.end > totalLines) {
    const resume = Math.max(1, totalLines);
    throw new RuntimeError(
      "RANGE_OUT_OF_BOUNDS",
      `Note: offset ${parsed.start} is beyond the end of ${parsed.file} (${totalLines} lines scanned). Retry with a smaller offset (e.g. start=${resume})`,
      { file: parsed.file, start: parsed.start, end: parsed.end, totalLines, resumeOffset: resume },
    );
  }
  const endLine = selectedEnd ?? Math.max(wantedStart, totalLines);
  return {
    file: parsed.file,
    lines: { start: selectedStart ?? wantedStart, end: endLine },
    content,
    truncated,
    ...(truncated
      ? {
          resumeOffset: endLine,
          note: `truncated at line ${endLine}; resume with start=${endLine}`,
        }
      : {}),
  };
}

async function runSearch(
  runtime: RuntimeLike,
  context: RuntimeContext,
  command: readonly string[],
  query: string,
  options: SgrepSearchOptions,
): Promise<SgrepSearchResponse> {
  const value = await runtime.run([...outputArgs(options), ...command, "--", query, "."], context, options);
  return asSearchResponse(value);
}

async function resolveReadableFile(
  root: string,
  ref: SgrepRef,
  parsed: { file: string; start: number; end: number },
): Promise<{ unresolved: string; filePath: string; expectedStat: Stats }> {
  const unresolved = resolve(root, parsed.file);
  assertSafeReadPath(unresolved);
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
  return { unresolved, filePath, expectedStat };
}

async function openStableHandle(
  root: string,
  ref: SgrepRef,
  fileLabel: string,
  unresolved: string,
  filePath: string,
  expectedStat: Stats,
): Promise<FileHandle> {
  let handle: FileHandle;
  try {
    const noFollow = typeof constants.O_NOFOLLOW === "number" ? constants.O_NOFOLLOW : 0;
    handle = await open(filePath, constants.O_RDONLY | noFollow);
  } catch (cause) {
    throw new RuntimeError("READ_FAILED", `Unable to open ${fileLabel}`, {
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
    return handle;
  } catch (cause) {
    await handle.close();
    throw cause;
  }
}

export class SgrepCodeMode implements SgrepApi {
  readonly #api: Readonly<SgrepApi>;

  constructor(private readonly runtime: RuntimeLike, private readonly context: RuntimeContext) {
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

  async execute<T>(plan: SgrepPlan<T>): Promise<T> {
    if (typeof plan !== "function") throw new RuntimeError("INVALID_PLAN", "Code Mode plan must be a function");
    return await plan(this.#api);
  }

  async keywordSearch(query: string, options: SgrepSearchOptions = {}): Promise<SgrepSearchResponse> {
    return runSearch(this.runtime, this.context, ["keyword"], requiredText(query, "query"), options);
  }

  async astSearch(pattern: string, options: SgrepSearchOptions = {}): Promise<SgrepSearchResponse> {
    return runSearch(this.runtime, this.context, [], `pattern: ${requiredText(pattern, "pattern")}`, options);
  }

  async semanticSearch(query: string, options: SgrepSearchOptions = {}): Promise<SgrepSearchResponse> {
    return runSearch(this.runtime, this.context, ["semantic"], requiredText(query, "query"), options);
  }

  async find(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse> {
    return this.keywordSearch(query, options);
  }

  async astFind(pattern: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse> {
    return this.astSearch(pattern, options);
  }

  async semantic(query: string, options?: SgrepSearchOptions): Promise<SgrepSearchResponse> {
    return this.semanticSearch(query, options);
  }

  async codeRead(
    ids: SgrepRef | Pick<SgrepHit, "ref"> | readonly (SgrepRef | Pick<SgrepHit, "ref">)[],
    options: SgrepReadOptions = {},
  ): Promise<SgrepReadResult[]> {
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
    const results: SgrepReadResult[] = [];
    for (const [index, value] of values.entries()) {
      checkAbort(options.signal);
      const ref = refValue(value);
      const parsed = parseRef(ref);
      const { unresolved, filePath, expectedStat } = await resolveReadableFile(root, ref, parsed);
      const handle = await openStableHandle(root, ref, parsed.file, unresolved, filePath, expectedStat);
      try {
        const budget = perRefChars + (index < remainder ? 1 : 0);
        results.push({ ref, ...await readLineWindow(handle, parsed, contextLines, budget, options.signal) });
      } finally {
        await handle.close();
      }
    }
    return results;
  }

  async read(
    ids: SgrepRef | Pick<SgrepHit, "ref"> | readonly (SgrepRef | Pick<SgrepHit, "ref">)[],
    options?: SgrepReadOptions,
  ): Promise<SgrepReadResult[]> {
    return await this.codeRead(ids, options);
  }
}

export function createSgrepCodeMode(runtime: RuntimeLike, context: RuntimeContext): SgrepCodeMode {
  return new SgrepCodeMode(runtime, context);
}
