/**
 * The four registered pi tools: asgrep (Code Mode), asgrep_search,
 * asgrep_index, asgrep_status — plus the session pool, freshness wiring,
 * and workspace event hooks that serve them.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { existsSync } from "node:fs";
import { dirname, isAbsolute, join, relative, sep } from "node:path";
import { Type } from "typebox";
import {
  createAsgrepConnector,
  runCodemode,
  runNativeBatch,
  runBatchViaStdin,
  CODEMODE_TYPES_FOR_MODEL,
  NativeSessionPool,
  argvFor,
  asEnvelope,
  applyQueryScope,
  warmCodemodeSandbox,
  resetCodemodeSandboxForTests,
  isClosedWorkerError,
  type StickyWorker,
} from "../codemode/index.js";
import { AstSgrepRuntime, FreshnessCoordinator, RuntimeError } from "../runtime/runtime.js";
import type { MachineEnvelope, RunOptions } from "../runtime/types.js";
import type { FreshnessRuntime } from "../runtime/freshness.js";
import { RESOLVED_ROOT } from "../runtime/types.js";
import {
  ASGREP_PROMPT_GUIDELINES,
  ASGREP_PROMPT_GUIDELINES_HOST_FILES,
  ASGREP_PROMPT_SNIPPET,
  formatCodemodeResult,
} from "../ui/present.js";
import { EMPTY_CALL, renderAsgrepResult } from "../ui/card.js";
import {
  bounded,
  errorDetails,
  failure,
  isFreshnessTimeout,
  extractInPath,
  report,
  success,
  type FreshnessLike,
  type RuntimeLike,
  type ToolContext,
  type Update,
} from "./results.js";

export const DEFAULT_LIMIT = 8;
const MAX_LIMIT = 100;
const MAX_EXCERPT_LINES = 100;

const searchParameters = Type.Object({
  query: Type.String({ maxLength: 4_096 }),
  mode: Type.Optional(Type.Unsafe<SearchMode>({
    type: "string",
    enum: ["natural", "pattern", "defs", "callers", "chain", "semantic", "word", "literal", "regex", "imports"],
    default: "natural",
  })),
  limit: Type.Optional(Type.Integer({ default: DEFAULT_LIMIT })),
  excerptLines: Type.Optional(Type.Integer({ default: 0 })),
  in: Type.Optional(Type.String({ maxLength: 512, description: "Bound to a directory or glob" })),
  lang: Type.Optional(Type.String({ maxLength: 32, description: "Language filter (rs, ts, py)" })),
}, { additionalProperties: false });

const indexParameters = Type.Object({
  force: Type.Optional(Type.Boolean({ default: false, description: "Rebuild the index from scratch" })),
}, { additionalProperties: false });


const editParameters = Type.Object({
  path: Type.Optional(Type.String({ maxLength: 512 })),
  oldText: Type.Optional(Type.String({ description: "Exact text to replace (must match once)" })),
  newText: Type.Optional(Type.String()),
  edits: Type.Optional(Type.Array(Type.Object({
    path: Type.Optional(Type.String({ minLength: 1, maxLength: 512 })),
    oldText: Type.String({ minLength: 1 }),
    newText: Type.String(),
  }), { maxItems: 64, description: "Multi-edit entries; top-level path is the default" })),
}, { additionalProperties: false });

const readParameters = Type.Object({
  path: Type.Optional(Type.String({ maxLength: 512 })),
  ref: Type.Optional(Type.String({ description: "Hit ref path#L12-L40" })),
  refs: Type.Optional(Type.Array(Type.String(), { maxItems: 24, description: "Several refs in one call" })),
  start: Type.Optional(Type.Integer()),
  end: Type.Optional(Type.Integer()),
  contextLines: Type.Optional(Type.Integer()),
  maxChars: Type.Optional(Type.Integer()),
}, { additionalProperties: false });

const codemodeParameters = Type.Object({
  code: Type.String({
    minLength: 1,
    maxLength: 32_000,
    description: "JavaScript: async () => { ... } or a bare body with return.",
  }),
  timeoutMs: Type.Optional(Type.Integer({ description: "Timeout ms (default 30000)" })),
}, { additionalProperties: false });

type SearchMode = "natural" | "pattern" | "defs" | "callers" | "chain" | "semantic" | "word" | "literal" | "regex" | "imports";

function queryForMode(query: string, mode: SearchMode): string {
  if (mode === "pattern" || mode === "defs" || mode === "callers" || mode === "word" || mode === "literal" || mode === "regex" || mode === "imports") {
    return `${mode}: ${query}`;
  }
  return query;
}

function searchArgs(params: {
  query: string;
  mode?: SearchMode;
  limit?: number;
  excerptLines?: number;
  in?: string;
  lang?: string;
  fileFilter?: string;
}): string[] {
  const mode = params.mode ?? "natural";
  const query = queryForMode(scopedSearchQuery(params), mode);
  const output = withSearchLang(
    ["--json", "--format", "agent-capsule", "--limit", String(params.limit ?? DEFAULT_LIMIT), "--excerpt-lines", String(params.excerptLines ?? 0)],
    params.lang,
  );
  return mode === "chain" || mode === "semantic"
    ? [mode, query, ".", ...output]
    : [...output, query, "."];
}

function scopedSearchQuery(params: { query: string; in?: string; fileFilter?: string }): string {
  return applyQueryScope(params.query, {
    ...(typeof params.in === "string" ? { in: params.in } : {}),
    ...(typeof params.fileFilter === "string" ? { fileFilter: params.fileFilter } : {}),
  }) ?? params.query;
}

function withSearchLang(argv: string[], lang: string | undefined): string[] {
  const trimmed = lang?.trim();
  return trimmed ? ["--lang", trimmed, ...argv] : argv;
}

/**
 * pi ships `read`, `edit`, `write`, `bash`, `grep`, `find`, `ls` built in, so on
 * a normal Pi host our one-shot file tools would be paid for twice and never
 * needed. They stay REGISTERED — an MCP-style host, a `--no-builtin-tools`
 * session, or a host that drops the built-ins still gets them — but they are
 * left out of the active set when the host already provides read+edit. Pi only
 * sends ACTIVE tools (schema, snippet, guidelines) to the model, so this is the
 * difference between ~271 tokens per request and nothing.
 *
 * ASGREP_KEEP_FILE_TOOLS=1 pins them active regardless.
 */
