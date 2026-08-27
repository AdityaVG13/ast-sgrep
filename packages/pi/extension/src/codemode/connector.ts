import type { MachineEnvelope } from "../runtime.js";
import type { ChainArgs, EditArgs, FindArgs, ReadArgs, SearchArgs } from "./types.js";
import {
  createCodemodeDispatcher,
  type BatchCapableHost,
  type DispatchStats,
} from "./dispatch.js";

const DEFAULT_LIMIT = 8;

/**
 * Spawn/CLI transport. Hosts provide argv `run` only — never a typed twin.
 * Typed entry lives solely on {@link DispatchSurface} (dispatcher output).
 */
export type ConnectorHost = {
  run(
    args: readonly string[],
    context: { cwd: string },
    options?: { signal?: AbortSignal },
  ): Promise<MachineEnvelope>;
};

/**
 * Trusted typed dispatch after coalescing. `call` is required; no argv peer
 * that can disagree with tool+args.
 */
export type DispatchSurface = {
  call(
    tool: string,
    args: Record<string, unknown>,
    context: { cwd: string },
    options?: { signal?: AbortSignal },
  ): Promise<MachineEnvelope>;
};

export type AsgrepConnector = {
  search(input: SearchArgs, options?: { signal?: AbortSignal }): Promise<MachineEnvelope>;
  find(input: FindArgs, options?: { signal?: AbortSignal }): Promise<MachineEnvelope>;
  read(input: ReadArgs, options?: { signal?: AbortSignal }): Promise<MachineEnvelope>;
  edit(input: EditArgs, options?: { signal?: AbortSignal }): Promise<MachineEnvelope>;
  semantic(input: SearchArgs, options?: { signal?: AbortSignal }): Promise<MachineEnvelope>;
  chain(input: ChainArgs, options?: { signal?: AbortSignal }): Promise<MachineEnvelope>;
  defs(input: { symbol: string; limit?: number; excerptLines?: number }, options?: { signal?: AbortSignal }): Promise<MachineEnvelope>;
  callers(input: { symbol: string; limit?: number; excerptLines?: number }, options?: { signal?: AbortSignal }): Promise<MachineEnvelope>;
  imports(input: { module: string; limit?: number; excerptLines?: number }, options?: { signal?: AbortSignal }): Promise<MachineEnvelope>;
  indexStatus(options?: { signal?: AbortSignal }): Promise<MachineEnvelope>;
  indexRepo(input?: { force?: boolean }, options?: { signal?: AbortSignal }): Promise<MachineEnvelope>;
  /** Progressive discovery (like deferred tools) — list/filter available asgrep tools. */
  catalogSearch(input: { query: string }, options?: { signal?: AbortSignal }): Promise<MachineEnvelope>;
  catalogDescribe(input: { name: string }, options?: { signal?: AbortSignal }): Promise<MachineEnvelope>;
};

export type ConnectorBundle = {
  asgrep: AsgrepConnector;
  stats: () => DispatchStats;
  resetStats: () => void;
};

function clampLimit(limit: number | undefined): number {
  if (limit === undefined) return DEFAULT_LIMIT;
  return Math.min(100, Math.max(1, Math.trunc(limit)));
}

function clampExcerpt(excerptLines: number | undefined): number {
  if (excerptLines === undefined) return 0;
  return Math.min(100, Math.max(0, Math.trunc(excerptLines)));
}

/**
 * Host-side connector: typed methods the Code Mode program calls.
 *
 * Same-tick calls (Promise.all) are coalesced by CodemodeDispatcher so N
 * lookups share sticky serve / one warm batch process when available.
 */
