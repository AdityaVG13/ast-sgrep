/**
 * Index freshness coordination: dirty tracking, shared root-owned refreshes,
 * bounded per-caller waits, filesystem watchers.
 */
import { existsSync, realpathSync, statSync, watch, type FSWatcher } from "node:fs";
import { basename, dirname, isAbsolute, join, resolve } from "node:path";
import {
  DEFAULT_FRESHNESS_WAIT_MS,
  DEFAULT_REFRESH_INTERVAL_MS,
  RuntimeError,
  type InternalRuntimeContext,
  type MachineEnvelope,
  RESOLVED_ROOT,
  type RunOptions,
  type RuntimeContext,
} from "./types.js";
import { finitePositive } from "./config.js";
import {
  indexCompletion,
  incompatibleStatusFailure,
  indexHealth,
  pathContained,
  type IndexHealth,
} from "./index-health.js";

export { type IndexHealth } from "./index-health.js";

const MAX_TARGETED_INDEX_PATHS = 1_024;

export interface FreshnessRuntime {
  run(args: readonly string[], context: RuntimeContext, options?: RunOptions): Promise<MachineEnvelope>;
  resolveRoot(context: RuntimeContext): Promise<string>;
  inspectIndexCompatibility?(context: RuntimeContext): Promise<IndexHealth>;
  rebuildIncompatibleIndex?(context: RuntimeContext, options?: RunOptions): Promise<MachineEnvelope>;
  /** Absolute database path whose SQLite/derived writes are owned by this runtime. */
  resolveIndexPath?(root: string): string;
  /**
   * Optional warm native call (session sticky pool). When present, freshness
   * prefers this over cold `run` for status/index — same Searcher as Code Mode.
   */
  nativeCall?(
    tool: string,
    args: Record<string, unknown>,
    context: RuntimeContext,
    options?: RunOptions,
  ): Promise<MachineEnvelope>;
  /** Enable low-latency external filesystem change detection for real runtimes. */
  watchExternalChanges?: boolean;
}

export interface FreshnessCoordinatorOptions {
  refreshIntervalMs?: number;
  /** Cap on how long one caller waits for an in-flight refresh (serve-stale after). */
  maxWaitMs?: number;
  now?: () => number;
  watchFactory?: FreshnessWatchFactory;
}

export type FreshnessWatchFactory = (
  root: string,
  options: { recursive: true; persistent: false; encoding: "utf8" },
  listener: (eventType: "rename" | "change", filename: string | null) => void,
) => FSWatcher;

type RootFreshness = {
  dirtyGeneration: number;
  cleanGeneration: number;
  dirtyPaths: Set<string>;
  fullScanRequired: boolean;
  initialized: boolean;
  lastRefreshAt: number;
  inFlight: Promise<void> | undefined;
  refreshAbort: AbortController | undefined;
  waiterCount: number;
  /** undefined = untried/retry, null = unsupported for this root. */
  watcher: FSWatcher | null | undefined;
};
type PendingFreshness = {
  paths: Set<string>;
  fullScanRequired: boolean;
  consumedFullScanRoots: Set<string>;
};

/** Probe compatibility hook then status; map incompat operational failures to health. */
async function probeIndexHealth(
  runtime: FreshnessRuntime,
  rootContext: RuntimeContext,
  options: RunOptions,
): Promise<IndexHealth> {
  const hinted = await runtime.inspectIndexCompatibility?.(rootContext);
  if (hinted === "missing" || hinted === "incompatible") return hinted;
  try {
    const status = runtime.nativeCall
      ? await runtime.nativeCall("index_status", {}, rootContext, options)
      : await runtime.run(["status", ".", "--json"], rootContext, options);
    return indexHealth(status, hinted === "ready");
  } catch (cause) {
    if (!incompatibleStatusFailure(cause)) throw cause;
    return "incompatible";
  }
}

