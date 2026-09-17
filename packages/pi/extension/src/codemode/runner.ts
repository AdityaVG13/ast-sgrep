import { Worker } from "node:worker_threads";
import type { AsgrepConnector } from "./connector.js";
import type { DispatchStats } from "./dispatch.js";
import { coerceHostArgs, normalizeCode, packGuestCall, resolveHostMethod, timeoutHint, unknownMethodError } from "./guest-api.js";
import { CODEMODE_HOST_METHODS, type CodemodeHostMethod } from "./types.js";

export { normalizeCode };

/** Closed sum: success|failure — ok:true with error (or ok:false without) is unrepresentable. */
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
const MAX_BRIDGE_CALLS = 256;
const MAX_BRIDGE_REQUEST_CHARS = 64_000;
const MAX_ERROR_CHARS = 8_192;
const MAX_LOG_LINES = 100;
const MAX_LOG_CHARS = 64_000;
const MAX_RESULT_JSON_CHARS = 1_000_000;
const MAX_TIMER_MS = 2_147_483_647;
/** Hard ceiling on guest heap; an OOM guest kills its isolate, never the host. */
const WORKER_MAX_OLD_SPACE_MB = 512;
const WORKER_MAX_YOUNG_SPACE_MB = 64;
/** Parent-side RSS guard: a run may grow process RSS by this multiple of the heap cap. */
const MEMORY_SLACK = 1.5;
const MEMORY_POLL_MS = 50;
/** Grace for in-flight host calls after the guest reports done. */
const DRAIN_PENDING_MS = 250;

const WORKER_URL = new URL("./guest-worker.mjs", import.meta.url);
const ABORT_MESSAGE = "codemode timed out or aborted: pass timeoutMs to allow longer runs";

type HostMethod = CodemodeHostMethod;

type HostFn = (
  args: Record<string, unknown>,
  options?: { signal?: AbortSignal },
) => Promise<unknown>;

type WorkerMessage =
  | { op: "call-batch"; runId?: number; calls: Array<{ id: string; method: string; payload: string }> }
  | { op: "log"; runId?: number; line: string }
  | { op: "result"; runId?: number; serialized?: string }
  | { op: "error"; runId?: number; error: string }
  | { op: "ready"; runId?: number };

// ---------------------------------------------------------------------------
// Standby worker lifecycle (supernova shape): at most one pristine isolate is
// kept warm; claiming it spawns a replacement immediately. A used worker is
// NEVER pooled — every program gets a virgin heap.
// ---------------------------------------------------------------------------

type WorkerHandle = {
  worker: Worker;
  ready: Promise<void>;
  dead: boolean;
};

let idle: WorkerHandle | null = null;
let runSeq = 0;
const activeWorkers = new Set<Worker>();

function spawnWorker(): WorkerHandle {
  // Inline eval'd bootstrap: imports the real guest-worker file. execArgv is
  // emptied so --import tsx / --test flags cannot leak in and crash the isolate.
  const worker = new Worker("import(" + JSON.stringify(WORKER_URL.href) + ")", {
    eval: true,
    execArgv: [],
    resourceLimits: {
      maxOldGenerationSizeMb: WORKER_MAX_OLD_SPACE_MB,
      maxYoungGenerationSizeMb: WORKER_MAX_YOUNG_SPACE_MB,
    },
  });
  const handle: WorkerHandle = { worker, ready: Promise.resolve(), dead: false };
  worker.on("error", () => { handle.dead = true; });
  worker.on("exit", () => {
    handle.dead = true;
    if (idle === handle) idle = null;
  });
  handle.ready = new Promise<void>((resolve, reject) => {
    const cleanup = () => {
      worker.off("message", onMessage);
      worker.off("error", onFail);
      worker.off("exit", onFail);
    };
    const onMessage = (msg: WorkerMessage) => {
      if (msg?.op !== "ready") return;
      cleanup();
      resolve();
    };
    const onFail = (err: unknown) => {
      cleanup();
      reject(err instanceof Error ? err : new Error("codemode worker exited before ready (code " + err + ")"));
    };
    worker.on("message", onMessage);
    worker.on("error", onFail);
    worker.on("exit", onFail);
  });
  handle.ready.catch(() => undefined);
  return handle;
}

function killWorker(handle: WorkerHandle | null | undefined): Promise<void> | undefined {
  if (!handle) return undefined;
  if (idle === handle) idle = null;
  handle.dead = true;
  return handle.worker.terminate().then(() => undefined, () => undefined);
}

function acquireWorker(): WorkerHandle {
  const candidate = idle;
  idle = null;
  const handle = candidate && !candidate.dead ? candidate : spawnWorker();
  if (candidate && candidate.dead) void killWorker(candidate);
  handle.worker.ref?.();
  // Pipeline the replacement while this run executes.
  warmCodemodeSandbox().catch(() => undefined);
  return handle;
}