/**
 * Tools that MUTATE the index never ride the warm session: its calls are
 * serialized, so a write there blocks every read queued behind it.
 *
 * Exported so the routing contract is testable without a live session.
 */
export function writesOffSession(tool: string): boolean {
  return tool === "index_repo";
}

export function hostProvidesFileTools(pi: ExtensionAPI, env: NodeJS.ProcessEnv = process.env): boolean {
  if (env.ASGREP_KEEP_FILE_TOOLS === "1") return false;
  const api = pi as unknown as { getActiveTools?: () => string[] };
  try {
    if (typeof api.getActiveTools !== "function") return false;
    // Active by name, whatever supplies it: pi's built-ins, a wrapped host tool,
    // or another extension. Our own tools are named asgrep_read/asgrep_edit, so
    // this can only be somebody else's reader/editor.
    const active = api.getActiveTools();
    return active.includes("read") && active.includes("edit");
  } catch {
    return false;
  }
}

/** Drop our file tools from the active set; capability stays registered. */
function deactivateRedundantFileTools(pi: ExtensionAPI): void {
  const api = pi as unknown as { getActiveTools?: () => string[]; setActiveTools?: (names: string[]) => void };
  try {
    if (typeof api.getActiveTools !== "function" || typeof api.setActiveTools !== "function") return;
    const active = api.getActiveTools();
    const redundant = new Set(["asgrep_read", "asgrep_edit"]);
    const next = active.filter((name) => !redundant.has(name));
    if (next.length !== active.length) api.setActiveTools(next);
  } catch {
    // A host without tool-set control keeps today's behaviour.
  }
}