/** Run index_repo via native sticky pool or CLI argv. force=true → reindex. */
async function runIndex(
  runtime: FreshnessRuntime,
  force: boolean,
  rootContext: RuntimeContext,
  options: RunOptions,
): Promise<void> {
  const response = runtime.nativeCall
    ? await runtime.nativeCall("index_repo", { force }, rootContext, options)
    : await runtime.run([force ? "reindex" : "index", ".", "--json"], rootContext, options);
  const { failed, walkErrors } = indexCompletion(response, true);
  if (failed > 0 || walkErrors) {
    throw new RuntimeError(
      "INDEX_UPDATE_INCOMPLETE",
      "ast-sgrep did not complete the full index reconciliation",
      { failed, walkErrors, force },
    );
  }
}

/** Update known changed paths without walking the repository. */
async function runTargetedIndex(
  runtime: FreshnessRuntime,
  paths: string[],
  rootContext: RuntimeContext,
  options: RunOptions,
): Promise<void> {
  for (let offset = 0; offset < paths.length; offset += MAX_TARGETED_INDEX_PATHS) {
    const chunk = paths.slice(offset, offset + MAX_TARGETED_INDEX_PATHS);
    const response = runtime.nativeCall
      ? await runtime.nativeCall(
        "index_repo",
        { paths: chunk },
        rootContext,
        options,
      )
      : await runtime.run(
        ["index", ".", "--json", ...chunk.flatMap((path) => ["--path", path])],
        rootContext,
        options,
      );
    const { failed } = indexCompletion(response, false);
    if (failed > 0) {
      throw new RuntimeError(
        "INDEX_UPDATE_INCOMPLETE",
        `ast-sgrep failed to update ${failed} changed path${failed === 1 ? "" : "s"}`,
        { failed, pathCount: chunk.length },
      );
    }
  }
}

function canonicalizeAffectedPath(path: string): string {
  const absolute = resolve(path);
  const unresolved = [basename(absolute)];
  let existing = dirname(absolute);
  for (;;) {
    try {
      return resolve(realpathSync(existing), ...unresolved.reverse());
    } catch (cause) {
      const code = (cause as NodeJS.ErrnoException).code;
      const parent = dirname(existing);
      if ((code !== "ENOENT" && code !== "ENOTDIR") || parent === existing) return resolve(path);
      unresolved.push(basename(existing));
      existing = parent;
    }
  }
}

function canonicalizeRootPath(path: string): string {
  try {
    return realpathSync(resolve(path));
  } catch {
    return canonicalizeAffectedPath(path);
  }
}

function changesIgnoreRules(path: string): boolean {
  const name = basename(path);
  return name === ".gitignore" || name === ".ignore" || name === ".asgrepignore";
}

function ignoredIndexWrite(root: string, path: string, indexPath: string): boolean {
  const defaultIndexDirectory = join(root, ".asgrep");
  if (pathContained(defaultIndexDirectory, path)) return true;

  const indexDirectory = dirname(indexPath);
  if (dirname(path) !== indexDirectory) return false;
  const name = basename(path);
  const sqliteArtifact = (database: string) => {
    const suffix = name.slice(database.length);
    return name.startsWith(database) && (
      suffix === ""
      || suffix === "-wal"
      || suffix === "-shm"
      || suffix === "-journal"
      || suffix === ".reindex.lock"
      || /^\.corrupt(?:\.\d+)?(?:-(?:wal|shm|journal))?$/u.test(suffix)
    );
  };
  return sqliteArtifact(basename(indexPath))
    || sqliteArtifact("lexical.db")
    || name === "semantic.ivf"
    || (name.startsWith(".semantic.ivf.") && name.endsWith(".tmp"));
}

function existingDirectory(path: string): boolean {
  try {
    return statSync(path).isDirectory();
  } catch {
    return false;
  }
}