/** Spawn the warm standby isolate (session_start / pre-call). */
export function warmCodemodeSandbox(): Promise<void> {
  if (idle && !idle.dead) return idle.ready;
  const handle = spawnWorker();
  idle = handle;
  void handle.ready.then(() => {
    if (idle === handle) handle.worker.unref?.();
  }, () => undefined);
  return handle.ready;
}

/** Drop the standby isolate and any in-flight runs (tests / session shutdown). */
export async function resetCodemodeSandboxForTests(): Promise<void> {
  await killWorker(idle);
  await Promise.all([...activeWorkers].map((worker) => worker.terminate().catch(() => undefined)));
}

// ---------------------------------------------------------------------------
// Admission funnel: validate everything before any worker work happens.
// ---------------------------------------------------------------------------

function admitTimeout(requested: number | undefined): { timeoutMs: number } | { error: string } {
  if (requested === undefined) return { timeoutMs: DEFAULT_TIMEOUT_MS };
  if (!Number.isFinite(requested) || requested <= 0) return { error: "timeoutMs must be a positive finite number" };
  return { timeoutMs: Math.min(MAX_TIMER_MS, Math.max(1, Math.trunc(requested))) };
}

function admitRun(code: string, timeoutMs: number | undefined, signal?: AbortSignal): { code: string; timeoutMs: number } | { error: string } {
  if (code.length > MAX_CODE_CHARS) return { error: "code exceeds " + MAX_CODE_CHARS + " characters; split into multiple asgrep calls" };
  if (signal?.aborted) return { error: "codemode aborted" };
  const timeout = admitTimeout(timeoutMs);
  if ("error" in timeout) return timeout;
  return { code: normalizeCode(code), timeoutMs: timeout.timeoutMs };
}

// ---------------------------------------------------------------------------
// GuestRun: one run = one worker = one class instance. Explicit lifecycle
// states replace the closure flag soup: finished → terminal, accepting →
// messages still honored, completing → draining, aborting → user/timeout path.
// ---------------------------------------------------------------------------

class GuestRun {
  private finished = false;
  private accepting = true;
  private completing = false;
  private aborting = false;
  private handle: WorkerHandle | undefined;
  private readonly pending = new Set<Promise<void>>();
  private readonly logs: string[] = [];
  private logChars = 0;
  private callCount = 0;
  private lastMethod: string | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private memTimer: ReturnType<typeof setInterval> | undefined;
  private hostError: string | undefined;
  private readonly wall0 = performance.now();
  private readonly runController = new AbortController();
  private readonly rssStart: number;
  private readonly rssLimit: number;
  private resolve!: (outcome: CodemodeRunResult) => void;

  constructor(
    private readonly code: string,
    private readonly timeoutMs: number,
    private readonly hostMethods: Record<HostMethod, HostFn>,
    private readonly signal: AbortSignal | undefined,
    private readonly statsFn: (() => DispatchStats) | undefined,
    private readonly runId: number,
  ) {
    this.rssStart = process.memoryUsage().rss;
    this.rssLimit = this.rssStart + WORKER_MAX_OLD_SPACE_MB * MEMORY_SLACK * 1_048_576;
  }

  private wall(): number {
    return performance.now() - this.wall0;
  }

  private ok(result: unknown): CodemodeRunSuccess {
    const out: CodemodeRunSuccess = { ok: true, result, logs: this.logs, code: this.code, wallMs: this.wall() };
    const stats = this.statsFn?.();
    if (stats) out.stats = stats;
    return out;
  }

  private err(error: string): CodemodeRunFailure {
    const out: CodemodeRunFailure = {
      ok: false,
      result: null,
      logs: this.logs,
      error: timeoutHint(error).slice(0, MAX_ERROR_CHARS),
      code: this.code,
      wallMs: this.wall(),
    };
    const stats = this.statsFn?.();
    if (stats) out.stats = stats;
    return out;
  }

  private cleanup(): void {
    clearTimeout(this.timer);
    clearInterval(this.memTimer);
    this.signal?.removeEventListener("abort", this.onAbort);
    if (this.handle) {
      this.handle.worker.off("message", this.onMessage);
      this.handle.worker.off("error", this.onError);
      this.handle.worker.off("exit", this.onExit);
    }
  }

  private finish(outcome: CodemodeRunResult): void {
    if (this.finished) return;
    this.finished = true;
    this.accepting = false;
    this.cleanup();
    this.runController.abort();
    if (this.handle) {
      activeWorkers.delete(this.handle.worker);
      void killWorker(this.handle);
    }
    this.resolve(outcome);
  }

  private fail(message: string): void {
    this.finish(this.err(message));
  }

  private abort(): void {
    if (this.finished || this.aborting) return;
    this.aborting = true;
    this.fail(ABORT_MESSAGE);
    this.aborting = false;
  }