export function registerAstSgrepTools(
  pi: ExtensionAPI,
  runtime: RuntimeLike = new AstSgrepRuntime(pi),
  freshness: FreshnessLike = runtime instanceof AstSgrepRuntime
    ? new FreshnessCoordinator({ refreshIntervalMs: runtime.config.refreshIntervalMs! })
    : new FreshnessCoordinator(),
): void {
  const pool = new NativeSessionPool();
  let poolConfigured = false;
  // Prefer a registration-local pool so tests / multi-agent hosts do not share
  // sticky state. sharedNativePool remains for advanced single-session reuse.

  const ensurePool = (): void => {
    if (poolConfigured) return;
    try {
      const env = runtime.nativeEnv?.() ?? { NO_COLOR: "1" };
      let binary: string | undefined;
      try {
        binary = runtime.resolveBinaryPath?.({ env });
      } catch {
        binary = undefined;
      }
      const opts: {
        binary?: string;
        env: NodeJS.ProcessEnv;
        timeoutMs?: number;
        maxOutputBytes?: number;
        useEmbed?: boolean;
        indexPath?: string;
      } = { env };
      if (binary) opts.binary = binary;
      if (runtime.config?.timeoutMs !== undefined) opts.timeoutMs = runtime.config.timeoutMs;
      if (runtime.config?.maxOutputBytes !== undefined) opts.maxOutputBytes = runtime.config.maxOutputBytes;
      if (typeof env.ASGREP_NO_EMBED === "string") {
        opts.useEmbed = env.ASGREP_NO_EMBED !== "1" && env.ASGREP_NO_EMBED !== "true";
      }
      if (typeof env.ASGREP_INDEX_PATH === "string") opts.indexPath = env.ASGREP_INDEX_PATH;
      pool.configure(opts);
    } catch {
      pool.configure({});
    }
    poolConfigured = true;
  };

  /**
   * Context for a follow-up call at an already-resolved root. The marker keeps
   * a configured root from being re-applied against it: that re-resolution is
   * how a subdirectory anchor silently turned back into the subdirectory.
   */
  const rootedAt = (root: string): { cwd: string; [RESOLVED_ROOT]: true } => ({ cwd: root, [RESOLVED_ROOT]: true });

  /**
   * Cold checkout: build the index in the background at session start.
   *
   * Returns immediately when the index file already exists (the common case),
   * so a warm session pays one stat while a cold one gets its first search
   * answered from a warm index instead of waiting behind the build.
   */
  const warmColdIndex = async (root: string): Promise<void> => {
    // Opt out on very large checkouts where the startup walk is not worth it:
    // ASGREP_NO_WARM_INDEX=1.
    if ((runtime.nativeEnv?.() ?? {}).ASGREP_NO_WARM_INDEX === "1") return;
    const indexPathFor = runtime.resolveIndexPath;
    if (typeof indexPathFor !== "function") return;
    try {
      if (existsSync(indexPathFor.call(runtime, root))) return;
    } catch {
      return;
    }
    await runCli(["index", ".", "--json", "--no-embed"], rootedAt(root));
  };

  const resolveRoot = async (context: { cwd: string }): Promise<string> =>
    runtime.resolveRoot ? await runtime.resolveRoot(context) : context.cwd;

  /**
   * One index per checkout. Pi hands us the session cwd; when that cwd sits
   * inside a checkout that already owns an index, that index serves it —
   * scoped to the cwd — instead of a second multi-hundred-MB `.asgrep` growing
   * beside it. An explicit ASGREP_INDEX_PATH already shares one index across
   * every root, so it is left alone.
   */
  const anchorRoot = async (cwd: string): Promise<{ root: string; scope?: string }> => {
    const root = await resolveRoot({ cwd });
    const resolveIndexPath = runtime.resolveIndexPath;
    if (typeof resolveIndexPath !== "function") return { root };
    const env = runtime.nativeEnv?.() ?? {};
    const configured = env.ASGREP_INDEX_PATH;
    if (typeof configured === "string" && configured !== "") return { root };
    const indexAt = (dir: string): boolean => {
      try {
        return existsSync(resolveIndexPath.call(runtime, dir));
      } catch {
        return false;
      }
    };
    if (indexAt(root)) return { root };
    // Scope the walk to this checkout: an index that merely lives above the git
    // work tree root (a home directory, a shared scratch tree) belongs to no
    // project here and must not capture this session's searches.
    let workTree: string | undefined;
    for (let dir = root;; dir = dirname(dir)) {
      if (existsSync(join(dir, ".git"))) { workTree = dir; break; }
      const parent = dirname(dir);
      if (dir === parent) break;
    }
    const within = (dir: string): boolean => workTree === undefined || dir === workTree || dir.startsWith(workTree + sep);
    for (let dir = dirname(root);; dir = dirname(dir)) {
      const parent = dirname(dir);
      if (dir === parent) break;
      if (!within(dir)) break;
      if (!indexAt(dir)) continue;
      const scope = relative(dir, root).split(sep).join("/");
      return scope !== "" && scope !== "." ? { root: dir, scope } : { root: dir };
    }
    // Nothing indexed in this checkout yet: the index belongs at its root, not
    // in whichever subdirectory this session happens to sit in.
    if (workTree !== undefined && workTree !== root) {
      const scope = relative(workTree, root).split(sep).join("/");
      return scope !== "" && scope !== "." ? { root: workTree, scope } : { root: workTree };
    }
    return { root };
  };

  /**
   * Closed backend availability after sticky acquire fails.
   * Parallel `{napi,cli}` bools made `{napi:true,cli:true}` and both-false+free-cause
   * separate states; runtime only needs one live backend or a single unavailable variant.
   */
  type BackendAvailability =
    | { kind: "napi" }
    | { kind: "cli" }
    | { kind: "unavailable"; cause?: string };

  const probeCli = (options: RunOptions = {}): BackendAvailability => {
    // Test fixtures inject `run` without a resolver; production always has resolveBinaryPath.
    if (typeof runtime.resolveBinaryPath !== "function") return { kind: "cli" };
    try {
      const base = runtime.nativeEnv?.() ?? {};
      if (options.env) {
        runtime.resolveBinaryPath({ env: { ...base, ...options.env } });
      } else {
        runtime.resolveBinaryPath({ env: base });
      }
      return { kind: "cli" };
    } catch (cause) {
      return {
        kind: "unavailable",
        cause: cause instanceof Error ? cause.message : String(cause),
      };
    }
  };

  const requireBackend = (availability: BackendAvailability, context: { cwd: string }): void => {
    if (availability.kind !== "unavailable") return;
    throw new RuntimeError(
      "BACKEND_UNAVAILABLE",
      `ast-sgrep backend unavailable (no NAPI session and no CLI binary)${availability.cause ? `: ${availability.cause}` : ""}. Run /asgrep-doctor for recovery details.`,
      {
        backend: "unavailable",
        // Agent-facing mirrors of the closed unavailable variant (not an open product).
        napi: false,
        cli: false,
        cwd: context.cwd,
        hint: "Install @ast-sgrep/<platform> or run npm run build:native in packages/pi/extension",
        ...(availability.cause ? { cause: availability.cause } : {}),
      },
    );
  };

  const runCli = async (
    args: readonly string[],
    context: { cwd: string },
    options: RunOptions = {},
  ): Promise<MachineEnvelope> => {
    requireBackend(probeCli(options), context);
    return runtime.run(args, context, options);
  };

  /** Typed sticky call that degrades to null when no backend is available or
   * the session is crash-looping — the caller's CLI fallback owns the error
   * fidelity when the binary itself is also broken. pool.call owns the
   * respawn-on-closed retry; we only translate its outcomes. */
  const callSticky = async (
    root: string,
    tool: string,
    args: Record<string, unknown>,
    options: RunOptions = {},
  ): Promise<MachineEnvelope | null> => {
    try {
      return await pool.call(root, tool, args, options.signal ? { signal: options.signal } : {});
    } catch (cause) {
      // Aborted: return null so the CLI fallback surfaces the cancellation with
      // its own semantics (runCli -> runtime.run maps abort to CANCELLED).
      if (options.signal?.aborted) return null;
      const message = cause instanceof Error ? cause.message : String(cause);
      if (isClosedWorkerError(cause) || message.includes("backend unavailable")) return null;
      throw cause;
    }
  };

  const nativeCall = async (
    tool: string,
    args: Record<string, unknown>,
    context: { cwd: string },
    options: RunOptions = {},
  ): Promise<MachineEnvelope> => {
    ensurePool();
    const root = await resolveRoot(context);
    // Writes never ride the warm session. Its calls are serialized, so an index
    // running there blocks every read queued behind it (measured p100: a search
    // waited 9.2s for a background reindex). Index work goes out of process;
    // SQLite WAL lets readers keep their own snapshot meanwhile.
    if (writesOffSession(tool)) return runCli(argvFor(tool, args), context, options);
    const sticky = await callSticky(root, tool, args, options);
    if (sticky) return asEnvelope(sticky);
    // Cold CLI only when a real binary resolves -- never remap missing natives to BINARY_RESOLUTION_FAILED.
    return runCli(argvFor(tool, args), context, options);
  };

  // Freshness + tools share the same warm in-process Searcher as Code Mode.
  const warmRuntime: FreshnessRuntime = {
    run: (args, context, options) => runtime.run(args, context, options),
    resolveRoot: (context) => resolveRoot(context),
    nativeCall,
  };
  // Optional runtime capabilities pass through when present — bound, since
  // they are class methods whose private fields live on the runtime instance.
  for (const key of ["watchExternalChanges", "resolveIndexPath", "inspectIndexCompatibility"] as const) {
    const member = runtime[key];
    if (member !== undefined) {
      (warmRuntime as unknown as Record<string, unknown>)[key] =
        typeof member === "function" ? member.bind(runtime) : member;
    }
  }
  if (runtime.rebuildIncompatibleIndex) {
    warmRuntime.rebuildIncompatibleIndex = async (context, options) => {
      const root = await resolveRoot(context);
      await pool.invalidate(root);
      return runtime.rebuildIncompatibleIndex!(context, options);
    };
  }

  /**
   * Freshness gate shared by the one-shot tools: ensureFresh with a bounded
   * timeout fallback, or a scoped-path index when the query carries in:/fileFilter.
   *
   * Bounded means serve-stale, not fail: a caller that ran out of freshness
   * budget still queries the current index and is told the result may be stale.
   * The session is never torn down here — the shared refresh runs on it, so
   * invalidating would kill the index work the caller just stopped waiting for
   * and leave the root permanently stale.
   */
  const freshRoot = async (
    cwd: string,
    signal: AbortSignal | undefined,
    scopedPath?: string,
  ): Promise<{ root: string; scope?: string; freshness?: "stale" }> => {
    const options = signal ? { signal } : {};
    const anchor = await anchorRoot(cwd);
    const scope = anchor.scope;
    // A subtree refresh still lands in the checkout's own index.
    const target = scopedPath ? (scope ? `${scope}/${scopedPath}` : scopedPath) : undefined;
    if (target) {
      try {
        await nativeCall("index_repo", { paths: [target] }, rootedAt(anchor.root), options);
      } catch (cause) {
        if (!isFreshnessTimeout(cause, signal)) throw cause;
        return { root: anchor.root, ...(scope ? { scope } : {}), freshness: "stale" as const };
      }
      return { root: anchor.root, ...(scope ? { scope } : {}) };
    }
    try {
      const resolved = await freshness.ensureFresh(warmRuntime, rootedAt(anchor.root), options);
      // The contract is a root string; a host/test double that returns nothing
      // must not hand an undefined cwd to the runtime.
      const root = typeof resolved === "string" && resolved !== "" ? resolved : anchor.root;
      return { root, ...(scope ? { scope } : {}) };
    } catch (cause) {
      if (!isFreshnessTimeout(cause, signal)) throw cause;
      return { root: anchor.root, ...(scope ? { scope } : {}), freshness: "stale" as const };
    }
  };

  /** Anchor the caller's own in:/fileFilter scope under the checkout root. */
  const withAnchorScope = <T extends { in?: string; fileFilter?: string }>(params: T, scope: string | undefined): T => {
    if (!scope) return params;
    const nested = params.in ?? params.fileFilter;
    const combined = typeof nested === "string" && nested.trim() ? `${scope}/${nested.replace(/^(?:\.\/)+/u, "")}` : scope;
    return { ...params, in: combined };
  };

  /** An index with no files is not a no-match: it answers nothing at all. */
  const probeIndexState = async (
    root: string,
    context: { cwd: string },
    options: RunOptions,
  ): Promise<{ files: number; semanticChunks?: number } | undefined> => {
    try {
      const status = await machineCall(root, "index_status", {}, ["status", ".", "--json"], context, options);
      if (typeof status.file_count !== "number") return undefined;
      const probe: { files: number; semanticChunks?: number } = { files: status.file_count };
      if (typeof status.semantic_chunk_count === "number") probe.semanticChunks = status.semantic_chunk_count;
      return probe;
    } catch {
      // Coverage is a note on an answer, never a failure of its own.
      return undefined;
    }
  };

  const zeroHitResponse = (response: { hits?: unknown; ok?: unknown }): boolean =>
    response.ok !== false && Array.isArray(response.hits) && response.hits.length === 0;

  /** Typed sticky call first, argv fallback when no session — the shape every
   * one-shot tool shares. */
  const machineCall = async (
    root: string,
    tool: string,
    stickyArgs: Record<string, unknown>,
    argv: string[],
    context: { cwd: string },
    options: RunOptions,
  ): Promise<MachineEnvelope> =>
    (await callSticky(root, tool, stickyArgs, options)) ?? await runCli(argv, context, options);

  /** Native launch env + resolved binary for the codemode batch host. */
  const nativeLaunch = (): { env: NodeJS.ProcessEnv; binary: string | null } => {
    const env = runtime.nativeEnv?.() ?? { NO_COLOR: "1" };
    let binary: string | null = null;
    try {
      binary = runtime.resolveBinaryPath?.({ env }) ?? null;
    } catch {
      binary = null;
    }
    return { env, binary };
  };

  /** Host surface for codemode: argv run + optional sticky + stdin batch. */
  const buildBatchHost = (sticky: StickyWorker | null, env: NodeJS.ProcessEnv, binary: string | null) => {
    const host: {
      run: (args: readonly string[], context: { cwd: string }, runOptions?: { signal?: AbortSignal }) => Promise<MachineEnvelope>;
      sticky: StickyWorker | null;
      runBatch?: (
        calls: Array<{ id: string; tool: string; args: Record<string, unknown> }>,
        context: { cwd: string },
        runOptions?: { signal?: AbortSignal },
      ) => ReturnType<typeof runNativeBatch>;
    } = {
      run: (args, context, runOptions) => runtime.run(args, context, runOptions ?? {}),
      sticky,
    };
    if (binary) {
      host.runBatch = (calls, context, runOptions) =>
        runNativeBatch(
          (a, c, o) => runtime.run(a, c, o ?? {}),
          calls,
          context,
          runOptions,
          (body, c, o) => {
            const stdinOpts: Parameters<typeof runBatchViaStdin>[0] = { binary, cwd: c.cwd, body, env };
            if (o?.signal) stdinOpts.signal = o.signal;
            if (runtime.config?.timeoutMs !== undefined) stdinOpts.timeoutMs = runtime.config.timeoutMs;
            if (runtime.config?.maxOutputBytes !== undefined) stdinOpts.maxOutputBytes = runtime.config.maxOutputBytes;
            return runBatchViaStdin(stdinOpts);
          },
        );
    }
    return host;
  };

  let stopWorkspaceEvents: (() => void) | undefined;
  function watchWorkspaceChanges(): void {
    stopWorkspaceEvents ??= pi.events?.on("workspace:changed", (data: unknown) => {
      if (!data || typeof data !== "object") return;
      const event = data as { version?: unknown; cwd?: unknown; paths?: unknown };
      if (event.version !== 1 || typeof event.cwd !== "string" || !isAbsolute(event.cwd)) return;
      if (event.paths === null) {
        freshness.markRootDirty?.(event.cwd);
      } else if (Array.isArray(event.paths) && event.paths.every(p => typeof p === "string" && isAbsolute(p))) {
        for (const file of event.paths) freshness.markAffectedPath(file, event.cwd);
      }
    });
  }
  watchWorkspaceChanges();

  pi.on("tool_result", (event, ctx) => {
    if (event.isError) return;
    if (event.toolName !== "write" && event.toolName !== "edit") return;
    const path = event.input.path;
    if (typeof path === "string") freshness.markAffectedPath(path, ctx.cwd);
  });
  pi.on("session_start", (_event, ctx) => {
    watchWorkspaceChanges();
    // Built-in read/edit present and active: keep ours registered (other hosts
    // need them) but off the model's tool list.
    if (hostProvidesFileTools(pi)) deactivateRedundantFileTools(pi);
    // Warm the in-process Searcher at session start so the first asgrep
    // search does not pay NAPI/SQLite open on the user's first lookup.
    void (async () => {
      try {
        ensurePool();
        const root = await resolveRoot({ cwd: ctx.cwd });
        await Promise.all([pool.acquire(root), warmCodemodeSandbox()]);
        // Cold checkout: build the index now, in the background, out of process.
        // Session start (system prompt, first model turn) is a second or two of
        // free time, and a lexical/AST index of a few thousand files takes a few
        // hundred ms — so the first search answers from a warm index instead of
        // waiting behind a build (measured cold first search: 271ms and rising
        // with repo size). Failures stay silent: the search path owns recovery.
        await warmColdIndex(root);
      } catch {
        // Doctor reports backend errors; a failed warmup must not block the session.
      } finally {
        const warning = runtime.binaryWarning?.();
        if (warning && ctx.hasUI) ctx.ui.notify(warning, "warning");
      }
    })();
  });
  pi.on("session_shutdown", () => {
    stopWorkspaceEvents?.();
    stopWorkspaceEvents = undefined;
    freshness.shutdown?.();
    void pool.shutdown();
    void resetCodemodeSandboxForTests();
  });

  // Primary surface: Code Mode -- in-process NAPI (MCP-class), compose in JS.
  // Sibling to MCP: pick one surface; both link core, never each other.
  pi.registerTool({
    name: "asgrep",
    label: "asgrep",
    promptSnippet: ASGREP_PROMPT_SNIPPET,
    promptGuidelines: [...(hostProvidesFileTools(pi) ? ASGREP_PROMPT_GUIDELINES_HOST_FILES : ASGREP_PROMPT_GUIDELINES)],
    description: [
      "Code search: use it for any code lookup instead of grep.",
      "The returned value is your result.",
      CODEMODE_TYPES_FOR_MODEL,
      "Example: async () => (await asgrep.search(\"auth\", { limit: 5 })).hits",
    ].join("\n"),
    parameters: codemodeParameters,
    // The card owns its own frame: no host Box padding/background around it,
    // and no duplicate title line above it.
    renderShell: "self",
    renderCall() {
      return EMPTY_CALL;
    },
    renderResult(result, options, theme, context) {
      return renderAsgrepResult(result, options, theme, context);
    },
    async execute(_toolCallId, params, signal, onUpdate, ctx) {
      report(onUpdate, "codemode", "started");
      try {
        const timeoutMs = typeof params.timeoutMs === "number"
          ? params.timeoutMs
          : runtime.config?.timeoutMs ?? 30_000;
        const deadline = Date.now() + timeoutMs;
        const timeoutSignal = AbortSignal.timeout(timeoutMs);
        const operationSignal = signal
          ? AbortSignal.any([signal, timeoutSignal])
          : timeoutSignal;
        const options = { signal: operationSignal };
        ensurePool();
        const { root, scope, freshness: fresh } = await freshRoot(ctx.cwd, signal);
        const { env, binary } = nativeLaunch();
        // In-process NAPI first; CLI sticky only if addon missing.
        const sticky: StickyWorker | null = await pool.acquire(root);
        const bundle = createAsgrepConnector(buildBatchHost(sticky, env, binary), rootedAt(root), { ...options, ...(scope ? { scope } : {}) });
        bundle.resetStats();
        const codemodeOptions: {
          stats: () => ReturnType<typeof bundle.stats>;
          timeoutMs?: number;
          signal?: AbortSignal;
        } = { stats: bundle.stats };
        codemodeOptions.timeoutMs = Math.max(1, deadline - Date.now());
        codemodeOptions.signal = operationSignal;
        await warmCodemodeSandbox().catch(() => undefined);
        const outcome = await runCodemode(params.code, bundle.asgrep, codemodeOptions);
        report(onUpdate, "codemode", "completed");
        if (!outcome.ok) {
          return {
            content: [{ type: "text" as const, text: bounded(`codemode failed: ${outcome.error}`) }],
            details: {
              ok: false,
              command: "codemode",
              error: { code: "CODEMODE_ERROR", message: outcome.error, details: { logs: outcome.logs, stats: outcome.stats } },
              code: outcome.code,
              stats: outcome.stats,
              trace: bundle.trace(),
              wallMs: outcome.wallMs,
              backend: pool.backend(),
            },
          };
        }
        const rendered = formatCodemodeResult(outcome.result, {
          ...(outcome.stats ? { stats: outcome.stats } : {}),
          wallMs: outcome.wallMs,
          backend: pool.backend(),
        });
        const activationMs = outcome.wallMs;
        return {
          content: [{ type: "text" as const, text: bounded(rendered) }],
          details: {
            ok: true,
            command: "codemode",
            result: outcome.result,
            logs: outcome.logs,
            rendered,
            stats: outcome.stats,
            trace: bundle.trace(),
            wallMs: outcome.wallMs,
            activationMs,
            backend: pool.backend(),
            ...(fresh ? { freshness: fresh } : {}),
          },
        };
      } catch (cause) {
        return failure("codemode", cause, signal);
      }
    },
  });

  // Escape hatches: one-shot tools for simple lookups. Prefer asgrep.
  // They ride the same session sticky pool when available (no cold spawn).
  pi.registerTool({
    name: "asgrep_search",
    label: "asgrep search",
    promptSnippet: "One-shot search",
    description: "One-shot search. Use asgrep (Code Mode) for anything multi-step, parallel, or filtered.",
    parameters: searchParameters,
    renderShell: "self",
    renderCall() {
      return EMPTY_CALL;
    },
    renderResult(result, options, theme, context) {
      return renderAsgrepResult(result, options, theme, context);
    },
    async execute(_toolCallId, params, signal, onUpdate, ctx) {
      const options = signal ? { signal } : {};
      const started = performance.now();
      report(onUpdate, "search", "started");
      try {
        ensurePool();
        const scopedPath = (typeof params.in === "string" ? params.in : undefined)
          ?? extractInPath(params.query);
        const fresh = await freshRoot(ctx.cwd, signal, scopedPath);
        // The checkout owns the index; the caller's scope rides under it.
        const anchored = withAnchorScope(params, fresh.scope);
        const [tool, args] = searchToolCall(anchored);
        const response = await machineCall(fresh.root, tool, args, searchArgs(anchored), rootedAt(fresh.root), options);
        const notes: string[] = [];
        if (fresh.freshness === "stale") {
          notes.push("index refresh is still running; this answer came from the current index and may be stale");
        }
        let indexState: "empty" | "ready" | undefined;
        if (zeroHitResponse(response)) {
          const probe = await probeIndexState(fresh.root, rootedAt(fresh.root), options);
          if (probe) {
            indexState = probe.files === 0 ? "empty" : "ready";
            if (indexState === "empty") {
              notes.push("index has 0 files: this repository is not indexed -- run /asgrep-index (or asgrep.indexRepo()) and retry");
            } else if ((params.mode ?? "natural") === "semantic" && probe.semanticChunks === 0) {
              // Freshness refreshes index lexical/AST only, so a semantic query
              // on a cold repo has nothing to rank yet.
              notes.push("no embeddings yet: this index was built lexical-only -- run /asgrep-index (or asgrep.indexRepo()) to build vectors");
            }
          }
        }
        report(onUpdate, "search", "completed");
        return success("search", response, {
          query: params.query,
          mode: params.mode ?? "natural",
          activationMs: performance.now() - started,
          backend: pool.backend(),
          // Drives excerpt rendering in the model-facing text: capsules carry
          // body text whether or not it was asked for.
          excerptLines: params.excerptLines ?? 0,
          ...(fresh.freshness ? { freshness: fresh.freshness } : {}),
          ...(indexState ? { indexState } : {}),
          ...(notes.length > 0 ? { notes } : {}),
        });
      } catch (cause) {
        return failure("search", cause, signal);
      }
    },
  });

  // Trained-priorty escape hatches: direct edit/read without writing JS.
  // Both ride the connector's arg plumbing and the same sticky pool.
  pi.registerTool({
    name: "asgrep_edit",
    label: "asgrep edit",
    promptSnippet: "Exact-string edit",
    description: "Edit by exact-string replace. edits[] applies many atomically.",
    parameters: editParameters,
    renderShell: "self",
    renderCall() {
      return EMPTY_CALL;
    },
    renderResult(result, options, theme, context) {
      return renderAsgrepResult(result, options, theme, context);
    },
    async execute(_toolCallId, params, signal, onUpdate, ctx) {
      report(onUpdate, "edit", "started");
      try {
        ensurePool();
        const options = signal ? { signal } : {};
        const { root, scope, freshness: fresh } = await freshRoot(ctx.cwd, signal);
        const sticky = await pool.acquire(root);
        const bundle = createAsgrepConnector({ run: (a, c, o) => runtime.run(a, c, o), sticky }, rootedAt(root), { ...options, ...(scope ? { scope } : {}) });
        const response = await bundle.asgrep.edit(params);
        report(onUpdate, "edit", "completed");
        return success("edit", response as MachineEnvelope, { backend: pool.backend(), ...(fresh ? { freshness: fresh } : {}) });
      } catch (cause) {
        return failure("edit", cause, signal);
      }
    },
  });

  pi.registerTool({
    name: "asgrep_read",
    label: "asgrep read",
    promptSnippet: "Read file window or hit ref",
    description: "Read a file window or resolve hit refs (path#L1-L40).",
    parameters: readParameters,
    renderShell: "self",
    renderCall() {
      return EMPTY_CALL;
    },
    renderResult(result, options, theme, context) {
      return renderAsgrepResult(result, options, theme, context);
    },
    async execute(_toolCallId, params, signal, onUpdate, ctx) {
      report(onUpdate, "read", "started");
      try {
        // Reading source must remain available when the binary or index is broken.
        const options = signal ? { signal } : {};
        const { root, scope } = await anchorRoot(ctx.cwd);
        const bundle = createAsgrepConnector({ run: (a, c, o) => runtime.run(a, c, o) }, rootedAt(root), { ...options, localReads: true, ...(scope ? { scope } : {}) });
        const response = await bundle.asgrep.read(params);
        report(onUpdate, "read", "completed");
        return success("read", response as MachineEnvelope, { backend: "filesystem" });
      } catch (cause) {
        return failure("read", cause, signal);
      }
    },
  });

  pi.registerTool({
    name: "asgrep_index",
    label: "asgrep index",
    promptSnippet: "Build or rebuild the index",
    description: "Build or rebuild the index.",
    parameters: indexParameters,
    renderShell: "self",
    renderCall() {
      return EMPTY_CALL;
    },
    renderResult(result, options, theme, context) {
      return renderAsgrepResult(result, options, theme, context);
    },
    async execute(_toolCallId, params, signal, onUpdate, ctx) {
      const force = params.force === true;
      const command = force ? "reindex" : "index";
      report(onUpdate, command, "started");
      try {
        ensurePool();
        const options = signal ? { signal } : {};
        const { root, freshness: fresh } = await freshRoot(ctx.cwd, signal);
        // Always out of process: an index inside the warm session would block
        // every read queued behind it (measured 9.2s p100 for a search during a
        // reindex). The explicit path keeps embeddings; implicit refreshes skip
        // them (see runtime/freshness.ts).
        const response = await runCli([command, ".", "--json"], rootedAt(root), options);
        report(onUpdate, command, "completed");
        return success(command, response, { ...(fresh ? { freshness: fresh } : {}) });
      } catch (cause) {
        return failure(command, cause, signal);
      }
    },
  });

  // No asgrep_status tool: index/runtime status is a diagnostic, not a lookup.
  // The model reads it in Code Mode (asgrep.indexStatus()) and humans have
  // /asgrep-status, so the schema does not carry it on every request.
}