function markStatePathDirty(state: RootFreshness, path: string): void {
  state.dirtyGeneration += 1;
  if (changesIgnoreRules(path)) {
    state.dirtyPaths.clear();
    state.fullScanRequired = true;
  } else if (!state.fullScanRequired) {
    if (!state.dirtyPaths.has(path) && state.dirtyPaths.size >= MAX_TARGETED_INDEX_PATHS) {
      state.dirtyPaths.clear();
      state.fullScanRequired = true;
    } else {
      state.dirtyPaths.add(path);
    }
  }
}

function markStateFullScan(state: RootFreshness): void {
  state.dirtyGeneration += 1;
  state.dirtyPaths.clear();
  state.fullScanRequired = true;
}

function cancelledRefreshWait(): RuntimeError {
  return new RuntimeError("CANCELLED", "ast-sgrep freshness wait was cancelled");
}

/** Stop one caller waiting without transferring cancellation ownership to shared work. */
function waitForRefresh(refresh: Promise<void>, signal?: AbortSignal, waitMs?: number): Promise<void> {
  if (!signal && (!waitMs || waitMs <= 0)) return refresh;
  if (signal?.aborted) return Promise.reject(cancelledRefreshWait());
  return new Promise<void>((resolveWait, rejectWait) => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const cleanup = () => {
      signal?.removeEventListener("abort", onAbort);
      if (timer) clearTimeout(timer);
    };
    const onAbort = () => {
      cleanup();
      rejectWait(cancelledRefreshWait());
    };
    signal?.addEventListener("abort", onAbort, { once: true });
    // A caller never spends its whole budget on freshness: after waitMs it
    // serves whatever index state exists instead of dying on a 60s index.
    if (waitMs && waitMs > 0) {
      timer = setTimeout(() => {
        cleanup();
        rejectWait(new RuntimeError(
          "TIMEOUT",
          "ast-sgrep freshness wait exceeded " + waitMs + "ms; serving the current index",
          { timeoutMs: waitMs },
        ));
      }, waitMs);
    }
    refresh.then(
      () => {
        cleanup();
        resolveWait();
      },
      (cause) => {
        cleanup();
        rejectWait(cause);
      },
    );
  });
}

/** Shared refresh continues while other waiters remain; the last cancel stops it. */
function attachRefreshWaiter(state: RootFreshness, refresh: Promise<void>, signal?: AbortSignal, waitMs?: number): Promise<void> {
  state.waiterCount += 1;
  let cancelledByWaiter = false;
  const wait = waitForRefresh(refresh, signal, waitMs).catch((cause: unknown) => {
    cancelledByWaiter = cause instanceof RuntimeError && cause.code === "CANCELLED" && signal?.aborted === true;
    throw cause;
  });
  return wait.finally(() => {
    state.waiterCount = Math.max(0, state.waiterCount - 1);
    if (cancelledByWaiter && state.waiterCount === 0 && state.inFlight !== undefined) {
      state.refreshAbort?.abort();
    }
  });
}

export class FreshnessCoordinator {
  readonly #states = new Map<string, RootFreshness>();
  readonly #pending = new Map<string, PendingFreshness>();
  readonly #interval: number;
  readonly #maxWaitMs: number;
  readonly #now: () => number;
  readonly #watchFactory: FreshnessWatchFactory;

  constructor(options: FreshnessCoordinatorOptions = {}) {
    this.#interval = finitePositive(options.refreshIntervalMs, DEFAULT_REFRESH_INTERVAL_MS, "refreshIntervalMs");
    this.#maxWaitMs = finitePositive(options.maxWaitMs, DEFAULT_FRESHNESS_WAIT_MS, "maxWaitMs");
    this.#now = options.now ?? Date.now;
    this.#watchFactory = options.watchFactory ?? watch;
  }

