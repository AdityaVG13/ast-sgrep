import vm from "node:vm";
import type { AsgrepConnector } from "./connector.js";

export type CodemodeRunResult = {
  ok: boolean;
  result: unknown;
  logs: string[];
  error?: string;
  code: string;
};

const DEFAULT_TIMEOUT_MS = 30_000;
const MAX_CODE_CHARS = 32_000;

/** Strip markdown fences and normalize to an async IIFE expression. */
export function normalizeCode(raw: string): string {
  let code = raw.trim();
  if (code.startsWith("```")) {
    code = code.replace(/^```(?:javascript|js|typescript|ts)?\s*/i, "").replace(/\s*```$/, "").trim();
  }
  // Already an async arrow / function expression the model wrote as the tool body.
  if (/^async\s*\(/.test(code) || /^async\s+function\b/.test(code)) {
    return `(${code.endsWith(";") ? code.slice(0, -1) : code})()`;
  }
  // Bare statements / expression body — wrap and return last expression when possible.
  return `(async () => {\n${code}\n})()`;
}

/**
 * Run model-generated JavaScript with only `asgrep` + safe builtins.
 *
 * This is a capability sandbox (no require/process/fetch), not an OS security
 * boundary — same trust model as the Pi package itself.
 */
export async function runCodemode(
  rawCode: string,
  asgrep: AsgrepConnector,
  options: { timeoutMs?: number } = {},
): Promise<CodemodeRunResult> {
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  if (rawCode.length > MAX_CODE_CHARS) {
    return {
      ok: false,
      result: null,
      logs: [],
      error: `code exceeds ${MAX_CODE_CHARS} characters`,
      code: rawCode.slice(0, 200),
    };
  }

  const logs: string[] = [];
  const pushLog = (...args: unknown[]) => {
    logs.push(args.map((a) => (typeof a === "string" ? a : safeJson(a))).join(" "));
  };

  const context = vm.createContext({
    asgrep,
    console: {
      log: pushLog,
      info: pushLog,
      warn: pushLog,
      error: pushLog,
      debug: pushLog,
    },
    Promise,
    JSON,
    Array,
    Object,
    Map,
    Set,
    Math,
    Number,
    String,
    Boolean,
    Date,
    RegExp,
    Error,
    TypeError,
    RangeError,
    parseInt,
    parseFloat,
    isNaN,
    isFinite,
    undefined,
  });

  const code = normalizeCode(rawCode);
  let script: vm.Script;
  try {
    script = new vm.Script(code, { filename: "asgrep-codemode.js" });
  } catch (cause) {
    return {
      ok: false,
      result: null,
      logs,
      error: cause instanceof Error ? cause.message : String(cause),
      code,
    };
  }

  try {
    const produced = script.runInContext(context, { timeout: timeoutMs, displayErrors: true });
    const result = await Promise.race([
      Promise.resolve(produced),
      new Promise<never>((_, reject) => {
        setTimeout(() => reject(new Error(`codemode timeout after ${timeoutMs}ms`)), timeoutMs);
      }),
    ]);
    // Structured-clone out of the vm context so host assertions see plain objects.
    return { ok: true, result: cloneOut(result), logs, code };
  } catch (cause) {
    return {
      ok: false,
      result: null,
      logs,
      error: cause instanceof Error ? cause.message : String(cause),
      code,
    };
  }
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
