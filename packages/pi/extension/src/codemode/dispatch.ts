/**
 * Same-tick call coalescing + typed batch / sticky-serve dispatch.
 *
 * Amdahl: serial cost is process spawn + SQLite open. Sticky serve kills spawn
 * for the whole Code Mode program; batch coalescing kills it per Promise.all wave.
 */

import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { MachineEnvelope } from "../runtime.js";
import type { ConnectorHost } from "./connector.js";

export type CodemodeToolCall = {
  tool: string;
  args: Record<string, unknown>;
};

export type DispatchStats = {
  waves: number;
  calls: number;
  batchedCalls: number;
  parallelSpawnCalls: number;
  stickyCalls: number;
  wallMs: number;
};

export type BatchResult = {
  results: Array<{ id: string; ok: boolean; value?: unknown; error?: string }>;
  mode?: string;
  wall_ms?: number;
  all_ok?: boolean;
};

export type StickyWorker = {
  call(
    tool: string,
    args: Record<string, unknown>,
    options?: { signal?: AbortSignal },
  ): Promise<MachineEnvelope>;
  batch(
    calls: Array<{ id: string; tool: string; args: Record<string, unknown> }>,
    options?: { signal?: AbortSignal },
  ): Promise<BatchResult>;
  end(): Promise<void>;
};

export type BatchCapableHost = ConnectorHost & {
  /** One-shot warm batch (codemode-batch). */
  runBatch?(
    calls: Array<{ id: string; tool: string; args: Record<string, unknown> }>,
    context: { cwd: string },
    options?: { signal?: AbortSignal },
  ): Promise<BatchResult>;
  /** Sticky NDJSON worker for the whole Code Mode program (preferred). */
  sticky?: StickyWorker | null;
};

type Pending = {
  tool: string;
  args: Record<string, unknown>;
  /** Legacy argv path when typed call is unavailable. */
  argv?: readonly string[];
  context: { cwd: string };
  options?: { signal?: AbortSignal };
  resolve: (value: MachineEnvelope) => void;
  reject: (reason: unknown) => void;
};

const MAX_WAVE = 32;

/**
 * Wraps a host so Promise.all([asgrep.search, asgrep.defs, …]) collapses into
 * one microtask wave. Prefers sticky serve → one-shot batch → overlapped spawn.
 */