  /** One caller's freshness budget: bounded, and never more than its own timeout. */
  #waitBudget(options: RunOptions): number {
    const budget = this.#maxWaitMs;
    return options.timeoutMs !== undefined ? Math.min(budget, options.timeoutMs) : budget;
  }

  markAffectedPath(path: string, cwd: string): void {
    const affected = canonicalizeAffectedPath(isAbsolute(path) ? path : resolve(canonicalizeAffectedPath(cwd), path));
    let matched = false;
    for (const [root, state] of this.#states) {
      if (!pathContained(root, affected)) continue;
      markStatePathDirty(state, affected);
      matched = true;
    }
    if (!matched) {
      const pendingRoot = canonicalizeRootPath(cwd);
      // Before root resolution, the caller's cwd is the only trustworthy
      // confinement boundary. Do not retain unrelated/escaping paths forever.
      if (!pathContained(pendingRoot, affected)) return;
      let pending = this.#pending.get(pendingRoot);
      if (!pending) {
        pending = { paths: new Set(), fullScanRequired: false, consumedFullScanRoots: new Set() };
        this.#pending.set(pendingRoot, pending);
      }
      if (changesIgnoreRules(affected)) {
        pending.paths.clear();
        pending.fullScanRequired = true;
      } else if (!pending.fullScanRequired) {
        if (!pending.paths.has(affected) && pending.paths.size >= MAX_TARGETED_INDEX_PATHS) {
          pending.paths.clear();
          pending.fullScanRequired = true;
        } else {
          pending.paths.add(affected);
        }
      }
    }
  }

  markRootDirty(root: string): void {
    const canonical = canonicalizeRootPath(root);
    const state = this.#states.get(canonical);
    if (state) {
      markStateFullScan(state);
    } else {
      this.#pending.set(canonical, {
        paths: new Set(),
        fullScanRequired: true,
        consumedFullScanRoots: new Set(),
      });
    }
  }

  async ensureFresh(runtime: FreshnessRuntime, context: RuntimeContext, options: RunOptions = {}): Promise<string> {
    const root = canonicalizeRootPath(await runtime.resolveRoot(context));
    const rootContext: InternalRuntimeContext = { cwd: root, [RESOLVED_ROOT]: true };
    let state = this.#states.get(root);
    if (!state) {
      state = {
        dirtyGeneration: 0,
        cleanGeneration: 0,
        dirtyPaths: new Set(),
        fullScanRequired: false,
        initialized: false,
        lastRefreshAt: 0,
        inFlight: undefined,
        refreshAbort: undefined,
        waiterCount: 0,
        watcher: undefined,
      };
      this.#states.set(root, state);
    }
    if (runtime.watchExternalChanges && state.watcher === undefined) {
      const indexPath = canonicalizeAffectedPath(
        runtime.resolveIndexPath?.(root) ?? join(root, ".asgrep", "index.db"),
      );
      this.#startWatcher(root, state, indexPath);
    }
    for (const [pendingRoot, pending] of this.#pending) {
      if (!pathContained(pendingRoot, root) && !pathContained(root, pendingRoot)) continue;
      if (pending.fullScanRequired) {
        if (!pending.consumedFullScanRoots.has(root)) {
          markStateFullScan(state);
          pending.consumedFullScanRoots.add(root);
        }
        continue;
      }
      for (const path of pending.paths) {
        if (!pathContained(root, path)) continue;
        markStatePathDirty(state, path);
        pending.paths.delete(path);
      }
      if (pending.paths.size === 0) this.#pending.delete(pendingRoot);
    }
    if (state.inFlight) {
      await attachRefreshWaiter(state, state.inFlight, options.signal, this.#waitBudget(options));
      return this.ensureFresh(runtime, rootContext, options);
    }
    if (options.signal?.aborted) throw cancelledRefreshWait();
    const now = this.#now();
    const elapsed = now - state.lastRefreshAt;
    // Lease expiry: initialized and interval elapsed (or clock went backwards).
    // Expiry re-probes status (missing/incompatible) but must not walk a ready
    // index. First search of a ready, clean index is the same: status only.
    const expired = state.initialized && (elapsed < 0 || elapsed >= this.#interval);
    if (state.initialized && state.cleanGeneration === state.dirtyGeneration && !expired) return root;

    const refreshGeneration = state.dirtyGeneration;
    const refreshPaths = [...state.dirtyPaths];
    const fullScanRequired = state.fullScanRequired;
    // Correctness work belongs to the root, not to whichever request happened
    // to start it. Individual callers may stop waiting, but cannot cancel the
    // shared refresh while other callers still depend on it. The last waiter
    // abort stops the in-flight index so Pi/tool cancel cannot leave rayon
    // workers burning CPU.
    const refreshAbort = new AbortController();
    state.refreshAbort = refreshAbort;
    const sharedOptions: RunOptions = { signal: refreshAbort.signal };
    if (options.timeoutMs !== undefined) sharedOptions.timeoutMs = options.timeoutMs;
    if (options.env !== undefined) sharedOptions.env = options.env;
    const refresh = (async () => {
      const health = await probeIndexHealth(runtime, rootContext, sharedOptions);
      const dirty = refreshGeneration > state!.cleanGeneration;
      if (health === "incompatible") {
        // Requisite variety: force rebuild path (hook or reindex).
        if (runtime.rebuildIncompatibleIndex) await runtime.rebuildIncompatibleIndex(rootContext, sharedOptions);
        else await runIndex(runtime, true, rootContext, sharedOptions);
      } else if (health === "missing") {
        await runIndex(runtime, false, rootContext, sharedOptions);
      } else if (dirty && (fullScanRequired || refreshPaths.length === 0)) {
        await runIndex(runtime, false, rootContext, sharedOptions);
      } else if (dirty) {
        await runTargetedIndex(runtime, refreshPaths, rootContext, sharedOptions);
      }
      state!.initialized = true;
      state!.cleanGeneration = refreshGeneration;
      if (state!.dirtyGeneration === refreshGeneration) {
        state!.dirtyPaths.clear();
        state!.fullScanRequired = false;
      }
      state!.lastRefreshAt = this.#now();
    })();
    let tracked!: Promise<void>;
    tracked = refresh.finally(() => {
      if (state!.inFlight === tracked) {
        state!.inFlight = undefined;
        state!.refreshAbort = undefined;
      }
    });
    state.inFlight = tracked;
    // If every waiter is cancelled, the root-owned refresh still needs a
    // rejection handler while it finishes in the background.
    void tracked.catch(() => undefined);
    await attachRefreshWaiter(state, tracked, options.signal, this.#waitBudget(options));
    if (state.cleanGeneration !== state.dirtyGeneration) {
      return this.ensureFresh(runtime, rootContext, options);
    }
    return root;
  }

  shutdown(): void {
    for (const state of this.#states.values()) state.watcher?.close();
    this.#states.clear();
    this.#pending.clear();
  }

  #startWatcher(root: string, state: RootFreshness, indexPath: string): void {
    if (!existsSync(root)) {
      state.watcher = null;
      markStateFullScan(state);
      return;
    }
    try {
      const watcher = this.#watchFactory(
        root,
        { recursive: true, persistent: false, encoding: "utf8" },
        (eventType, filename) => {
          if (!filename) {
            markStateFullScan(state);
            return;
          }
          const affected = canonicalizeAffectedPath(join(root, filename));
          if (ignoredIndexWrite(root, affected, indexPath)) return;
          if (eventType === "rename" || existingDirectory(affected)) {
            markStateFullScan(state);
            return;
          }
          markStatePathDirty(state, affected);
        },
      );
      watcher.on("error", () => {
        watcher.close();
        // Watcher errors (including backend overflow) make event history
        // unknowable. Scan once, then rely on the periodic correctness lease;
        // retrying a permanently broken watcher on every request hot-loops.
        if (state.watcher === watcher) state.watcher = null;
        markStateFullScan(state);
      });
      state.watcher = watcher;
    } catch {
      // Do one correctness scan now, then rely on periodic scans instead of
      // retrying (and rescanning) on every query on unsupported filesystems.
      state.watcher = null;
      markStateFullScan(state);
    }
  }
}
