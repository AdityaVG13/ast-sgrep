import vm from "node:vm";
import type { AsgrepConnector } from "./connector.js";
import type { DispatchStats } from "./dispatch.js";

export type CodemodeRunResult = {
  ok: boolean;
  result: unknown;
  logs: string[];
  error?: string;
  code: string;
  stats?: DispatchStats;
  wallMs: number;
};

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

/**
 * Run model-generated JavaScript with only `asgrep` + safe builtins.
 *
 * Uses the shared microtask queue so host Promises from `asgrep.*` resolve under
 * `Promise.all`. Do not enable `microtaskMode: 'afterEvaluate'` — that isolates
 * queues and breaks cross-context await.
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
  const hostMethods = {
    search: asgrep.search,
    semantic: asgrep.semantic,
    chain: asgrep.chain,
    defs: asgrep.defs,
    callers: asgrep.callers,
    imports: asgrep.imports,
    indexStatus: asgrep.indexStatus,
    indexRepo: asgrep.indexRepo,
    catalogSearch: asgrep.catalogSearch,
    catalogDescribe: asgrep.catalogDescribe,
  };
  type HostMethod = keyof typeof hostMethods;

  // Never expose host-realm functions or objects to model code. A direct host
  // function lets `fn.constructor("return process")()` escape `node:vm`.
  const bridge = async (method: string, payload: string): Promise<string> => {
    try {
      if (!Object.hasOwn(hostMethods, method)) throw new Error(`unknown asgrep method: ${method}`);
      const input = JSON.parse(payload) as Record<string, unknown>;
      const value = await (hostMethods[method as HostMethod] as (arg: Record<string, unknown>) => Promise<unknown>)(input);
      return JSON.stringify({ ok: true, value });
    } catch (cause) {
      return JSON.stringify({
        ok: false,
        error: cause instanceof Error ? cause.message : String(cause),
      });
    }
  };
  const logBridge = (line: string) => logs.push(line);
  Object.setPrototypeOf(bridge, null);
  Object.setPrototypeOf(logBridge, null);
  Object.freeze(bridge);
  Object.freeze(logBridge);

  const globals = Object.create(null) as Record<string, unknown>;
  globals.__asgrepBridge = bridge;
  globals.__asgrepLog = logBridge;
  const context = vm.createContext(globals, {
    codeGeneration: { strings: false, wasm: false },
  });
  new vm.Script(SANDBOX_BOOTSTRAP, { filename: "asgrep-codemode-bootstrap.js" }).runInContext(context, {
    timeout: 1_000,
  });

  const code = normalizeCode(rawCode);
  let script: vm.Script;
  try {
    script = new vm.Script(code, { filename: "asgrep-codemode.js" });
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
    const produced = script.runInContext(context, {
      displayErrors: true,
      timeout: timeoutMs,
    });
    const races: Array<Promise<unknown>> = [
      Promise.resolve(produced),
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

const SANDBOX_BOOTSTRAP = `
{
  const hostCall = globalThis.__asgrepBridge;
  const hostLog = globalThis.__asgrepLog;
  delete globalThis.__asgrepBridge;
  delete globalThis.__asgrepLog;

  const invoke = async (method, args = {}) => {
    const response = JSON.parse(await hostCall(method, JSON.stringify(args)));
    if (!response.ok) throw new Error(response.error || \`asgrep.\${method} failed\`);
    return response.value;
  };
  const api = Object.create(null);
  for (const method of [
    "search", "semantic", "chain", "defs", "callers", "imports",
    "indexStatus", "indexRepo", "catalogSearch", "catalogDescribe",
  ]) {
    Object.defineProperty(api, method, {
      enumerable: true,
      value: (args = {}) => invoke(method, args),
    });
  }
  Object.freeze(api);

  const formatLog = (value) => {
    if (typeof value === "string") return value;
    try { return JSON.stringify(value); } catch { return String(value); }
  };
  const consoleApi = Object.create(null);
  for (const level of ["log", "info", "warn", "error", "debug"]) {
    Object.defineProperty(consoleApi, level, {
      enumerable: true,
      value: (...args) => hostLog(args.map(formatLog).join(" ")),
    });
  }
  Object.freeze(consoleApi);

  Object.defineProperty(globalThis, "asgrep", { value: api, configurable: false, writable: false });
  Object.defineProperty(globalThis, "console", { value: consoleApi, configurable: false, writable: false });
}
`;

function resultOk(
  result: unknown,
  logs: string[],
  code: string,
  wall0: number,
  statsFn?: () => DispatchStats,
): CodemodeRunResult {
  const out: CodemodeRunResult = { ok: true, result, logs, code, wallMs: Date.now() - wall0 };
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
): CodemodeRunResult {
  const out: CodemodeRunResult = { ok: false, result: null, logs, error, code, wallMs: Date.now() - wall0 };
  const stats = statsFn?.();
  if (stats) out.stats = stats;
  return out;
}

function safeJson(value: unknown): string {
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
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
