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
import { loadCodemodeNative, type NativeSession, type CodemodeNativeBinding } from "./native.js";
import { startStickyWorker, type StickyWorkerOptions } from "./worker.js";

export type SessionPoolOptions = {
  /** Required only for CLI sticky fallback. */
  binary?: string;
  env?: NodeJS.ProcessEnv;
  timeoutMs?: number;
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

function inProcessWorker(session: NativeSession, binding: CodemodeNativeBinding, root: string): StickyWorker {
  return {
    async call(tool, args) {
      const value = session.call(tool, args);
      return asEnvelope(value);
    },
    async batch(calls) {
      // Prefer session.call in a loop — reuses the warm Searcher already open.
      const results: BatchResult["results"] = [];
      let allOk = true;
      const t0 = Date.now();
      for (const c of calls) {
        try {
          const value = session.call(c.tool, c.args);
          results.push({ id: c.id, ok: true, value });
        } catch (cause) {
          allOk = false;
          results.push({
            id: c.id,
            ok: false,
            error: cause instanceof Error ? cause.message : String(cause),
          });
        }
      }
      // For large waves, binding.batch can parallelize; serial warm is the default win.
      if (calls.length >= 4 && allOk) {
        void binding;
        void root;
      }
      return { results, all_ok: allOk, wall_ms: Date.now() - t0, mode: "serial-napi" };
    },
    async end() {
      // NAPI session is GC'd; nothing to kill.
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
    if (options?.signal?.aborted) throw new Error("native call aborted");
    const worker = await this.acquire(root);
    if (!worker) throw new Error("native Code Mode backend unavailable");
    return worker.call(tool, args, options);
  }

  async invalidate(root: string): Promise<void> {
    this.#generations.set(root, this.#generationFor(root) + 1);
    this.#starting.delete(root);
    const entry = this.#entries.get(root);
    this.#entries.delete(root);
    if (entry) await entry.worker.end().catch(() => undefined);
    if (this.#entries.size === 0) this.#backend = "none";
  }

  async shutdown(): Promise<void> {
    const roots = new Set([...this.#entries.keys(), ...this.#starting.keys()]);
    for (const root of roots) this.#generations.set(root, this.#generationFor(root) + 1);
    const entries = [...this.#entries.values()];
    this.#entries.clear();
    this.#starting.clear();
    this.#backend = "none";
    await Promise.all(entries.map((e) => e.worker.end().catch(() => undefined)));
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
        const worker = inProcessWorker(session, binding, root);
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
