/**
 * Session-scoped native Code Mode sessions.
 *
 * Primary path: in-process NAPI (`CodeModeSession` inside Node) — same model as
 * MCP linking core. Zero CLI spawn.
 *
 * Fallback: sticky `codemode-serve` child only when the `.node` addon is missing
 * (unsupported host / incomplete install). Doctor reports that as degraded.
 */

import type { MachineEnvelope } from "../runtime.js";
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
    call(tool, args, options) {
      if (options?.signal?.aborted) return Promise.reject(abortError());
      const sync = session.callNow;
      if (inflight === 0 && !closed && sync && FAST_LOOKUP.has(tool)) {
        try {
          return Promise.resolve(asEnvelope(sync.call(session, tool, args ?? {}), tool));
        } catch (cause) {
          if (!isBusyError(cause)) return Promise.reject(cause);
        }
      }
      return enqueue(
        async () => asEnvelope(await session.call(tool, args, options?.signal), tool),
        options?.signal,
      );
    },
    batch(calls, options) {
      return enqueue(async () => {
        const response = await session.batch(calls, options?.signal);
        const result: BatchResult = {
          results: response.results,
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

  async acquire(root: string): Promise<StickyWorker | null> {
    if (this.#shutdownPromise) return null;
    const existing = this.#entries.get(root);
    if (existing) return existing.worker;

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
    const worker = await this.acquire(root);
    if (!worker) throw new Error("native Code Mode backend unavailable");
    return worker.call(tool, args, options);
  }

  async invalidate(root: string): Promise<void> {
    this.#generations.set(root, this.#generationFor(root) + 1);
    const starting = this.#starting.get(root);
    this.#starting.delete(root);
    const entry = this.#entries.get(root);
    this.#entries.delete(root);
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

    // 1) In-process NAPI (preferred — zero spawn).
    const binding = loadCodemodeNative();
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
        return worker;
      } catch {
        // Fall through to CLI sticky.
      }
    }

    // 2) CLI sticky fallback (degraded).
    if (!opts.binary) return null;
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
      this.#entries.set(root, { root, worker, generation: gen, backend: "cli" });
      this.#backend = "cli";
      return worker;
    } catch {
      return null;
    }
  }
}

/** Singleton for advanced hosts; tools registration uses a local pool. */
export const sharedNativePool = new NativeSessionPool();
