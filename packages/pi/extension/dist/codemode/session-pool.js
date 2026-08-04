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
import { asEnvelope } from "./dispatch.js";
import { startStickyWorker } from "./worker.js";
export class NativeSessionPool {
    #entries = new Map();
    #starting = new Map();
    #options = null;
    #generation = 0;
    #startFn;
    constructor(startFn = startStickyWorker) {
        this.#startFn = startFn;
    }
    configure(options) {
        this.#options = options;
    }
    configured() {
        return this.#options !== null;
    }
    /**
     * Acquire (or start) the sticky worker for a root. Does not take a call-level
     * AbortSignal — killing the session worker on one cancelled tool would thrash
     * every other concurrent call.
     */
    async acquire(root) {
        if (!this.#options)
            return null;
        const existing = this.#entries.get(root);
        if (existing)
            return existing.worker;
        const inFlight = this.#starting.get(root);
        if (inFlight)
            return inFlight;
        const start = this.#start(root);
        this.#starting.set(root, start);
        try {
            return await start;
        }
        finally {
            this.#starting.delete(root);
        }
    }
    async call(root, tool, args = {}, options) {
        if (options?.signal?.aborted)
            throw new Error("native call aborted");
        const worker = await this.acquire(root);
        if (!worker)
            throw new Error("native session pool not configured");
        return asEnvelope(await worker.call(tool, args, options));
    }
    /** Drop the worker for a root (e..g. after fatal protocol error). */
    async invalidate(root) {
        this.#generation += 1;
        const entry = this.#entries.get(root);
        this.#entries.delete(root);
        if (entry)
            await entry.worker.end().catch(() => undefined);
    }
    async shutdown() {
        const entries = [...this.#entries.values()];
        this.#entries.clear();
        this.#starting.clear();
        await Promise.all(entries.map((e) => e.worker.end().catch(() => undefined)));
    }
    async #start(root) {
        const opts = this.#options;
        if (!opts)
            return null;
        const gen = this.#generation;
        try {
            const stickyOpts = {
                binary: opts.binary,
                cwd: root,
            };
            if (opts.env)
                stickyOpts.env = opts.env;
            if (opts.timeoutMs !== undefined)
                stickyOpts.timeoutMs = opts.timeoutMs;
            const worker = await this.#startFn(stickyOpts);
            if (gen !== this.#generation) {
                await worker.end().catch(() => undefined);
                return null;
            }
            this.#entries.set(root, { root, worker, generation: gen });
            return worker;
        }
        catch {
            return null;
        }
    }
}
/** Singleton used by the Pi extension for the process lifetime. */
export const sharedNativePool = new NativeSessionPool();
