/**
 * Session-scoped sticky native workers — one warm `codemode-serve` per project root.
 *
 * Like pi-codex-conversion's SharedCodeModeRuntime / host session: pay spawn +
 * SQLite open once per root for the Pi session, then all Code Mode programs,
 * direct tools, and freshness checks reuse the same Searcher.
 *
 * Still a CLI child (packaging constraint — see docs/codemode.md). Eliminating
 * the process boundary entirely needs a NAPI addon; this is the pragmatic max
 * without changing the release contract.
 */

import type { MachineEnvelope } from "../runtime.js";
import { asEnvelope, type StickyWorker } from "./dispatch.js";
import { startStickyWorker, type StickyWorkerOptions } from "./worker.js";

export type SessionPoolOptions = {
  binary: string;
  env?: NodeJS.ProcessEnv;
  timeoutMs?: number;
};

export type StickyStarter = (options: StickyWorkerOptions) => Promise<StickyWorker>;

type Entry = {
  root: string;
  worker: StickyWorker;
  generation: number;
};

export class NativeSessionPool {
  #entries = new Map<string, Entry>();
  #starting = new Map<string, Promise<StickyWorker | null>>();
  #options: SessionPoolOptions | null = null;
  #generation = 0;
  #startFn: StickyStarter;

  constructor(startFn: StickyStarter = startStickyWorker) {
    this.#startFn = startFn;
  }

  configure(options: SessionPoolOptions): void {
    this.#options = options;
  }

  configured(): boolean {
    return this.#options !== null;
  }

  /**
   * Acquire (or start) the sticky worker for a root. Does not take a call-level
   * AbortSignal — killing the session worker on one cancelled tool would thrash
   * every other concurrent call.
   */
  async acquire(root: string): Promise<StickyWorker | null> {
    if (!this.#options) return null;
    const existing = this.#entries.get(root);
    if (existing) return existing.worker;

    const inFlight = this.#starting.get(root);
    if (inFlight) return inFlight;

    const start = this.#start(root);
    this.#starting.set(root, start);
    try {
      return await start;
    } finally {
      this.#starting.delete(root);
    }
  }

  async call(
    root: string,
    tool: string,
    args: Record<string, unknown> = {},
    options?: { signal?: AbortSignal },
  ): Promise<MachineEnvelope> {
    if (options?.signal?.aborted) throw new Error("native call aborted");
    const worker = await this.acquire(root);
    if (!worker) throw new Error("native session pool not configured");
    return asEnvelope(await worker.call(tool, args, options));
  }

  /** Drop the worker for a root (e..g. after fatal protocol error). */
  async invalidate(root: string): Promise<void> {
    this.#generation += 1;
    const entry = this.#entries.get(root);
    this.#entries.delete(root);
    if (entry) await entry.worker.end().catch(() => undefined);
  }

  async shutdown(): Promise<void> {
    const entries = [...this.#entries.values()];
    this.#entries.clear();
    this.#starting.clear();
    await Promise.all(entries.map((e) => e.worker.end().catch(() => undefined)));
  }

  async #start(root: string): Promise<StickyWorker | null> {
    const opts = this.#options;
    if (!opts) return null;
    const gen = this.#generation;
    try {
      const stickyOpts: StickyWorkerOptions = {
        binary: opts.binary,
        cwd: root,
      };
      if (opts.env) stickyOpts.env = opts.env;
      if (opts.timeoutMs !== undefined) stickyOpts.timeoutMs = opts.timeoutMs;
      const worker = await this.#startFn(stickyOpts);
      if (gen !== this.#generation) {
        await worker.end().catch(() => undefined);
        return null;
      }
      this.#entries.set(root, { root, worker, generation: gen });
      return worker;
    } catch {
      return null;
    }
  }
}

/** Singleton used by the Pi extension for the process lifetime. */
export const sharedNativePool = new NativeSessionPool();
