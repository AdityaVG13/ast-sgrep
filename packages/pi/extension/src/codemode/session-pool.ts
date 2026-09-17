/**
 * Session-scoped native Code Mode sessions.
 *
 * Primary path: in-process NAPI (`CodeModeSession` inside Node) — same model as
 * MCP linking core. Zero CLI spawn.
 *
 * Fallback: sticky `codemode-serve` child only when the `.node` addon is missing
 * (unsupported host / incomplete install). Doctor reports that as degraded.
 */

import type { MachineEnvelope } from "../runtime/runtime.js";
import { asEnvelope, type BatchResult, type StickyWorker } from "./dispatch.js";
import { loadCodemodeNative, type NativeSession } from "./native.js";
import { startStickyWorker, type StickyWorkerOptions } from "./worker.js";

export type SessionPoolOptions = {
  /** Required only for CLI sticky fallback. */
  binary?: string;
  env?: NodeJS.ProcessEnv;
  timeoutMs?: number;
  maxOutputBytes?: number;
  root?: string;
  indexPath?: string;
  useEmbed?: boolean;
  limit?: number;
};

export type StickyStarter = (options: StickyWorkerOptions) => Promise<StickyWorker>;

type Entry = {
  root: string;
  worker: StickyWorker;
  generation: number;
  backend: "napi" | "cli";
};

const abortError = (): Error => Object.assign(new Error("native call aborted"), { name: "AbortError" });

/** Bounded metadata and symbol lookups that may run on the JS thread. */
const FAST_LOOKUP = new Set([
  "defs",
  "callers",
  "imports",
  "index_status",
  "catalog_search",
  "catalog_describe",
  "find",
  "read",
]);

function isBusyError(cause: unknown): boolean {
  const message = cause instanceof Error ? cause.message : String(cause);
  return /session is busy/i.test(message);
}

/** Unique search must not run on the JS thread; cache hits may. */
export function isUncachedSearchError(cause: unknown): boolean {
  const message = cause instanceof Error ? cause.message : String(cause);
  return /use call\(\) for search/i.test(message);
}

/** NAPI can surface SQLite u64 counters as BigInt (writer_generation exceeds
 * 2^53). BigInt breaks JSON.stringify downstream (pi serializes result
 * details) — normalize to Number at the boundary. Precision past 2^53 is
 * display-only here; equality comparisons still hold since both sides
 * convert the same integer identically. */
function normalizeNativeValue(value: unknown): unknown {
  if (typeof value === "bigint") return Number(value);
  if (Array.isArray(value)) return value.map(normalizeNativeValue);
  if (value && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [key, entry] of Object.entries(value)) out[key] = normalizeNativeValue(entry);
    return out;
  }
  return value;
}

export function isClosedWorkerError(cause: unknown): boolean {
  const message = cause instanceof Error ? cause.message : String(cause);
  return /codemode-serve is closed|native session is closed/i.test(message);
}

function workerIsClosed(worker: StickyWorker): boolean {
  return worker.closed?.() === true;
}

function inProcessWorker(session: NativeSession): StickyWorker {
  let tail = Promise.resolve();
  let closed = false;
  let inflight = 0;

  const enqueue = <T>(operation: () => Promise<T>, signal?: AbortSignal): Promise<T> => {
    if (closed) return Promise.reject(new Error("native session is closed"));
    if (signal?.aborted) return Promise.reject(abortError());
    inflight += 1;
    const slot = tail.then(() => {
      if (signal?.aborted) throw abortError();
      return operation();
    });
    tail = slot.then(() => {
      inflight -= 1;
    }, () => {
      inflight -= 1;
    });
    if (!signal) return slot;
    return new Promise<T>((resolve, reject) => {
      const abort = () => reject(abortError());
      signal.addEventListener("abort", abort, { once: true });
      slot.then(resolve, reject).finally(() => signal.removeEventListener("abort", abort));
    });
  };

  return {
    closed: () => closed,
    call(tool, args, options) {
      if (options?.signal?.aborted) return Promise.reject(abortError());
      const sync = session.callNow;
      if (inflight === 0 && !closed && sync && (FAST_LOOKUP.has(tool) || tool === "search")) {
        try {
          const value = sync.call(session, tool, args ?? {});
          if (!(tool === "search" && value == null)) {
            return Promise.resolve(asEnvelope(normalizeNativeValue(value), tool));
          }
        } catch (cause) {
          if (tool === "search" && isUncachedSearchError(cause)) {
            // Older native addons threw on unique search.
          } else if (!isBusyError(cause)) {
            return Promise.reject(cause);
          }
        }
      }
      return enqueue(
        async () => asEnvelope(normalizeNativeValue(await session.call(tool, args, options?.signal)), tool),
        options?.signal,
      );
    },
    batch(calls, options) {
      return enqueue(async () => {
        const response = await session.batch(calls, options?.signal);
        const result: BatchResult = {
          results: normalizeNativeValue(response.results) as BatchResult["results"],
          all_ok: response.allOk,
          wall_ms: response.wallMs,
          mode: response.mode,
        };
        return result;
      }, options?.signal);
    },
    async end() {
      closed = true;
      await tail;
      // The NAPI session is released when this worker closure is dropped.
    },
  };
}

