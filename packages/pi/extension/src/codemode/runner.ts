import type { AsgrepConnector } from "./connector.js";
import type { DispatchStats } from "./dispatch.js";

/** Closed sum: success|failure — `ok:true` with `error` (or `ok:false` without) is unrepresentable. */
export type CodemodeRunSuccess = {
  ok: true;
  result: unknown;
  logs: string[];
  code: string;
  stats?: DispatchStats;
  wallMs: number;
};

export type CodemodeRunFailure = {
  ok: false;
  result: null;
  error: string;
  logs: string[];
  code: string;
  stats?: DispatchStats;
  wallMs: number;
};

export type CodemodeRunResult = CodemodeRunSuccess | CodemodeRunFailure;

const DEFAULT_TIMEOUT_MS = 30_000;
const MAX_CODE_CHARS = 32_000;

/** Strip markdown fences and normalize to an async IIFE expression. */
export function normalizeCode(raw: string): string {
  let code = raw.trim();
  if (code.startsWith("```")) {
    code = code.replace(/^```(?:javascript|js|typescript|ts)?\s*/i, "").replace(/\s*```$/, "").trim();
  }
  if (/^async\s*\(/.test(code) || /^async\s+function\b/.test(code)) {
    return `(${code.endsWith(";") ? code.slice(0, -1) : code})()`;
  }
  return `(async () => {\n${code}\n})()`;
}

type HostMethod = keyof Pick<
  AsgrepConnector,
  | "search"
  | "semantic"
  | "chain"
  | "defs"
  | "callers"
  | "imports"
  | "indexStatus"
  | "indexRepo"
  | "catalogSearch"
  | "catalogDescribe"
>;

/**
 * Run model-generated JavaScript against the typed `asgrep` connector.
 *
 * Trust model (OpenCode-style): Code Mode is an orchestration pattern, not an OS
 * jail. The Pi package already runs with the installing user's privileges. Authority
 * is the explicit `asgrep.*` surface passed into the program — same idea as
 * OpenCode CodeMode exposing only host-supplied tools. We intentionally do **not**
 * use `node:vm` / isolates: same-realm `AsyncFunction` is faster and enough for
 * composition (`Promise.all`, filter, shape).
 */
export async function runCodemode(
  rawCode: string,
  asgrep: AsgrepConnector,
  options: {
    timeoutMs?: number;
    signal?: AbortSignal;
    stats?: () => DispatchStats;
  } = {},
): Promise<CodemodeRunResult> {
  const timeoutMs = Math.max(1, options.timeoutMs ?? DEFAULT_TIMEOUT_MS);
  const wall0 = Date.now();
  if (rawCode.length > MAX_CODE_CHARS) {
    return resultErr(`code exceeds ${MAX_CODE_CHARS} characters`, [], rawCode.slice(0, 200), wall0, options.stats);
  }
  if (options.signal?.aborted) {
    return resultErr("codemode aborted", [], rawCode.slice(0, 200), wall0, options.stats);
  }

  const logs: string[] = [];
  const formatLog = (value: unknown): string => {
    if (typeof value === "string") return value;
    try {
      return JSON.stringify(value);
    } catch {
      return String(value);
    }
  };
  const consoleApi = Object.freeze({
    log: (...args: unknown[]) => {
      logs.push(args.map(formatLog).join(" "));
    },
    info: (...args: unknown[]) => {
      logs.push(args.map(formatLog).join(" "));
    },
    warn: (...args: unknown[]) => {
      logs.push(args.map(formatLog).join(" "));
    },
    error: (...args: unknown[]) => {
      logs.push(args.map(formatLog).join(" "));
    },
    debug: (...args: unknown[]) => {
      logs.push(args.map(formatLog).join(" "));
    },
  });

  const methods: HostMethod[] = [
    "search",
    "semantic",
    "chain",
    "defs",
    "callers",
    "imports",
    "indexStatus",
    "indexRepo",
    "catalogSearch",
    "catalogDescribe",
  ];
  const api: Record<string, (args?: Record<string, unknown>) => Promise<unknown>> = Object.create(null);
  for (const method of methods) {
    const fn = asgrep[method].bind(asgrep) as (args?: Record<string, unknown>) => Promise<unknown>;
    api[method] = (args = {}) => fn(args);
  }
  Object.freeze(api);

  const code = normalizeCode(rawCode);
  // Same-realm AsyncFunction: faster than node:vm; no microtask-queue isolation issues.
  const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor as new (
    ...args: string[]
  ) => (...args: unknown[]) => Promise<unknown>;

  let run: (...args: unknown[]) => Promise<unknown>;
  try {
    run = new AsyncFunction(
      "asgrep",
      "console",
      `"use strict";\nreturn await (${code});`,
    );
  } catch (cause) {
    return resultErr(
      cause instanceof Error ? cause.message : String(cause),
      logs,
      code,
      wall0,
      options.stats,
    );
  }

  let timer: ReturnType<typeof setTimeout> | undefined;
  let onAbort: (() => void) | undefined;
  try {
    const races: Array<Promise<unknown>> = [
      Promise.resolve(run(api, consoleApi)),
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error(`codemode timeout after ${timeoutMs}ms`)), timeoutMs);
      }),
    ];
    if (options.signal) {
      races.push(new Promise<never>((_, reject) => {
        onAbort = () => reject(new Error("codemode aborted"));
        options.signal!.addEventListener("abort", onAbort, { once: true });
      }));
    }
    const value = await Promise.race(races);
    return resultOk(cloneOut(value), logs, code, wall0, options.stats);
  } catch (cause) {
    return resultErr(
      cause instanceof Error ? cause.message : String(cause),
      logs,
      code,
      wall0,
      options.stats,
    );
  } finally {
    if (timer) clearTimeout(timer);
    if (onAbort) options.signal?.removeEventListener("abort", onAbort);
  }
}

function resultOk(
  result: unknown,
  logs: string[],
  code: string,
  wall0: number,
  statsFn?: () => DispatchStats,
): CodemodeRunSuccess {
  const out: CodemodeRunSuccess = { ok: true, result, logs, code, wallMs: Date.now() - wall0 };
  const stats = statsFn?.();
  if (stats) out.stats = stats;
  return out;
}

function resultErr(
  error: string,
  logs: string[],
  code: string,
  wall0: number,
  statsFn?: () => DispatchStats,
): CodemodeRunFailure {
  const out: CodemodeRunFailure = { ok: false, result: null, logs, error, code, wallMs: Date.now() - wall0 };
  const stats = statsFn?.();
  if (stats) out.stats = stats;
  return out;
}

function cloneOut(value: unknown): unknown {
  if (value === undefined) return undefined;
  try {
    return structuredClone(value);
  } catch {
    try {
      return JSON.parse(JSON.stringify(value)) as unknown;
    } catch {
      return value;
    }
  }
}