  private onAbort = (): void => this.abort();

  private hostLog(line: string): void {
    if (this.logs.length >= MAX_LOG_LINES || this.logChars >= MAX_LOG_CHARS) return;
    const remaining = MAX_LOG_CHARS - this.logChars;
    const bounded = line.length <= remaining ? line : line.slice(0, Math.max(0, remaining - 1)) + "…";
    this.logs.push(bounded);
    this.logChars += bounded.length;
  }

  private async hostCall(method: string, payload: string): Promise<string> {
    try {
      if (this.runController.signal.aborted) {
        throw Object.assign(new Error("codemode aborted"), { name: "AbortError" });
      }
      if (this.callCount >= MAX_BRIDGE_CALLS) {
        throw new Error("codemode exceeds " + MAX_BRIDGE_CALLS + " host calls");
      }
      this.callCount += 1;
      this.lastMethod = method;
      if (payload.length > MAX_BRIDGE_REQUEST_CHARS) {
        throw new Error("codemode call arguments exceed " + MAX_BRIDGE_REQUEST_CHARS + " characters");
      }
      const resolved = resolveHostMethod(method);
      if (!resolved || !Object.hasOwn(this.hostMethods, resolved)) {
        throw new Error(unknownMethodError(method));
      }
      const parsed = JSON.parse(payload) as Record<string, unknown>;
      const packed = Array.isArray(parsed.__guestArgs)
        ? packGuestCall(resolved, parsed.__guestArgs)
        : parsed;
      const input = coerceHostArgs(resolved, packed);
      const invokeHost = this.hostMethods[resolved];
      if (!invokeHost) throw new Error(unknownMethodError(method));
      const value = await invokeHost(input, { signal: this.runController.signal });
      return JSON.stringify({ ok: true, value }, jsonSafe);
    } catch (cause) {
      return JSON.stringify({ ok: false, error: safeErrorMessage(cause).slice(0, MAX_ERROR_CHARS) });
    }
  }

  private respond(id: string, body: string): void {
    if (!this.handle || this.finished) return;
    try {
      this.handle.worker.postMessage({ op: "call-result", runId: this.runId, id, body });
    } catch {
      // Worker already terminated; the run is settled.
    }
  }

  private onCallBatch(calls: Array<{ id: string; method: string; payload: string }>): void {
    for (const call of calls) {
      const work = this.hostCall(call.method, call.payload)
        .then((body) => this.respond(call.id, body))
        .catch(() => this.respond(call.id, JSON.stringify({ ok: false, error: "codemode call failed" })));
      this.pending.add(work);
      void work.finally(() => this.pending.delete(work));
    }
  }

  private onResult(msg: { serialized?: string }): void {
    try {
      const serialized = msg.serialized;
      const result = serialized === undefined ? undefined : (JSON.parse(serialized) as unknown);
      void this.complete(this.ok(result));
    } catch (cause) {
      this.fail("codemode result decode failed: " + safeErrorMessage(cause));
    }
  }

  private onMessage = (msg: WorkerMessage): void => {
    if (this.finished || !this.accepting || !msg || typeof msg !== "object") return;
    if (typeof msg.runId === "number" && msg.runId !== this.runId) return;
    if (msg.op === "log") {
      this.hostLog(msg.line);
      return;
    }
    if (msg.op === "call-batch") {
      this.onCallBatch(Array.isArray(msg.calls) ? msg.calls : []);
      return;
    }
    if (msg.op === "result") {
      this.onResult(msg);
      return;
    }
    if (msg.op === "error") {
      this.fail(msg.error);
    }
  };

  private onError = (cause: Error): void => {
    void this.complete(this.err("codemode worker error: " + cause.message));
  };

  private onExit = (exitCode: number): void => {
    void this.complete(this.err("codemode worker exited code=" + exitCode));
  };

  /** Give in-flight host calls a bounded settle window, then surface leaks. */
  private async drainPending(outcome: CodemodeRunResult): Promise<void> {
    if (this.pending.size === 0) return;
    await Promise.race([
      Promise.allSettled(this.pending),
      new Promise((resolve) => setTimeout(resolve, DRAIN_PENDING_MS)),
    ]);
    if (this.pending.size && outcome.ok) {
      this.hostError ??= "program completed with a host call still running";
    }
  }

  private async complete(outcome: CodemodeRunResult): Promise<void> {
    if (this.finished || this.completing) return;
    this.completing = true;
    this.accepting = false;
    await this.drainPending(outcome);
    if (this.finished) return;
    this.finish(outcome.ok && this.hostError ? this.err(this.hostError) : outcome);
  }

