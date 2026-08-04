/**
 * Session-scoped native Code Mode sessions.
 *
 * Primary path: in-process NAPI (`CodeModeSession` inside Node) — same model as
 * MCP linking core. Zero CLI spawn.
 *
 * Fallback: sticky `codemode-serve` child only when the `.node` addon is missing
 * (unsupported host / incomplete install). Doctor reports that as degraded.
 */
import { asEnvelope } from "./dispatch.js";
import { loadCodemodeNative } from "./native.js";
import { startStickyWorker } from "./worker.js";
function inProcessWorker(session, binding, root) {
    return {
        async call(tool, args) {
            const value = session.call(tool, args);
            return asEnvelope(value);
        },
        async batch(calls) {
            // Prefer session.call in a loop — reuses the warm Searcher already open.
            const results = [];
            let allOk = true;
            const t0 = Date.now();
            for (const c of calls) {
                try {
                    const value = session.call(c.tool, c.args);
                    results.push({ id: c.id, ok: true, value });
                }
                catch (cause) {
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
    #entries = new Map();
    #starting = new Map();
    #options = null;
    #generation = 0;
    #startFn;
    #backend = "none";
    constructor(startFn = startStickyWorker) {
        this.#startFn = startFn;
    }
    configure(options) {
        this.#options = options;
    }
    configured() {
        return this.#options !== null || loadCodemodeNative() !== null;
    }
    /** Active backend after first successful acquire. */
    backend() {
        return this.#backend;
    }
    async acquire(root) {
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
            throw new Error("native Code Mode backend unavailable");
        return worker.call(tool, args, options);
    }
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
        const gen = this.#generation;
        const opts = this.#options ?? {};
        // 1) In-process NAPI (preferred — zero spawn).
        const binding = loadCodemodeNative();
        if (binding) {
            try {
                const config = { root };
                if (opts.indexPath)
                    config.indexPath = opts.indexPath;
                if (opts.limit !== undefined)
                    config.limit = opts.limit;
                if (opts.useEmbed !== undefined)
                    config.useEmbed = opts.useEmbed;
                const session = new binding.Session(config);
                const worker = inProcessWorker(session, binding, root);
                if (gen !== this.#generation)
                    return null;
                this.#entries.set(root, { root, worker, generation: gen, backend: "napi" });
                this.#backend = "napi";
                return worker;
            }
            catch {
                // Fall through to CLI sticky.
            }
        }
        // 2) CLI sticky fallback (degraded).
        if (!opts.binary)
            return null;
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
            this.#entries.set(root, { root, worker, generation: gen, backend: "cli" });
            this.#backend = "cli";
            return worker;
        }
        catch {
            return null;
        }
    }
}
/** Singleton for advanced hosts; tools registration uses a local pool. */
export const sharedNativePool = new NativeSessionPool();
