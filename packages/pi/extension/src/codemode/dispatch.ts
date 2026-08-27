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
import type { ConnectorHost, DispatchSurface } from "./connector.js";

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
  context: { cwd: string };
  options?: { signal?: AbortSignal };
  settled: boolean;
  resolve: (value: MachineEnvelope) => void;
  reject: (reason: unknown) => void;
};

const MAX_WAVE = 32;
const MUTATING_TOOLS = new Set(["index_repo", "edit"]);

const abortError = (): Error => Object.assign(new Error("codemode aborted"), { name: "AbortError" });

function rejectWave(wave: Pending[], cause: unknown): void {
  for (const item of wave) item.reject(cause);
}

function sharedBatchOptions(wave: Pending[]): { signal?: AbortSignal } | undefined {
  const signal = wave[0]?.options?.signal;
  return signal && wave.every((item) => item.options?.signal === signal) ? { signal } : undefined;
}

function isSharedAbort(cause: unknown, options: { signal?: AbortSignal } | undefined): boolean {
  return options?.signal !== undefined
    && (options.signal.aborted || (cause instanceof Error && cause.name === "AbortError"));
}

/**
 * Wraps a host so Promise.all([asgrep.search, asgrep.defs, …]) collapses into
 * one microtask wave. Prefers sticky serve → one-shot batch → overlapped spawn.
 */
export function createCodemodeDispatcher(host: BatchCapableHost): {
  host: DispatchSurface;
  stats: () => DispatchStats;
  resetStats: () => void;
} {
  let pending: Pending[] = [];
  let scheduled = false;
  let stats: DispatchStats = emptyStats();

  const flush = async () => {
    const wave = pending.filter((item) => !item.settled);
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
        const chunk = wave.slice(offset, offset + MAX_WAVE).filter((item) => !item.settled);
        if (chunk.length === 0) continue;
        await settleWave(host, chunk, stats);
      }
    } finally {
      stats.wallMs += Date.now() - waveStarted;
    }
  };

  const enqueue = (item: Pending): Promise<MachineEnvelope> =>
    new Promise<MachineEnvelope>((resolve, reject) => {
      const signal = item.options?.signal;
      const cleanup = () => signal?.removeEventListener("abort", onAbort);
      item.resolve = (value) => {
        if (item.settled) return;
        item.settled = true;
        cleanup();
        resolve(value);
      };
      item.reject = (reason) => {
        if (item.settled) return;
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

  const dispatchHost: DispatchSurface = {
    call(tool, args, context, options) {
      const item: Pending = {
        tool,
        args,
        context,
        settled: false,
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
    const args = argvFor(item.tool, item.args);
    item.resolve(await host.run(args, item.context, item.options));
  } catch (err) {
    item.reject(err);
  }
}

async function settleWave(host: BatchCapableHost, wave: Pending[], stats: DispatchStats): Promise<void> {
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
    } catch (cause) {
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
  if (batchWave.length === 0) return;
  if (host.runBatch) {
    const transportOptions = sharedBatchOptions(batchWave);
    try {
      const calls = batchWave.map((item, index) => ({
        id: String(index),
        tool: item.tool,
        args: item.args,
      }));
      const batch = await host.runBatch(calls, batchWave[0]!.context, transportOptions);
      stats.batchedCalls += batchWave.length;
      settleFromBatch(batchWave, batch);
      return;
    } catch (cause) {
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
  await Promise.all(
    spawnWave.map(async (item) => {
      try {
        const args = argvFor(item.tool, item.args);
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
    item.resolve(asEnvelope(result.value, item.tool));
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

/** Build CLI argv for spawn fallback (typed path preferred). Data-driven tool table. */
type ArgvSpec =
  | { form: "capsule"; key: "query" | "symbol" | "module"; prefix?: string }
  | { form: "semantic" }
  | { form: "chain" }
  | { form: "find" }
  | { form: "status" }
  | { form: "index_repo" };

const ARGV_SPEC: Record<string, ArgvSpec> = {
  search: { form: "capsule", key: "query" },
  find: { form: "find" },
  semantic: { form: "semantic" },
  chain: { form: "chain" },
  defs: { form: "capsule", key: "symbol", prefix: "defs" },
  callers: { form: "capsule", key: "symbol", prefix: "callers" },
  imports: { form: "capsule", key: "module", prefix: "imports" },
  index_status: { form: "status" },
  index_repo: { form: "index_repo" },
};

function argStr(args: Record<string, unknown>, key: string): string {
  return String(args[key] ?? "");
}

export function argvFor(tool: string, args: Record<string, unknown>): string[] {
  const spec = ARGV_SPEC[tool];
  if (!spec) throw new Error(`codemode tool has no direct CLI fallback: ${tool}`);
  if (spec.form === "status") return ["status", ".", "--json"];
  if (spec.form === "index_repo") {
    const command = args.force === true ? "reindex" : "index";
    const paths = Array.isArray(args.paths)
      ? args.paths.filter((path): path is string => typeof path === "string")
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
  if (spec.form === "find") {
    const raw = argStr(args, "query").trim();
    let token = raw;
    if (/^blast:/i.test(raw)) {
      const target = raw.slice(raw.indexOf(":") + 1).trim();
      token = /[\\/.]/.test(target) ? `imports:${target}` : `callers:${target}`;
    } else if (!/^(defs|callers|imports|literal|regex|word|pattern):/i.test(raw)) {
      token = `word:${raw}`;
    }
    return [...capsule, token, "."];
  }
  // capsule (+ optional prefix for defs/callers/imports)
  const raw = argStr(args, spec.key);
  const token = spec.prefix ? `${spec.prefix}:${raw}` : raw;
  return [...capsule, token, "."];
}

function num(value: unknown, fallback: number): number {
  if (typeof value === "number" && Number.isFinite(value)) return Math.trunc(value);
  return fallback;
}


export function asEnvelope(value: unknown, command?: string): MachineEnvelope {
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
    ...(command ? { command } : {}),
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
