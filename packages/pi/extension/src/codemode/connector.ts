import type { MachineEnvelope } from "../runtime.js";
import type { ChainArgs, SearchArgs } from "./types.js";
import { createCodemodeDispatcher, type BatchCapableHost, type DispatchStats } from "./dispatch.js";

const DEFAULT_LIMIT = 8;

export type ConnectorHost = {
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
};

export type ConnectorBundle = {
  asgrep: AsgrepConnector;
  stats: () => DispatchStats;
  resetStats: () => void;
};

function capsuleArgs(limit: number, excerptLines: number): string[] {
  return ["--json", "--format", "agent-capsule", "--limit", String(limit), "--excerpt-lines", String(excerptLines)];
}

function clampLimit(limit: number | undefined): number {
  if (limit === undefined) return DEFAULT_LIMIT;
  return Math.min(100, Math.max(1, Math.trunc(limit)));
}

function clampExcerpt(excerptLines: number | undefined): number {
  if (excerptLines === undefined) return 0;
  return Math.min(100, Math.max(0, Math.trunc(excerptLines)));
}

/**
 * Host-side connector: typed methods the sandbox calls.
 *
 * Same-tick calls (Promise.all) are coalesced by CodemodeDispatcher so N
 * lookups share one warm `codemode-batch` process when available, otherwise
 * overlapped CLI spawns.
 */
export function createAsgrepConnector(
  host: BatchCapableHost,
  context: { cwd: string },
  options: { signal?: AbortSignal } = {},
): ConnectorBundle {
  const dispatcher = createCodemodeDispatcher(host);
  const run = (args: readonly string[]) =>
    dispatcher.host.run(args, context, options.signal ? { signal: options.signal } : {});

  const searchLike = (query: string, limit?: number, excerptLines?: number) =>
    run([...capsuleArgs(clampLimit(limit), clampExcerpt(excerptLines)), query, "."]);

  // Bound function properties (not methods) so vm call sites cannot lose `this`.
  const asgrep: AsgrepConnector = {
    search: (input) => searchLike(input.query, input.limit, input.excerptLines),
    semantic: (input) =>
      run([
        "semantic",
        input.query,
        ".",
        ...capsuleArgs(clampLimit(input.limit), clampExcerpt(input.excerptLines)),
      ]),
    chain: (input) =>
      run([
        "chain",
        input.query,
        ".",
        ...capsuleArgs(clampLimit(input.limit), clampExcerpt(input.excerptLines)),
      ]),
    defs: (input) => searchLike(`defs: ${input.symbol}`, input.limit, input.excerptLines),
    callers: (input) => searchLike(`callers: ${input.symbol}`, input.limit, input.excerptLines),
    imports: (input) => searchLike(`imports: ${input.module}`, input.limit, input.excerptLines),
    indexStatus: () => run(["status", ".", "--json"]),
    indexRepo: (input = {}) => {
      const command = input.force === true ? "reindex" : "index";
      return run([command, ".", "--json"]);
    },
  };

  return {
    asgrep,
    stats: dispatcher.stats,
    resetStats: dispatcher.resetStats,
  };
}