/** Map one-shot search params to typed sticky tool+args. Data-driven mode table. */

type SearchCallSpec =
  | { tool: "semantic" }
  | { tool: "chain" }
  | { tool: "defs" | "callers" | "imports"; key: "symbol" | "module" }
  | { tool: "search"; prefix?: string };

const SEARCH_CALL_SPEC: { [M in SearchMode]: SearchCallSpec } = {
  semantic: { tool: "semantic" },
  chain: { tool: "chain" },
  defs: { tool: "defs", key: "symbol" },
  callers: { tool: "callers", key: "symbol" },
  imports: { tool: "imports", key: "module" },
  pattern: { tool: "search", prefix: "pattern" },
  word: { tool: "search", prefix: "word" },
  literal: { tool: "search", prefix: "literal" },
  regex: { tool: "search", prefix: "regex" },
  natural: { tool: "search" },
};

function searchToolCall(params: {
  query: string;
  mode?: SearchMode;
  limit?: number;
  excerptLines?: number;
  in?: string;
  lang?: string;
  fileFilter?: string;
}): [string, Record<string, unknown>] {
  const mode: SearchMode = params.mode ?? "natural";
  const limit = params.limit ?? DEFAULT_LIMIT;
  const excerpt_lines = params.excerptLines ?? 0;
  const query = scopedSearchQuery(params);
  const spec = SEARCH_CALL_SPEC[mode];
  const lang = typeof params.lang === "string" ? params.lang.trim() : "";
  if (spec.tool === "semantic") {
    return ["semantic", { query, limit, excerpt_lines, format: "capsule", ...(lang ? { lang } : {}) }];
  }
  if (spec.tool === "chain") {
    return ["chain", { query, limit, top_n: 20 }];
  }
  if (spec.tool === "search") {
    const prefixed = spec.prefix ? `${spec.prefix}: ${query}` : query;
    return ["search", { query: prefixed, limit, excerpt_lines, format: "capsule", ...(lang ? { lang } : {}) }];
  }
  return [spec.tool, { [spec.key]: params.query, limit, excerpt_lines, ...(lang ? { lang } : {}) }];
}
