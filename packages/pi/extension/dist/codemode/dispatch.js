/**
 * Same-tick call coalescing + optional warm batch process.
 *
 * Amdahl: for N independent Code Mode tool calls started in one Promise.all,
 * one process (warm Searcher / parallel SQLite readers) beats N cold CLI spawns.
 * Serial fraction ≈ process start + SQLite open; parallel fraction ≈ search work.
 */
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
const MAX_WAVE = 32;
/**
 * Wraps a ConnectorHost so Promise.all([asgrep.search, asgrep.defs, …]) collapses
 * into one microtask wave and prefers a single batch process when available.
 */
export function createCodemodeDispatcher(host) {
    let pending = [];
    let scheduled = false;
    let stats = emptyStats();
    const flush = async () => {
        const wave = pending;
        pending = [];
        scheduled = false;
        if (wave.length === 0)
            return;
        stats.waves += 1;
        stats.calls += wave.length;
        const waveStarted = Date.now();
        try {
            if (wave.length === 1) {
                const item = wave[0];
                const value = await host.run(item.args, item.context, item.options);
                item.resolve(value);
            }
            else if (host.runBatch && wave.length <= MAX_WAVE && wave.every(isBatchableArgs)) {
                const calls = wave.map((item, index) => ({
                    id: String(index),
                    tool: toolFromArgs(item.args),
                    args: argsObjectFromArgv(item.args),
                }));
                try {
                    const batch = await host.runBatch(calls, wave[0].context, wave[0].options);
                    stats.batchedCalls += wave.length;
                    const byId = new Map(batch.results.map((r) => [r.id, r]));
                    for (let i = 0; i < wave.length; i++) {
                        const item = wave[i];
                        const result = byId.get(String(i));
                        if (!result) {
                            item.reject(new Error(`codemode-batch missing result id=${i}`));
                            continue;
                        }
                        if (!result.ok) {
                            item.reject(new Error(result.error ?? `codemode-batch call ${i} failed`));
                            continue;
                        }
                        item.resolve(asEnvelope(result.value));
                    }
                }
                catch (cause) {
                    // Binary may not support batch yet — fall back to overlapped spawns.
                    stats.parallelSpawnCalls += wave.length;
                    await Promise.all(wave.map(async (item) => {
                        try {
                            item.resolve(await host.run(item.args, item.context, item.options));
                        }
                        catch (err) {
                            item.reject(err);
                        }
                    }));
                    void cause;
                }
            }
            else {
                stats.parallelSpawnCalls += wave.length;
                await Promise.all(wave.map(async (item) => {
                    try {
                        item.resolve(await host.run(item.args, item.context, item.options));
                    }
                    catch (err) {
                        item.reject(err);
                    }
                }));
            }
        }
        finally {
            stats.wallMs += Date.now() - waveStarted;
        }
    };
    const dispatchHost = {
        run(args, context, options) {
            return new Promise((resolve, reject) => {
                const item = { args, context, resolve, reject };
                if (options)
                    item.options = options;
                pending.push(item);
                if (!scheduled) {
                    scheduled = true;
                    queueMicrotask(() => {
                        void flush();
                    });
                }
            });
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
function emptyStats() {
    return { waves: 0, calls: 0, batchedCalls: 0, parallelSpawnCalls: 0, wallMs: 0 };
}
function isBatchableArgs(item) {
    const head = item.args[0];
    // Search-style argv starts with --json; subcommands we support in batch.
    if (head === "--json")
        return true;
    if (head === "semantic" || head === "chain" || head === "status" || head === "index" || head === "reindex") {
        return true;
    }
    return false;
}
function toolFromArgs(args) {
    if (args[0] === "semantic")
        return "semantic";
    if (args[0] === "chain")
        return "chain";
    if (args[0] === "status")
        return "index_status";
    if (args[0] === "index" || args[0] === "reindex")
        return "index_repo";
    // Capsule search argv: --json --format agent-capsule --limit N --excerpt-lines E QUERY .
    const query = args.length >= 2 ? args[args.length - 2] : "";
    if (query.startsWith("defs:"))
        return "defs";
    if (query.startsWith("callers:"))
        return "callers";
    if (query.startsWith("imports:"))
        return "imports";
    return "search";
}
function argsObjectFromArgv(args) {
    if (args[0] === "status")
        return {};
    if (args[0] === "index")
        return { force: false };
    if (args[0] === "reindex")
        return { force: true };
    if (args[0] === "semantic" || args[0] === "chain") {
        const query = args[1] ?? "";
        const limit = readFlag(args, "--limit");
        const excerptLines = readFlag(args, "--excerpt-lines");
        const out = { query };
        if (limit !== undefined)
            out.limit = limit;
        if (excerptLines !== undefined)
            out.excerpt_lines = excerptLines;
        return out;
    }
    // search / defs / callers / imports
    const query = args.length >= 2 ? args[args.length - 2] : "";
    const limit = readFlag(args, "--limit");
    const excerptLines = readFlag(args, "--excerpt-lines");
    if (query.startsWith("defs:")) {
        return { symbol: query.slice("defs:".length).trim(), ...(limit !== undefined ? { limit } : {}) };
    }
    if (query.startsWith("callers:")) {
        return { symbol: query.slice("callers:".length).trim(), ...(limit !== undefined ? { limit } : {}) };
    }
    if (query.startsWith("imports:")) {
        return { module: query.slice("imports:".length).trim(), ...(limit !== undefined ? { limit } : {}) };
    }
    const out = { query, format: "capsule" };
    if (limit !== undefined)
        out.limit = limit;
    if (excerptLines !== undefined)
        out.excerpt_lines = excerptLines;
    return out;
}
function readFlag(args, flag) {
    const idx = args.indexOf(flag);
    if (idx < 0 || idx + 1 >= args.length)
        return undefined;
    const n = Number(args[idx + 1]);
    return Number.isFinite(n) ? n : undefined;
}
function asEnvelope(value) {
    if (value && typeof value === "object" && value.tool === "asgrep") {
        return value;
    }
    // Plugin/agent JSON from Rust session — wrap for connector consumers that expect envelopes.
    const record = (value && typeof value === "object") ? value : { value };
    return {
        tool: "asgrep",
        schema_version: "1.0.0",
        ok: true,
        ...record,
    };
}
/** Runtime helper: write requests file and invoke `asgrep codemode-batch`. */
export async function runNativeBatch(run, calls, context, options) {
    const dir = await mkdtemp(join(tmpdir(), "asgrep-codemode-"));
    const requestsPath = join(dir, "requests.json");
    try {
        await writeFile(requestsPath, JSON.stringify({
            root: context.cwd,
            parallel: true,
            calls,
        }), "utf8");
        const envelope = await run(["codemode-batch", "--requests", requestsPath, "--json"], context, options);
        const results = Array.isArray(envelope.results) ? envelope.results : [];
        const out = { results };
        if (typeof envelope.mode === "string")
            out.mode = envelope.mode;
        if (typeof envelope.wall_ms === "number")
            out.wall_ms = envelope.wall_ms;
        return out;
    }
    finally {
        await rm(dir, { recursive: true, force: true }).catch(() => undefined);
    }
}
