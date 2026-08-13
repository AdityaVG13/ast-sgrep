/**
 * Same-tick call coalescing + typed batch / sticky-serve dispatch.
 *
 * Amdahl: serial cost is process spawn + SQLite open. Sticky serve kills spawn
 * for the whole Code Mode program; batch coalescing kills it per Promise.all wave.
 */
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
const MAX_WAVE = 32;
const MUTATING_TOOLS = new Set(["index_repo"]);
const abortError = () => Object.assign(new Error("codemode aborted"), { name: "AbortError" });
function rejectWave(wave, cause) {
    for (const item of wave)
        item.reject(cause);
}
function sharedBatchOptions(wave) {
    const signal = wave[0]?.options?.signal;
    return signal && wave.every((item) => item.options?.signal === signal) ? { signal } : undefined;
}
function isSharedAbort(cause, options) {
    return options?.signal !== undefined
        && (options.signal.aborted || (cause instanceof Error && cause.name === "AbortError"));
}
/**
 * Wraps a host so Promise.all([asgrep.search, asgrep.defs, …]) collapses into
 * one microtask wave. Prefers sticky serve → one-shot batch → overlapped spawn.
 */
export function createCodemodeDispatcher(host) {
    let pending = [];
    let scheduled = false;
    let stats = emptyStats();
    const flush = async () => {
        const wave = pending.filter((item) => !item.settled);
        pending = [];
        scheduled = false;
        if (wave.length === 0)
            return;
        stats.waves += 1;
        stats.calls += wave.length;
        const waveStarted = Date.now();
        try {
            if (wave.length === 1) {
                await settleOne(host, wave[0], stats);
                return;
            }
            // Chunk oversized waves (batch max = 32).
            for (let offset = 0; offset < wave.length; offset += MAX_WAVE) {
                const chunk = wave.slice(offset, offset + MAX_WAVE).filter((item) => !item.settled);
                if (chunk.length === 0)
                    continue;
                await settleWave(host, chunk, stats);
            }
        }
        finally {
            stats.wallMs += Date.now() - waveStarted;
        }
    };
    const enqueue = (item) => new Promise((resolve, reject) => {
        const signal = item.options?.signal;
        const cleanup = () => signal?.removeEventListener("abort", onAbort);
        item.resolve = (value) => {
            if (item.settled)
                return;
            item.settled = true;
            cleanup();
            resolve(value);
        };
        item.reject = (reason) => {
            if (item.settled)
                return;
            item.settled = true;
            cleanup();
            reject(reason);
        };
        const onAbort = () => item.reject(abortError());
        if (signal?.aborted) {
            item.reject(abortError());
            return;
        }
        signal?.addEventListener("abort", onAbort, { once: true });
        pending.push(item);
        if (!scheduled) {
            scheduled = true;
            queueMicrotask(() => {
                void flush();
            });
        }
    });
    const dispatchHost = {
        call(tool, args, context, options) {
            const item = {
                tool,
                args,
                context,
                settled: false,
                resolve: () => undefined,
                reject: () => undefined,
            };
            if (options)
                item.options = options;
            return enqueue(item);
        },
    };
    return {
        host: dispatchHost,
        stats: () => ({ ...stats }),
        resetStats: () => {
            stats = emptyStats();
        },
    };
}
async function settleOne(host, item, stats) {
    try {
        if (host.sticky) {
            stats.stickyCalls += 1;
            item.resolve(await host.sticky.call(item.tool, item.args, item.options));
            return;
        }
        // N=1 without sticky: direct CLI (batch-of-1 is tempfile + protocol for no gain).
        const args = argvFor(item.tool, item.args);
        item.resolve(await host.run(args, item.context, item.options));
    }
    catch (err) {
        item.reject(err);
    }
}
async function settleWave(host, wave, stats) {
    if (host.sticky) {
        const transportOptions = sharedBatchOptions(wave);
        try {
            const calls = wave.map((item, index) => ({
                id: String(index),
                tool: item.tool,
                args: item.args,
            }));
            const batch = await host.sticky.batch(calls, transportOptions);
            stats.stickyCalls += wave.length;
            settleFromBatch(wave, batch);
            return;
        }
        catch (cause) {
            if (isSharedAbort(cause, transportOptions)) {
                rejectWave(wave, cause);
                return;
            }
            if (wave.some((item) => MUTATING_TOOLS.has(item.tool))) {
                // The worker may have committed a mutation before its transport died.
                // Replaying the wave through another transport would be ambiguous.
                rejectWave(wave, cause);
                return;
            }
            // Sticky died mid-program — fall through to one-shot / spawn.
        }
    }
    const batchWave = wave.filter((item) => !item.settled);
    if (batchWave.length === 0)
        return;
    if (host.runBatch) {
        const transportOptions = sharedBatchOptions(batchWave);
        try {
            const calls = batchWave.map((item, index) => ({
                id: String(index),
                tool: item.tool,
                args: item.args,
            }));
            const batch = await host.runBatch(calls, batchWave[0].context, transportOptions);
            stats.batchedCalls += batchWave.length;
            settleFromBatch(batchWave, batch);
            return;
        }
        catch (cause) {
            if (isSharedAbort(cause, transportOptions)) {
                rejectWave(batchWave, cause);
                return;
            }
            if (batchWave.some((item) => MUTATING_TOOLS.has(item.tool))) {
                rejectWave(batchWave, cause);
                return;
            }
            // Transport failure only — do NOT re-run when per-call ok:false.
        }
    }
    const spawnWave = batchWave.filter((item) => !item.settled);
    stats.parallelSpawnCalls += spawnWave.length;
    await Promise.all(spawnWave.map(async (item) => {
        try {
            const args = argvFor(item.tool, item.args);
            item.resolve(await host.run(args, item.context, item.options));
        }
        catch (err) {
            item.reject(err);
        }
    }));
}
function settleFromBatch(wave, batch) {
    const byId = new Map(batch.results.map((r) => [r.id, r]));
    for (let i = 0; i < wave.length; i++) {
        const item = wave[i];
        const result = byId.get(String(i));
        if (!result) {
            item.reject(new Error(`codemode-batch missing result id=${i}`));
            continue;
        }
        if (!result.ok) {
            item.reject(new Error(result.error ?? `codemode call ${i} failed`));
            continue;
        }
        item.resolve(asEnvelope(result.value, item.tool));
    }
}
function emptyStats() {
    return {
        waves: 0,
        calls: 0,
        batchedCalls: 0,
        parallelSpawnCalls: 0,
        stickyCalls: 0,
        wallMs: 0,
    };
}
const ARGV_SPEC = {
    search: { form: "capsule", key: "query" },
    semantic: { form: "semantic" },
    chain: { form: "chain" },
    defs: { form: "capsule", key: "symbol", prefix: "defs" },
    callers: { form: "capsule", key: "symbol", prefix: "callers" },
    imports: { form: "capsule", key: "module", prefix: "imports" },
    index_status: { form: "status" },
    index_repo: { form: "index_repo" },
};
function argStr(args, key) {
    return String(args[key] ?? "");
}
export function argvFor(tool, args) {
    const spec = ARGV_SPEC[tool];
    if (!spec)
        throw new Error(`codemode tool has no direct CLI fallback: ${tool}`);
    if (spec.form === "status")
        return ["status", ".", "--json"];
    if (spec.form === "index_repo") {
        const command = args.force === true ? "reindex" : "index";
        const paths = Array.isArray(args.paths)
            ? args.paths.filter((path) => typeof path === "string")
            : [];
        return [command, ".", "--json", ...paths.flatMap((path) => ["--path", path])];
    }
    const limit = num(args.limit, 8);
    if (spec.form === "chain") {
        return ["chain", argStr(args, "query"), ".", "--json", "--limit", String(limit)];
    }
    const excerpt = num(args.excerpt_lines ?? args.excerptLines, 0);
    const capsule = ["--json", "--format", "agent-capsule", "--limit", String(limit), "--excerpt-lines", String(excerpt)];
    if (spec.form === "semantic") {
        return ["semantic", argStr(args, "query"), ".", ...capsule];
    }
    // capsule (+ optional prefix for defs/callers/imports)
    const raw = argStr(args, spec.key);
    const token = spec.prefix ? `${spec.prefix}:${raw}` : raw;
    return [...capsule, token, "."];
}
function num(value, fallback) {
    if (typeof value === "number" && Number.isFinite(value))
        return Math.trunc(value);
    return fallback;
}
export function asEnvelope(value, command) {
    if (value &&
        typeof value === "object" &&
        value.tool === "asgrep" &&
        typeof value.ok === "boolean") {
        return value;
    }
    const record = value && typeof value === "object" ? value : { value };
    // Overrides AFTER spread so tool/ok/schema cannot be clobbered by payload fields.
    return {
        ...record,
        tool: "asgrep",
        ...(command ? { command } : {}),
        schema_version: "1.0.0",
        ok: true,
    };
}
/** One-shot batch via stdin (no tempfile) when spawn-with-stdin is available. */
export async function runNativeBatch(run, calls, context, options, writeBatch) {
    const body = JSON.stringify({
        root: context.cwd,
        // Auto/serial warm by default — N parallel SQLite opens are usually slower.
        parallel_mode: "auto",
        calls,
    });
    if (writeBatch) {
        const envelope = await writeBatch(body, context, options);
        return envelopeToBatch(envelope);
    }
    // Fallback: tempfile + pi.exec (no stdin).
    const dir = await mkdtemp(join(tmpdir(), "asgrep-codemode-"));
    const requestsPath = join(dir, "requests.json");
    try {
        await writeFile(requestsPath, body, "utf8");
        const envelope = await run(["codemode-batch", "--requests", requestsPath, "--json"], context, options);
        return envelopeToBatch(envelope);
    }
    finally {
        await rm(dir, { recursive: true, force: true }).catch(() => undefined);
    }
}
function envelopeToBatch(envelope) {
    const results = Array.isArray(envelope.results)
        ? envelope.results
        : [];
    const out = { results };
    if (typeof envelope.mode === "string")
        out.mode = envelope.mode;
    if (typeof envelope.wall_ms === "number")
        out.wall_ms = envelope.wall_ms;
    if (typeof envelope.all_ok === "boolean")
        out.all_ok = envelope.all_ok;
    return out;
}
