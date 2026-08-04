import type { MachineEnvelope } from "../runtime.js";
import type { ChainArgs, SearchArgs } from "./types.js";

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
 * Host-side connector: typed methods the sandbox calls. Each method maps to one
 * native CLI invocation. Independent methods may run concurrently via Promise.all.
 */
export function createAsgrepConnector(
  host: ConnectorHost,
  context: { cwd: string },
  options: { signal?: AbortSignal } = {},
): AsgrepConnector {
  const run = (args: readonly string[]) => host.run(args, context, options.signal ? { signal: options.signal } : {});

  const searchLike = (query: string, limit?: number, excerptLines?: number) =>
    run([...capsuleArgs(clampLimit(limit), clampExcerpt(excerptLines)), query, "."]);

  return {
    search(input) {
      return searchLike(input.query, input.limit, input.excerptLines);
    },
    semantic(input) {
      return run([
        "semantic",
        input.query,
        ".",
        ...capsuleArgs(clampLimit(input.limit), clampExcerpt(input.excerptLines)),
      ]);
    },
    chain(input) {
      return run([
        "chain",
        input.query,
        ".",
        ...capsuleArgs(clampLimit(input.limit), clampExcerpt(input.excerptLines)),
      ]);
    },
    defs(input) {
      return searchLike(`defs: ${input.symbol}`, input.limit, input.excerptLines);
    },
    callers(input) {
      return searchLike(`callers: ${input.symbol}`, input.limit, input.excerptLines);
    },
    imports(input) {
      return searchLike(`imports: ${input.module}`, input.limit, input.excerptLines);
    },
    indexStatus() {
      return run(["status", ".", "--json"]);
    },
    indexRepo(input = {}) {
      const command = input.force === true ? "reindex" : "index";
      return run([command, ".", "--json"]);
    },
  };
}