export function createCodemodeDispatcher(host: BatchCapableHost): {
  host: ConnectorHost;
  stats: () => DispatchStats;
  resetStats: () => void;
} {
  let pending: Pending[] = [];
  let scheduled = false;
  let stats: DispatchStats = emptyStats();

  const flush = async () => {
    const wave = pending;
    pending = [];
    scheduled = false;
    if (wave.length === 0) return;

    stats.waves += 1;
    stats.calls += wave.length;
    const waveStarted = Date.now();

    try {
      if (wave.length === 1) {
        await settleOne(host, wave[0]!, stats);
        return;
      }

      // Chunk oversized waves (batch max = 32).
      for (let offset = 0; offset < wave.length; offset += MAX_WAVE) {
        const chunk = wave.slice(offset, offset + MAX_WAVE);
        await settleWave(host, chunk, stats);
      }
    } finally {
      stats.wallMs += Date.now() - waveStarted;
    }
  };

  const enqueue = (item: Pending): Promise<MachineEnvelope> =>
    new Promise<MachineEnvelope>((resolve, reject) => {
      item.resolve = resolve;
      item.reject = reject;
      pending.push(item);
      if (!scheduled) {
        scheduled = true;
        queueMicrotask(() => {
          void flush();
        });
      }
    });

  const dispatchHost: ConnectorHost = {
    call(tool, args, context, options) {
      const item: Pending = { tool, args, context, resolve: () => undefined, reject: () => undefined };
      if (options) item.options = options;
      return enqueue(item);
    },
    run(args, context, options) {
      // Legacy argv: still coalesce, but tool inference is best-effort only.
      const tool = toolFromArgs(args);
      const callArgs = argsObjectFromArgv(args);
      const item: Pending = {
        tool,
        args: callArgs,
        argv: args,
        context,
        resolve: () => undefined,
        reject: () => undefined,
      };
      if (options) item.options = options;
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

async function settleOne(host: BatchCapableHost, item: Pending, stats: DispatchStats): Promise<void> {
  try {
    if (host.sticky) {
      stats.stickyCalls += 1;
      item.resolve(await host.sticky.call(item.tool, item.args, item.options));
      return;
    }
    // N=1 without sticky: direct CLI (batch-of-1 is tempfile + protocol for no gain).
    const args = item.argv ?? argvFor(item.tool, item.args);
    item.resolve(await host.run(args, item.context, item.options));
  } catch (err) {
    item.reject(err);
  }
}

async function settleWave(host: BatchCapableHost, wave: Pending[], stats: DispatchStats): Promise<void> {
  if (host.sticky) {
    try {
      const calls = wave.map((item, index) => ({
        id: String(index),
        tool: item.tool,
        args: item.args,
      }));
      const batch = await host.sticky.batch(calls, wave[0]?.options);
      stats.stickyCalls += wave.length;
      settleFromBatch(wave, batch);
      return;
    } catch (cause) {
      // Sticky died mid-program — fall through to one-shot / spawn.
      void cause;
    }
  }

  if (host.runBatch) {
    try {
      const calls = wave.map((item, index) => ({
        id: String(index),
        tool: item.tool,
        args: item.args,
      }));
      const batch = await host.runBatch(calls, wave[0]!.context, wave[0]!.options);
      stats.batchedCalls += wave.length;
      settleFromBatch(wave, batch);
      return;
    } catch (cause) {
      // Transport failure only — do NOT re-run when per-call ok:false.
      void cause;
    }
  }

  stats.parallelSpawnCalls += wave.length;
  await Promise.all(
    wave.map(async (item) => {
      try {
        const args = item.argv ?? argvFor(item.tool, item.args);
        item.resolve(await host.run(args, item.context, item.options));
      } catch (err) {
        item.reject(err);
      }
    }),
  );
}

function settleFromBatch(wave: Pending[], batch: BatchResult): void {
  const byId = new Map(batch.results.map((r) => [r.id, r]));
  for (let i = 0; i < wave.length; i++) {
    const item = wave[i]!;
    const result = byId.get(String(i));
    if (!result) {
      item.reject(new Error(`codemode-batch missing result id=${i}`));
      continue;
    }
    if (!result.ok) {
      item.reject(new Error(result.error ?? `codemode call ${i} failed`));
      continue;
    }
    item.resolve(asEnvelope(result.value));
  }
}

function emptyStats(): DispatchStats {
  return {
    waves: 0,
    calls: 0,
    batchedCalls: 0,
    parallelSpawnCalls: 0,
    stickyCalls: 0,
    wallMs: 0,
  };
}

/** Build CLI argv for spawn fallback (typed path preferred). */
export function argvFor(tool: string, args: Record<string, unknown>): string[] {
  const limit = num(args.limit, 8);
  const excerpt = num(args.excerpt_lines ?? args.excerptLines, 0);
  const capsule = ["--json", "--format", "agent-capsule", "--limit", String(limit), "--excerpt-lines", String(excerpt)];
  switch (tool) {
    case "search":
      return [...capsule, String(args.query ?? ""), "."];
    case "semantic":
      return ["semantic", String(args.query ?? ""), ".", ...capsule];
    case "chain":
      return ["chain", String(args.query ?? ""), ".", "--json", "--limit", String(limit)];
    case "defs":
      return [...capsule, `defs:${String(args.symbol ?? "")}`, "."];
    case "callers":
      return [...capsule, `callers:${String(args.symbol ?? "")}`, "."];
    case "imports":
      return [...capsule, `imports:${String(args.module ?? "")}`, "."];
    case "index_status":
      return ["status", ".", "--json"];
    case "index_repo":
      return [args.force === true ? "reindex" : "index", ".", "--json"];
    default:
      return [...capsule, String(args.query ?? ""), "."];
  }
}

function num(value: unknown, fallback: number): number {
  if (typeof value === "number" && Number.isFinite(value)) return Math.trunc(value);
  return fallback;
}

function toolFromArgs(args: readonly string[]): string {
  if (args[0] === "semantic") return "semantic";
  if (args[0] === "chain") return "chain";
  if (args[0] === "status") return "index_status";
  if (args[0] === "index" || args[0] === "reindex") return "index_repo";
  if (args[0] === "codemode-batch" || args[0] === "codemode-serve") return "search";
  const query = args.length >= 2 ? args[args.length - 2]! : "";
  // Only classify prefix forms when the whole query is a navigator, so
  // search({ query: "defs: auth in login" }) stays search.
  if (/^defs:\s*\S+$/.test(query)) return "defs";
  if (/^callers:\s*\S+$/.test(query)) return "callers";
  if (/^imports:\s*\S+$/.test(query)) return "imports";
  return "search";
}

function argsObjectFromArgv(args: readonly string[]): Record<string, unknown> {
  if (args[0] === "status") return {};
  if (args[0] === "index") return { force: false };
  if (args[0] === "reindex") return { force: true };
  if (args[0] === "semantic" || args[0] === "chain") {
    const query = args[1] ?? "";
    const limit = readFlag(args, "--limit");
    const excerptLines = readFlag(args, "--excerpt-lines");
    const out: Record<string, unknown> = { query };
    if (limit !== undefined) out.limit = limit;
    if (excerptLines !== undefined) out.excerpt_lines = excerptLines;
    return out;
  }
  const query = args.length >= 2 ? args[args.length - 2]! : "";
  const limit = readFlag(args, "--limit");
  const excerptLines = readFlag(args, "--excerpt-lines");
  if (/^defs:\s*/.test(query)) {
    const out: Record<string, unknown> = { symbol: query.replace(/^defs:\s*/, "").trim() };
    if (limit !== undefined) out.limit = limit;
    if (excerptLines !== undefined) out.excerpt_lines = excerptLines;
    return out;
  }
  if (/^callers:\s*/.test(query)) {
    const out: Record<string, unknown> = { symbol: query.replace(/^callers:\s*/, "").trim() };
    if (limit !== undefined) out.limit = limit;
    if (excerptLines !== undefined) out.excerpt_lines = excerptLines;
    return out;
  }
  if (/^imports:\s*/.test(query)) {
    const out: Record<string, unknown> = { module: query.replace(/^imports:\s*/, "").trim() };
    if (limit !== undefined) out.limit = limit;
    if (excerptLines !== undefined) out.excerpt_lines = excerptLines;
    return out;
  }
  const out: Record<string, unknown> = { query, format: "capsule" };
  if (limit !== undefined) out.limit = limit;
  if (excerptLines !== undefined) out.excerpt_lines = excerptLines;
  return out;
}

function readFlag(args: readonly string[], flag: string): number | undefined {
  const idx = args.indexOf(flag);
  if (idx < 0 || idx + 1 >= args.length) return undefined;
  const n = Number(args[idx + 1]);
  return Number.isFinite(n) ? n : undefined;
}

export function asEnvelope(value: unknown): MachineEnvelope {
  if (
    value &&
    typeof value === "object" &&
    (value as MachineEnvelope).tool === "asgrep" &&
    typeof (value as MachineEnvelope).ok === "boolean"
  ) {
    return value as MachineEnvelope;
  }
  const record = value && typeof value === "object" ? (value as Record<string, unknown>) : { value };
  // Overrides AFTER spread so tool/ok/schema cannot be clobbered by payload fields.
  return {
    ...record,
    tool: "asgrep",
    schema_version: "1.0.0",
    ok: true,
  };
}

/** One-shot batch via stdin (no tempfile) when spawn-with-stdin is available. */
export async function runNativeBatch(
  run: ConnectorHost["run"],
  calls: Array<{ id: string; tool: string; args: Record<string, unknown> }>,
  context: { cwd: string },
  options?: { signal?: AbortSignal },
  writeBatch?: (body: string, context: { cwd: string }, options?: { signal?: AbortSignal }) => Promise<MachineEnvelope>,
): Promise<BatchResult> {
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
  } finally {
    await rm(dir, { recursive: true, force: true }).catch(() => undefined);
  }
}

function envelopeToBatch(envelope: MachineEnvelope): BatchResult {
  const results = Array.isArray(envelope.results)
    ? (envelope.results as Array<{ id: string; ok: boolean; value?: unknown; error?: string }>)
    : [];
  const out: BatchResult = { results };
  if (typeof envelope.mode === "string") out.mode = envelope.mode;
  if (typeof envelope.wall_ms === "number") out.wall_ms = envelope.wall_ms;
  if (typeof envelope.all_ok === "boolean") out.all_ok = envelope.all_ok;
  return out;
}
