import type { MachineEnvelope } from "../runtime.js";
import type { ChainArgs, SearchArgs } from "./types.js";
import {
  createCodemodeDispatcher,
  type BatchCapableHost,
  type DispatchStats,
} from "./dispatch.js";

const DEFAULT_LIMIT = 8;

export type ConnectorHost = {
  /** Typed tool call (preferred — no argv archaeology). */
  call?(
    tool: string,
    args: Record<string, unknown>,
    context: { cwd: string },
    options?: { signal?: AbortSignal },
  ): Promise<MachineEnvelope>;
  /** Legacy CLI argv (spawn fallback / direct tools). */
  run(args: readonly string[], context: { cwd: string }, options?: { signal?: AbortSignal }): Promise<MachineEnvelope>;
};

export type AsgrepConnector = {
  search(input: SearchArgs): Promise<MachineEnvelope>;
  semantic(input: SearchArgs): Promise<MachineEnvelope>;
  chain(input: ChainArgs): Promise<MachineEnvelope>;
  defs(input: { symbol: string; limit?: number; excerptLines?: number }): Promise<MachineEnvelope>;
  callers(input: { symbol: string; limit?: number; excerptLines?: number }): Promise<MachineEnvelope>;
  imports(input: { module: string; limit?: number; excerptLines?: number }): Promise<MachineEnvelope>;
  indexStatus(): Promise<MachineEnvelope>;
  indexRepo(input?: { force?: boolean }): Promise<MachineEnvelope>;
  /** Progressive discovery (like deferred tools) — list/filter available asgrep tools. */
  catalogSearch(input: { query: string }): Promise<MachineEnvelope>;
  catalogDescribe(input: { name: string }): Promise<MachineEnvelope>;
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
  const runOptions = options.signal ? { signal: options.signal } : {};

  const call = (tool: string, args: Record<string, unknown>) => {
    if (dispatcher.host.call) {
      return dispatcher.host.call(tool, args, context, runOptions);
    }
    // Should not happen — dispatcher always exposes call.
    return dispatcher.host.run([], context, runOptions);
  };

  // Bound function properties (not methods) so vm call sites cannot lose `this`.
  const asgrep: AsgrepConnector = {
    search: (input) =>
      call("search", {
        query: input.query,
        limit: clampLimit(input.limit),
        excerpt_lines: clampExcerpt(input.excerptLines),
        format: input.format === "agent" ? "agent" : "capsule",
      }),
    semantic: (input) =>
      call("semantic", {
        query: input.query,
        limit: clampLimit(input.limit),
        excerpt_lines: clampExcerpt(input.excerptLines),
        format: input.format === "agent" ? "agent" : "capsule",
      }),
    chain: (input) =>
      call("chain", {
        query: input.query,
        limit: clampLimit(input.limit),
        top_n: 20,
      }),
    defs: (input) =>
      call("defs", {
        symbol: input.symbol,
        limit: clampLimit(input.limit),
        excerpt_lines: clampExcerpt(input.excerptLines),
      }),
    callers: (input) =>
      call("callers", {
        symbol: input.symbol,
        limit: clampLimit(input.limit),
        excerpt_lines: clampExcerpt(input.excerptLines),
      }),
    imports: (input) =>
      call("imports", {
        module: input.module,
        limit: clampLimit(input.limit),
        excerpt_lines: clampExcerpt(input.excerptLines),
      }),
    indexStatus: () => call("index_status", {}),
    indexRepo: (input = {}) => call("index_repo", { force: input.force === true }),
    catalogSearch: (input) => call("catalog_search", { query: input.query }),
    catalogDescribe: (input) => call("catalog_describe", { name: input.name }),
  };

  return {
    asgrep,
    stats: dispatcher.stats,
    resetStats: dispatcher.resetStats,
  };
}