export class NativeSessionPool {
  #entries = new Map<string, Entry>();
  #starting = new Map<string, Promise<StickyWorker | null>>();
  #options: SessionPoolOptions | null = null;
  #generations = new Map<string, number>();
  #startFn: StickyStarter;
  #backend: "napi" | "cli" | "none" = "none";
  #shutdownPromise: Promise<void> | null = null;
  /** Real start failure per root — surfaced instead of a stale "closed" error. */
  #startFailures = new Map<string, { at: number; error: string }>();

  /** Crash-loop guard: a failing backend gets this long before respawn retries. */
  static #RESTART_BACKOFF_MS = 15_000;

  constructor(startFn: StickyStarter = startStickyWorker) {
    this.#startFn = startFn;
  }

  configure(options: SessionPoolOptions): void {
    this.#options = options;
  }

  configured(): boolean {
    return this.#options !== null || loadCodemodeNative() !== null;
  }

  /** Active backend after first successful acquire. */
  backend(): "napi" | "cli" | "none" {
    return this.#backend;
  }

  /** Why the last start attempt failed — for doctor/status and error fidelity. */
  lastStartError(root: string): string | undefined {
    return this.#startFailures.get(root)?.error;
  }

  async acquire(root: string): Promise<StickyWorker | null> {
    if (this.#shutdownPromise) return null;
    const existing = this.#entries.get(root);
    if (existing) {
      if (!workerIsClosed(existing.worker)) return existing.worker;
      await this.invalidate(root);
    }

    // A backend that just failed is crash-looping: skip the respawn for a
    // bounded window so callers fall back instead of paying a doomed spawn
    // per call. The window expires and recovery still happens automatically.
    const failure = this.#startFailures.get(root);
    if (failure && Date.now() - failure.at < NativeSessionPool.#RESTART_BACKOFF_MS) {
      return null;
    }

    const inFlight = this.#starting.get(root);
    if (inFlight) return inFlight;

    const start = this.#start(root);
    this.#starting.set(root, start);
    try {
      return await start;
    } finally {
      if (this.#starting.get(root) === start) this.#starting.delete(root);
    }
  }

  async call(
    root: string,
    tool: string,
    args: Record<string, unknown> = {},
    options?: { signal?: AbortSignal },
  ): Promise<MachineEnvelope> {
    if (options?.signal?.aborted) throw abortError();
    try {
      const worker = await this.acquire(root);
      if (!worker) {
        const startError = this.lastStartError(root);
        throw new Error(startError
          ? "codemode backend unavailable: " + startError
          : "native Code Mode backend unavailable");
      }
      return await worker.call(tool, args, options);
    } catch (cause) {
      if (options?.signal?.aborted || !isClosedWorkerError(cause)) throw cause;
      await this.invalidate(root);
      const retry = await this.acquire(root);
      if (!retry) {
        // Surface the real restart failure (spawn error / schema refusal), not
        // the stale "closed" message that hides it.
        const startError = this.lastStartError(root);
        if (startError) throw new Error("codemode backend unavailable: " + startError);
        throw cause;
      }
      return retry.call(tool, args, options);
    }
  }

  async invalidate(root: string): Promise<void> {
    this.#generations.set(root, this.#generationFor(root) + 1);
    const starting = this.#starting.get(root);
    this.#starting.delete(root);
    const entry = this.#entries.get(root);
    this.#entries.delete(root);
    this.#startFailures.delete(root);
    if (entry) await entry.worker.end().catch(() => undefined);
    if (starting) await starting.catch(() => null);
    if (this.#entries.size === 0) this.#backend = "none";
  }

  async shutdown(): Promise<void> {
    if (this.#shutdownPromise) return this.#shutdownPromise;
    const shutdown = this.#shutdownAll();
    this.#shutdownPromise = shutdown;
    try {
      await shutdown;
    } finally {
      if (this.#shutdownPromise === shutdown) this.#shutdownPromise = null;
    }
  }

  async #shutdownAll(): Promise<void> {
    const roots = new Set([...this.#entries.keys(), ...this.#starting.keys()]);
    for (const root of roots) this.#generations.set(root, this.#generationFor(root) + 1);
    const entries = [...this.#entries.values()];
    const starting = [...this.#starting.values()];
    this.#entries.clear();
    this.#starting.clear();
    this.#backend = "none";
    await Promise.all([
      ...entries.map((e) => e.worker.end().catch(() => undefined)),
      ...starting.map((start) => start.catch(() => null)),
    ]);
  }

  #generationFor(root: string): number {
    return this.#generations.get(root) ?? 0;
  }

  async #start(root: string): Promise<StickyWorker | null> {
    const gen = this.#generationFor(root);
    const opts = this.#options ?? {};
    const fail = (error: unknown): null => {
      this.#startFailures.set(root, {
        at: Date.now(),
        error: error instanceof Error ? error.message : String(error),
      });
      return null;
    };

    // 1) In-process NAPI (preferred — zero spawn).
    const binding = loadCodemodeNative();
    let napiError: unknown = null;
    if (binding) {
      try {
        const config: {
          root: string;
          indexPath?: string;
          limit?: number;
          useEmbed?: boolean;
        } = { root };
        if (opts.indexPath) config.indexPath = opts.indexPath;
        if (opts.limit !== undefined) config.limit = opts.limit;
        if (opts.useEmbed !== undefined) config.useEmbed = opts.useEmbed;
        const session = new binding.Session(config);
        const worker = inProcessWorker(session);
        if (gen !== this.#generationFor(root)) {
          await worker.end().catch(() => undefined);
          return null;
        }
        this.#entries.set(root, { root, worker, generation: gen, backend: "napi" });
        this.#backend = "napi";
        this.#startFailures.delete(root);
        return worker;
      } catch (cause) {
        // Fall through to CLI sticky, but keep the real error for fidelity.
        napiError = cause;
      }
    }

    // 2) CLI sticky fallback (degraded).
    if (!opts.binary) {
      return fail(napiError ?? new Error("native Code Mode backend unavailable (no addon, no binary)"));
    }
    try {
      const stickyOpts: StickyWorkerOptions = {
        binary: opts.binary,
        cwd: root,
      };
      if (opts.env) stickyOpts.env = opts.env;
      if (opts.timeoutMs !== undefined) stickyOpts.timeoutMs = opts.timeoutMs;
      if (opts.maxOutputBytes !== undefined) stickyOpts.maxOutputBytes = opts.maxOutputBytes;
      const worker = await this.#startFn(stickyOpts);
      if (gen !== this.#generationFor(root)) {
        await worker.end().catch(() => undefined);
        return null;
      }
      if (workerIsClosed(worker)) {
        await worker.end().catch(() => undefined);
        return fail(new Error("codemode-serve exited during startup"));
      }
      this.#entries.set(root, { root, worker, generation: gen, backend: "cli" });
      this.#backend = "cli";
      this.#startFailures.delete(root);
      return worker;
    } catch (cause) {
      return fail(cause);
    }
  }
}

/** Singleton for advanced hosts; tools registration uses a local pool. */
export const sharedNativePool = new NativeSessionPool();