  private memoryLimitError(): string {
    const now = process.memoryUsage().rss;
    const deltaMb = Math.max(0, (now - this.rssStart) / 1_048_576);
    const where = this.lastMethod ? " during " + this.lastMethod : "";
    return (
      "codemode exceeded memory limit: process RSS +" + deltaMb.toFixed(1) + "MB in " +
      Math.round(this.wall()) + "ms" + where + " (" + this.callCount + " host calls); " +
      "worker heap cap is " + WORKER_MAX_OLD_SPACE_MB + "MB — split the program or stream less data"
    );
  }

  private async boot(): Promise<void> {
    try {
      this.handle = acquireWorker();
      activeWorkers.add(this.handle.worker);
      await this.handle.ready;
      if (this.finished || this.signal?.aborted) return this.abort();
      this.handle.worker.on("message", this.onMessage);
      this.handle.worker.on("error", this.onError);
      this.handle.worker.on("exit", this.onExit);
      if (this.wall() >= this.timeoutMs) return this.abort();
      this.handle.worker.postMessage({ op: "run", runId: this.runId, code: this.code, timeoutMs: this.timeoutMs });
    } catch (cause) {
      if (this.finished) return;
      this.fail("codemode worker unavailable: " + safeErrorMessage(cause));
    }
  }

  start(): Promise<CodemodeRunResult> {
    return new Promise<CodemodeRunResult>((resolve) => {
      this.resolve = resolve;
      this.timer = setTimeout(() => this.abort(), this.timeoutMs);
      this.timer.unref?.();
      this.memTimer = setInterval(() => {
        if (process.memoryUsage().rss <= this.rssLimit) return;
        this.fail(this.memoryLimitError());
      }, MEMORY_POLL_MS);
      this.memTimer.unref?.();
      this.signal?.addEventListener("abort", this.onAbort, { once: true });
      void this.boot();
    });
  }
}

function bindHostMethods(asgrep: AsgrepConnector): Record<HostMethod, HostFn> {
  const wrap = (
    fn: (args: never, options?: { signal?: AbortSignal }) => Promise<unknown>,
  ): HostFn => (args, options) => fn(args as never, options);
  return {
    search: wrap(asgrep.search.bind(asgrep)),
    find: wrap(asgrep.find.bind(asgrep)),
    read: wrap(asgrep.read.bind(asgrep)),
    edit: wrap(asgrep.edit.bind(asgrep)),
    semantic: wrap(asgrep.semantic.bind(asgrep)),
    chain: wrap(asgrep.chain.bind(asgrep)),
    defs: wrap(asgrep.defs.bind(asgrep)),
    callers: wrap(asgrep.callers.bind(asgrep)),
    imports: wrap(asgrep.imports.bind(asgrep)),
    indexStatus: (_args, options) => asgrep.indexStatus(options),
    indexRepo: wrap(asgrep.indexRepo.bind(asgrep)),
    doctor: (_args, options) => asgrep.doctor(options),
    catalogSearch: wrap(asgrep.catalogSearch.bind(asgrep)),
    catalogDescribe: wrap(asgrep.catalogDescribe.bind(asgrep)),
  };
}

/** NAPI u64/i64 fields cross as BigInt; keep safe ints numeric, exact-string the rest. */
const jsonSafe = (_key: string, item: unknown): unknown =>
  typeof item === "bigint"
    ? item >= -9007199254740991n && item <= 9007199254740991n
      ? Number(item)
      : item.toString()
    : item;

/**
 * Run model-generated JavaScript against the typed asgrep connector.
 *
 * Execution happens in a single-use worker_threads isolate: the guest gets a
 * node:vm context inside the worker; asgrep/console are built there; the only
 * host channel is a JSON postMessage bridge carrying runId envelopes. Timeout
 * and abort call worker.terminate(), which is the only mechanism that actually
 * stops a detached guest microtask or a runaway heap.
 */
export function runCodemode(
  rawCode: string,
  asgrep: AsgrepConnector,
  options: {
    timeoutMs?: number;
    signal?: AbortSignal;
    stats?: () => DispatchStats;
  } = {},
): Promise<CodemodeRunResult> {
  const admitted = admitRun(rawCode, options.timeoutMs, options.signal);
  const wall0 = performance.now();
  if ("error" in admitted) {
    const out: CodemodeRunFailure = {
      ok: false, result: null, logs: [], error: admitted.error,
      code: rawCode.slice(0, 200), wallMs: performance.now() - wall0,
    };
    const stats = options.stats?.();
    if (stats) out.stats = stats;
    return Promise.resolve(out);
  }
  return new GuestRun(
    admitted.code,
    admitted.timeoutMs,
    bindHostMethods(asgrep),
    options.signal,
    options.stats,
    ++runSeq,
  ).start();
}

function safeErrorMessage(cause: unknown): string {
  try {
    return String(cause instanceof Error ? cause.message : cause);
  } catch {
    return "codemode call failed";
  }
}