export function createAsgrepConnector(
  host: BatchCapableHost,
  context: { cwd: string },
  options: { signal?: AbortSignal } = {},
): ConnectorBundle {
  const dispatcher = createCodemodeDispatcher(host);
  const combinedSignals = new WeakMap<AbortSignal, AbortSignal>();
  const callOptions = (signal?: AbortSignal): { signal?: AbortSignal } => {
    if (!options.signal) return signal ? { signal } : {};
    if (!signal || signal === options.signal) return { signal: options.signal };
    let combined = combinedSignals.get(signal);
    if (!combined) {
      combined = AbortSignal.any([options.signal, signal]);
      combinedSignals.set(signal, combined);
    }
    return { signal: combined };
  };

  const call = (tool: string, args: Record<string, unknown>, signal?: AbortSignal) =>
    dispatcher.host.call(tool, args, context, callOptions(signal));

  // Bound function properties (not methods) so vm call sites cannot lose `this`.
  const asgrep: AsgrepConnector = {
    search: (input, callOptions) =>
      call("search", {
        query: input.query,
        limit: clampLimit(input.limit),
        excerpt_lines: clampExcerpt(input.excerptLines),
        format: input.format === "agent" ? "agent" : "capsule",
      }, callOptions?.signal),
    find: (input, callOptions) =>
      call("find", {
        query: input.query,
        limit: clampLimit(input.limit),
        excerpt_lines: clampExcerpt(input.excerptLines),
        format: input.format === "agent" ? "agent" : "capsule",
      }, callOptions?.signal),
    read: (input, callOptions) =>
      call("read", {
        ...(typeof input.path === "string" ? { path: input.path } : {}),
        ...(input.start !== undefined ? { start: input.start } : {}),
        ...(input.end !== undefined ? { end: input.end } : {}),
        ...(typeof input.ref === "string" ? { ref: input.ref } : {}),
        ...(input.refs !== undefined ? { refs: input.refs } : {}),
        ...(input.contextLines !== undefined ? { context_lines: input.contextLines } : {}),
        ...(input.maxChars !== undefined ? { max_chars: input.maxChars } : {}),
      }, callOptions?.signal),
    edit: (input, callOptions) =>
      call("edit", {
        ...(typeof input.path === "string" ? { path: input.path } : {}),
        ...(typeof input.oldText === "string" ? { oldText: input.oldText } : {}),
        ...(typeof input.newText === "string" ? { newText: input.newText } : {}),
        ...(input.edits !== undefined ? { edits: input.edits } : {}),
      }, callOptions?.signal),
    semantic: (input, callOptions) =>
      call("semantic", {
        query: input.query,
        limit: clampLimit(input.limit),
        excerpt_lines: clampExcerpt(input.excerptLines),
        format: input.format === "agent" ? "agent" : "capsule",
      }, callOptions?.signal),
    chain: (input, callOptions) =>
      call("chain", {
        query: input.query,
        limit: clampLimit(input.limit),
        top_n: 20,
      }, callOptions?.signal),
    defs: (input, callOptions) =>
      call("defs", {
        symbol: input.symbol,
        limit: clampLimit(input.limit),
        excerpt_lines: clampExcerpt(input.excerptLines),
      }, callOptions?.signal),
    callers: (input, callOptions) =>
      call("callers", {
        symbol: input.symbol,
        limit: clampLimit(input.limit),
        excerpt_lines: clampExcerpt(input.excerptLines),
      }, callOptions?.signal),
    imports: (input, callOptions) =>
      call("imports", {
        module: input.module,
        limit: clampLimit(input.limit),
        excerpt_lines: clampExcerpt(input.excerptLines),
      }, callOptions?.signal),
    indexStatus: (callOptions) => call("index_status", {}, callOptions?.signal),
    indexRepo: (input = {}, callOptions) => call("index_repo", { force: input.force === true }, callOptions?.signal),
    catalogSearch: (input, callOptions) => call("catalog_search", { query: input.query }, callOptions?.signal),
    catalogDescribe: (input, callOptions) => call("catalog_describe", { name: input.name }, callOptions?.signal),
  };

  return {
    asgrep,
    stats: dispatcher.stats,
    resetStats: dispatcher.resetStats,
  };
}
