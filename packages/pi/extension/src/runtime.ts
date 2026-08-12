import { realpath } from "node:fs/promises";
import { constants, accessSync, existsSync, readdirSync, realpathSync, statSync, watch, type FSWatcher } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { basename, dirname, extname, isAbsolute, join, relative, resolve } from "node:path";
import { resolveBinary } from "ast-sgrep";

export const RUNTIME_VERSION = "1.4.0";
export const MACHINE_SCHEMA_VERSION = "1.0.0";
export const CONFIG_SCHEMA_VERSION = 1 as const;
export const INDEX_FORMAT_VERSION = 9 as const;
export const DEFAULT_TIMEOUT_MS = 30_000;
export const DEFAULT_MAX_OUTPUT_BYTES = 4 * 1024 * 1024;
export const DEFAULT_REFRESH_INTERVAL_MS = 30_000;
const MAX_TARGETED_INDEX_PATHS = 1_024;

export interface RuntimeConfig {
  schemaVersion?: typeof CONFIG_SCHEMA_VERSION;
  binaryPath?: string;
  root?: string;
  allowOutsideProject?: boolean;
  timeoutMs?: number;
  maxOutputBytes?: number;
  refreshIntervalMs?: number;
  env?: Readonly<Record<string, string>>;
}

export interface LegacyRuntimeConfig extends Omit<RuntimeConfig, "schemaVersion" | "timeoutMs" | "maxOutputBytes" | "refreshIntervalMs"> {
  schemaVersion?: 0;
  timeout?: number;
  maxOutput?: number;
  refreshInterval?: number;
}

export type RuntimeConfigInput = RuntimeConfig | LegacyRuntimeConfig;

export interface ConfigSources {
  explicitProjectConfig?: RuntimeConfigInput;
  projectSettings?: RuntimeConfigInput;
  globalSettings?: RuntimeConfigInput;
  environment?: NodeJS.ProcessEnv;
  defaults?: RuntimeConfigInput;
}

export interface RuntimeContext { cwd: string }
const RESOLVED_ROOT = Symbol("resolvedRoot");
type InternalRuntimeContext = RuntimeContext & { [RESOLVED_ROOT]?: true };
export interface RunOptions { signal?: AbortSignal; timeoutMs?: number; env?: Readonly<Record<string, string>> }
export interface ExecOptions { cwd: string; env: NodeJS.ProcessEnv; signal?: AbortSignal; timeout?: number }
export interface ExecResult { stdout: string; stderr: string; code?: number | null; exitCode?: number | null; signal?: string | null }
export interface PiExec { exec(command: string, args: readonly string[], options: ExecOptions): Promise<ExecResult> }
export interface MachineEnvelope { tool: "asgrep"; schema_version: string; ok: boolean; version?: string; machine_schema_version?: string; [key: string]: unknown }

export class RuntimeError extends Error {
  constructor(public readonly code: string, message: string, public readonly details: Readonly<Record<string, unknown>> = {}) {
    super(message);
    this.name = "AstSgrepRuntimeError";
  }
}

function finitePositive(value: unknown, fallback: number, name: string): number {
  if (value === undefined) return fallback;
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value <= 0) {
    throw new RuntimeError("INVALID_CONFIG", `${name} must be a positive integer`);
  }
  return value;
}

function sameSetting(current: unknown, legacy: unknown, currentName: string, legacyName: string): unknown {
  if (current !== undefined && legacy !== undefined && current !== legacy) {
    throw new RuntimeError("CONFIG_MIGRATION_CONFLICT", `Conflicting ${currentName} and legacy ${legacyName} values`, { currentName, legacyName });
  }
  return current ?? legacy;
}

const LEGACY_NUMBER_FIELDS = [
  ["timeoutMs", "timeout"],
  ["maxOutputBytes", "maxOutput"],
  ["refreshIntervalMs", "refreshInterval"],
] as const;

/** Convert schema 0/unversioned settings without mutating the rollback source. */
export function migrateConfig(input: RuntimeConfigInput = {}): RuntimeConfig {
  const value = { ...input } as RuntimeConfigInput & Record<string, unknown>;
  const schema = value.schemaVersion ?? 0;
  if (schema !== 0 && schema !== CONFIG_SCHEMA_VERSION) {
    throw new RuntimeError("CONFIG_VERSION_MISMATCH", "Unsupported ast-sgrep configuration schema", { supported: [0, CONFIG_SCHEMA_VERSION], actual: schema, rollbackSafe: true });
  }
  if (schema === CONFIG_SCHEMA_VERSION) return value as RuntimeConfig;
  const legacy = value as LegacyRuntimeConfig & Record<string, unknown>;
  const migrated: RuntimeConfig = { ...legacy, schemaVersion: CONFIG_SCHEMA_VERSION };
  for (const [currentName, legacyName] of LEGACY_NUMBER_FIELDS) {
    const next = sameSetting(value[currentName], legacy[legacyName], currentName, legacyName);
    if (next !== undefined) (migrated as Record<string, unknown>)[currentName] = next;
  }
  for (const [, legacyName] of LEGACY_NUMBER_FIELDS) {
    delete (migrated as Record<string, unknown>)[legacyName];
  }
  return migrated;
}

/** Serialize current settings for a schema-0 rollback without mutating the current value. */
export function rollbackConfig(input: RuntimeConfig): LegacyRuntimeConfig {
  const current = migrateConfig(input);
  const legacy: LegacyRuntimeConfig = { ...current, schemaVersion: 0 };
  for (const [currentName, legacyName] of LEGACY_NUMBER_FIELDS) {
    const value = (current as Record<string, unknown>)[currentName];
    if (value !== undefined) (legacy as Record<string, unknown>)[legacyName] = value;
    delete (legacy as Record<string, unknown>)[currentName];
  }
  return legacy;
}

function envConfig(env: NodeJS.ProcessEnv = {}): RuntimeConfig {
  const result: RuntimeConfig = {};
  // Canonical: ASGREP_BIN; alias AST_SGREP_BINARY (launcher historical name).
  const bin = env.ASGREP_BIN || env.AST_SGREP_BINARY;
  if (bin) result.binaryPath = bin;
  if (env.ASGREP_ROOT) result.root = env.ASGREP_ROOT;
  if (env.ASGREP_TIMEOUT_MS) result.timeoutMs = Number(env.ASGREP_TIMEOUT_MS);
  if (env.ASGREP_MAX_OUTPUT_BYTES) result.maxOutputBytes = Number(env.ASGREP_MAX_OUTPUT_BYTES);
  if (env.ASGREP_REFRESH_INTERVAL_MS) result.refreshIntervalMs = Number(env.ASGREP_REFRESH_INTERVAL_MS);
  return result;
}

/** Merge each setting independently, from the documented lowest to highest priority. */
export function resolveConfig(sources: ConfigSources = {}): Required<Pick<RuntimeConfig, "timeoutMs" | "maxOutputBytes">> & RuntimeConfig {
  const merged: RuntimeConfig = {
    timeoutMs: DEFAULT_TIMEOUT_MS,
    maxOutputBytes: DEFAULT_MAX_OUTPUT_BYTES,
    refreshIntervalMs: DEFAULT_REFRESH_INTERVAL_MS,
    ...migrateConfig(sources.defaults),
    ...envConfig(sources.environment),
    ...migrateConfig(sources.globalSettings),
    ...migrateConfig(sources.projectSettings),
    ...migrateConfig(sources.explicitProjectConfig),
  };
  merged.timeoutMs = finitePositive(merged.timeoutMs, DEFAULT_TIMEOUT_MS, "timeoutMs");
  merged.maxOutputBytes = finitePositive(merged.maxOutputBytes, DEFAULT_MAX_OUTPUT_BYTES, "maxOutputBytes");
  merged.refreshIntervalMs = finitePositive(merged.refreshIntervalMs, DEFAULT_REFRESH_INTERVAL_MS, "refreshIntervalMs");
  // Only explicit project configuration may relax project confinement.
  merged.allowOutsideProject = migrateConfig(sources.explicitProjectConfig).allowOutsideProject === true;
  merged.schemaVersion = CONFIG_SCHEMA_VERSION;
  return merged as Required<Pick<RuntimeConfig, "timeoutMs" | "maxOutputBytes">> & RuntimeConfig;
}

function pathContained(parent: string, child: string): boolean {
  const rel = relative(parent, child);
  const first = rel.split(/[\\/]/u, 1)[0];
  return rel === "" || (!isAbsolute(rel) && first !== "..");
}

export async function resolveRuntimeRoot(projectCwd: string, requestedRoot?: string, allowOutsideProject = false): Promise<string> {
  let project: string;
  let candidate: string;
  try {
    project = await realpath(resolve(projectCwd));
    candidate = await realpath(resolve(project, requestedRoot ?? "."));
  } catch (cause) {
    throw new RuntimeError("INVALID_ROOT", "Project or requested root does not exist", { projectCwd, requestedRoot, cause: cause instanceof Error ? cause.message : String(cause) });
  }
  if (!allowOutsideProject && !pathContained(project, candidate)) {
    throw new RuntimeError("ROOT_OUTSIDE_PROJECT", "Requested root resolves outside the project", { project, requestedRoot, resolvedRoot: candidate });
  }
  return candidate;
}

type BinaryResolver = typeof resolveBinary;
export interface RuntimeDependencies { resolveBinary?: BinaryResolver }

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
  now?: () => number;
  watchFactory?: FreshnessWatchFactory;
}

export type FreshnessWatchFactory = (
  root: string,
  options: { recursive: true; persistent: false; encoding: "utf8" },
  listener: (eventType: "rename" | "change", filename: string | null) => void,
) => FSWatcher;

export type IndexHealth = "ready" | "missing" | "incompatible";
type RootFreshness = {
  dirtyGeneration: number;
  cleanGeneration: number;
  dirtyPaths: Set<string>;
  fullScanRequired: boolean;
  initialized: boolean;
  lastRefreshAt: number;
  inFlight: Promise<void> | undefined;
  /** undefined = untried/retry, null = unsupported for this root. */
  watcher: FSWatcher | null | undefined;
};
type PendingFreshness = {
  paths: Set<string>;
  fullScanRequired: boolean;
  consumedFullScanRoots: Set<string>;
};

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}

function indexHealth(status: MachineEnvelope, knownExisting = false): IndexHealth {
  const index = record(status.index);
  const state = typeof index?.status === "string" ? index.status :
    typeof status.index_status === "string" ? status.index_status : undefined;
  if (state === "incompatible" || index?.compatible === false || status.index_compatible === false) return "incompatible";
  if (state === "missing" || index?.exists === false || status.indexed === false) return "missing";
  if (state === "ready" || state === "current" || index?.exists === true || status.indexed === true) return "ready";
  if (typeof status.index_path === "string" && typeof status.file_count === "number") {
    return knownExisting || status.file_count > 0 ? "ready" : "missing";
  }
  throw new RuntimeError("INDEX_STATUS_UNKNOWN", "ast-sgrep status did not report index freshness", { index: status.index, index_status: status.index_status });
}

function incompatibleStatusFailure(cause: unknown): boolean {
  if (!(cause instanceof RuntimeError) || (cause.code !== "OPERATIONAL_ERROR" && cause.code !== "PROCESS_FAILED")) return false;
  const text = `${cause.message} ${JSON.stringify(cause.details)}`;
  return /incompatib|unsupported.{0,24}schema|schema.{0,24}(version|mismatch)/i.test(text);
}

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

function indexCompletion(response: MachineEnvelope, requireWalkErrors: boolean): { failed: number; walkErrors: boolean } {
  const stats = record(response.stats) ?? response;
  const failed = stats.files_failed;
  const walkErrors = stats.walk_errors;
  if (!Number.isSafeInteger(failed) || (failed as number) < 0
    || (requireWalkErrors ? typeof walkErrors !== "boolean" : walkErrors !== undefined && typeof walkErrors !== "boolean")) {
    throw new RuntimeError(
      "INDEX_RESPONSE_INVALID",
      "ast-sgrep index response omitted valid completion status",
      { filesFailed: failed, walkErrors, requireWalkErrors },
    );
  }
  return {
    failed: failed as number,
    walkErrors: walkErrors === true,
  };
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
function waitForRefresh(refresh: Promise<void>, signal?: AbortSignal): Promise<void> {
  if (!signal) return refresh;
  if (signal.aborted) return Promise.reject(cancelledRefreshWait());
  return new Promise<void>((resolveWait, rejectWait) => {
    const onAbort = () => {
      signal.removeEventListener("abort", onAbort);
      rejectWait(cancelledRefreshWait());
    };
    signal.addEventListener("abort", onAbort, { once: true });
    refresh.then(
      () => {
        signal.removeEventListener("abort", onAbort);
        resolveWait();
      },
      (cause) => {
        signal.removeEventListener("abort", onAbort);
        rejectWait(cause);
      },
    );
  });
}

export class FreshnessCoordinator {
  readonly #states = new Map<string, RootFreshness>();
  readonly #pending = new Map<string, PendingFreshness>();
  readonly #interval: number;
  readonly #now: () => number;
  readonly #watchFactory: FreshnessWatchFactory;

  constructor(options: FreshnessCoordinatorOptions = {}) {
    this.#interval = finitePositive(options.refreshIntervalMs, DEFAULT_REFRESH_INTERVAL_MS, "refreshIntervalMs");
    this.#now = options.now ?? Date.now;
    this.#watchFactory = options.watchFactory ?? watch;
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
    const root = await runtime.resolveRoot(context);
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
      await waitForRefresh(state.inFlight, options.signal);
      return this.ensureFresh(runtime, rootContext, options);
    }
    const now = this.#now();
    const elapsed = now - state.lastRefreshAt;
    // Lease expiry: initialized and interval elapsed (or clock went backwards).
    const expired = state.initialized && (elapsed < 0 || elapsed >= this.#interval);
    if (state.initialized && state.cleanGeneration === state.dirtyGeneration && !expired) return root;

    const refreshGeneration = state.dirtyGeneration;
    const refreshPaths = [...state.dirtyPaths];
    const fullScanRequired = state.fullScanRequired;
    // Correctness work belongs to the root, not to whichever request happened
    // to start it. Individual callers may stop waiting, but cannot cancel the
    // shared refresh while other callers depend on it.
    const sharedOptions: RunOptions = {};
    if (options.timeoutMs !== undefined) sharedOptions.timeoutMs = options.timeoutMs;
    if (options.env !== undefined) sharedOptions.env = options.env;
    const refresh = (async () => {
      const health = await probeIndexHealth(runtime, rootContext, sharedOptions);
      const dirty = refreshGeneration > state!.cleanGeneration;
      if (health === "incompatible") {
        // Requisite variety: force rebuild path (hook or reindex).
        if (runtime.rebuildIncompatibleIndex) await runtime.rebuildIncompatibleIndex(rootContext, sharedOptions);
        else await runIndex(runtime, true, rootContext, sharedOptions);
      } else if (health === "missing" || !state!.initialized || expired || (dirty && (fullScanRequired || refreshPaths.length === 0))) {
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
      if (state!.inFlight === tracked) state!.inFlight = undefined;
    });
    state.inFlight = tracked;
    // If every waiter is cancelled, the root-owned refresh still needs a
    // rejection handler while it finishes in the background.
    void tracked.catch(() => undefined);
    await waitForRefresh(tracked, options.signal);
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

function getBinary(config: RuntimeConfig, env: NodeJS.ProcessEnv, resolver: BinaryResolver): string {
  let binary: string;
  try {
    const options = config.binaryPath ? { binaryPath: config.binaryPath, env } : { env };
    binary = resolver(options);
  } catch (cause) {
    const message = cause instanceof Error ? cause.message : String(cause);
    if (config.binaryPath) {
      throw new RuntimeError("BINARY_NOT_FOUND", `Configured ast-sgrep binary is unavailable: ${config.binaryPath}`, { binaryPath: config.binaryPath, cause: message });
    }
    throw new RuntimeError("BINARY_RESOLUTION_FAILED", "Unable to resolve an ast-sgrep binary for this platform", { cause: message });
  }
  try {
    accessSync(binary, constants.X_OK);
  } catch (cause) {
    throw new RuntimeError("BINARY_NOT_EXECUTABLE", `ast-sgrep binary is not executable: ${binary}`, { binaryPath: binary, cause: cause instanceof Error ? cause.message : String(cause) });
  }
  return binary;
}

function byteLength(value: string): number { return Buffer.byteLength(value, "utf8"); }

/** Present-field version identity checks. Pass `requireIdentity` for version --json. */
function assertVersionTriple(envelope: Partial<MachineEnvelope>, requireIdentity = false): void {
  // Compound guards (same short-circuit as nested if): check only when required or field present.
  if ((requireIdentity || envelope.version !== undefined) && envelope.version !== RUNTIME_VERSION) {
    throw new RuntimeError("VERSION_MISMATCH", "ast-sgrep binary version does not match the extension", { expected: RUNTIME_VERSION, actual: envelope.version });
  }
  if ((requireIdentity || envelope.machine_schema_version !== undefined) && envelope.machine_schema_version !== MACHINE_SCHEMA_VERSION) {
    throw new RuntimeError("PROTOCOL_MISMATCH", "ast-sgrep binary reports an incompatible machine protocol", { expected: MACHINE_SCHEMA_VERSION, actual: envelope.machine_schema_version });
  }
}

/**
 * Nonzero CLI exit: prefer structured failed envelope (OPERATIONAL_ERROR), else PROCESS_FAILED.
 * Always throws — error-path extract so parseEnvelope keeps success-path protocol field checks.
 */
function throwNonzeroProcessFailure(result: ExecResult, code: number): never {
  try {
    const value = record(JSON.parse(result.stdout) as unknown);
    // Wire-valid ok:false asgrep envelope → structured operational failure (not PROCESS_FAILED).
    if (value && value.tool === "asgrep" && value.schema_version === MACHINE_SCHEMA_VERSION && value.ok === false) {
      const failure = record(value.error);
      const message = typeof failure?.message === "string" ? failure.message : "ast-sgrep reported an operational failure";
      throw new RuntimeError("OPERATIONAL_ERROR", message, { command: value.command, error: failure, exitCode: code });
    }
  } catch (cause) {
    if (cause instanceof RuntimeError) throw cause;
  }
  throw new RuntimeError("PROCESS_FAILED", `ast-sgrep exited with code ${code}`, {
    exitCode: code,
    signal: result.signal ?? undefined,
    stderr: result.stderr.slice(0, 1024),
  });
}

/** Map exec failures (abort / timeout / generic) to RuntimeError. Re-throws RuntimeError as-is. */
function rethrowExecFailure(cause: unknown, options: RunOptions, timeout: number): never {
  if (cause instanceof RuntimeError) throw cause;
  if (options.signal?.aborted || (cause instanceof Error && cause.name === "AbortError")) {
    throw new RuntimeError("CANCELLED", "ast-sgrep execution was cancelled");
  }
  const message = cause instanceof Error ? cause.message : String(cause);
  if (/timeout|timed out/i.test(message)) {
    throw new RuntimeError("TIMEOUT", `ast-sgrep exceeded ${timeout}ms`, { timeoutMs: timeout });
  }
  throw new RuntimeError("EXEC_FAILED", "Unable to execute ast-sgrep", { cause: message });
}

function parseEnvelope(result: ExecResult, limit: number): MachineEnvelope {
  const stdoutBytes = byteLength(result.stdout);
  const stderrBytes = byteLength(result.stderr);
  // Byte lengths are non-negative: sum > limit covers either-side overflow and combined cap.
  if (stdoutBytes + stderrBytes > limit) {
    throw new RuntimeError("OUTPUT_LIMIT", "ast-sgrep output exceeded the configured limit", { limit, stdoutBytes, stderrBytes });
  }
  const code = result.exitCode ?? result.code ?? 0;
  if (code !== 0) {
    throwNonzeroProcessFailure(result, code);
  }
  let value: unknown;
  try { value = JSON.parse(result.stdout); }
  catch (cause) { throw new RuntimeError("MALFORMED_OUTPUT", "ast-sgrep returned malformed JSON", { cause: cause instanceof Error ? cause.message : String(cause) }); }
  const envelope = record(value) as Partial<MachineEnvelope> | undefined;
  if (!envelope) throw new RuntimeError("MALFORMED_OUTPUT", "ast-sgrep returned a non-object JSON payload");
  // Protocol field varieties (Ashby Keep) — sequential wire-contract checks stay here.
  if (envelope.tool !== "asgrep") throw new RuntimeError("TOOL_MISMATCH", "Response is not from ast-sgrep", { actual: envelope.tool });
  if (envelope.schema_version !== MACHINE_SCHEMA_VERSION) throw new RuntimeError("PROTOCOL_MISMATCH", "Unsupported ast-sgrep machine protocol", { expected: MACHINE_SCHEMA_VERSION, actual: envelope.schema_version });
  if (typeof envelope.ok !== "boolean") throw new RuntimeError("MALFORMED_OUTPUT", "ast-sgrep response is missing boolean ok");
  if (!envelope.ok) {
    // Preserve pre-extract failure shape: plain object check (arrays allowed as error bag).
    const failure = envelope.error && typeof envelope.error === "object" ? envelope.error as Record<string, unknown> : undefined;
    const message = typeof failure?.message === "string" ? failure.message : "ast-sgrep reported an operational failure";
    throw new RuntimeError("OPERATIONAL_ERROR", message, { command: envelope.command, error: failure });
  }
  assertVersionTriple(envelope);
  return envelope as MachineEnvelope;
}
function indexPathFor(root: string, env: NodeJS.ProcessEnv): string {
  const configured = env.ASGREP_INDEX_PATH;
  if (!configured) return join(root, ".asgrep", "index.db");
  const resolved = resolve(root, configured);
  return extname(resolved) === ".db" ? resolved : join(resolved, "index.db");
}

function indexQuarantines(indexPath: string): string[] {
  const quarantinePrefix = `${basename(indexPath)}.corrupt`;
  try {
    return readdirSync(dirname(indexPath), { withFileTypes: true })
      .filter((entry) => entry.isFile() && (entry.name === quarantinePrefix || entry.name.startsWith(`${quarantinePrefix}.`)))
      .map((entry) => join(dirname(indexPath), entry.name))
      .sort();
  } catch {
    return [];
  }
}

/** Classify a rebuild failure and identify recovery copies made by this attempt. */
function throwIndexRebuildFailed(cause: unknown, indexPath: string, quarantinesBefore: ReadonlySet<string>): never {
  const newQuarantines = indexQuarantines(indexPath).filter((path) => !quarantinesBefore.has(path));
  const recoveryPaths = [
    ...newQuarantines,
    ...(existsSync(indexPath) ? [indexPath] : []),
  ];
  throw new RuntimeError("INDEX_REBUILD_FAILED", "Incompatible index rebuild failed; the prior index remains recoverable", {
    indexPath,
    recoveryPath: recoveryPaths[0] ?? indexPath,
    recoveryPaths,
    priorIndexPreserved: recoveryPaths.length > 0,
    expectedIndexFormat: INDEX_FORMAT_VERSION,
    cause: cause instanceof Error ? cause.message : String(cause),
  });
}

function inspectIndexFile(path: string): IndexHealth {
  if (!existsSync(path)) return "missing";
  let database: DatabaseSync | undefined;
  try {
    database = new DatabaseSync(path, { readOnly: true });
    const row = database.prepare("PRAGMA user_version").get() as Record<string, unknown> | undefined;
    const version = Number(Object.values(row ?? {})[0]);
    if (version > INDEX_FORMAT_VERSION) {
      throw new RuntimeError("INDEX_VERSION_TOO_NEW", "Index schema is newer than this ast-sgrep runtime", {
        actual: version,
        supported: INDEX_FORMAT_VERSION,
        rollbackSafe: true,
      });
    }
    return version === INDEX_FORMAT_VERSION ? "ready" : "incompatible";
  } catch (cause) {
    if (cause instanceof RuntimeError) throw cause;
    return "incompatible";
  } finally {
    database?.close();
  }
}


export class AstSgrepRuntime {
  readonly watchExternalChanges = true;
  readonly config: ReturnType<typeof resolveConfig>;
  readonly #resolver: BinaryResolver;
  readonly #environment: NodeJS.ProcessEnv;
  constructor(private readonly pi: PiExec, sources: ConfigSources = {}, dependencies: RuntimeDependencies = {}) {
    this.#environment = sources.environment ?? process.env;
    this.config = resolveConfig({ ...sources, environment: this.#environment });
    this.#resolver = dependencies.resolveBinary ?? resolveBinary;
  }

  async resolveRoot(context: RuntimeContext): Promise<string> {
    return (context as InternalRuntimeContext)[RESOLVED_ROOT]
      ? resolveRuntimeRoot(context.cwd)
      : resolveRuntimeRoot(context.cwd, this.config.root, this.config.allowOutsideProject);
  }
  resolveIndexPath(root: string): string {
    return indexPathFor(root, { ...this.#environment, ...this.config.env });
  }
  async inspectIndexCompatibility(context: RuntimeContext): Promise<IndexHealth> {
    const root = await this.resolveRoot(context);
    return inspectIndexFile(indexPathFor(root, { ...this.#environment, ...this.config.env }));
  }

  async rebuildIncompatibleIndex(context: RuntimeContext, options: RunOptions = {}): Promise<MachineEnvelope> {
    const root = await this.resolveRoot(context);
    const env = { ...this.#environment, ...this.config.env, ...options.env };
    const indexPath = indexPathFor(root, env);
    const quarantinesBefore = new Set(indexQuarantines(indexPath));
    try {
      // Core reindex prepares files before opening one bulk transaction and
      // commits rewrites plus stale-row pruning together. Keeping the same DB
      // inode avoids stale warm NAPI sessions and removes rename crash windows.
      const response = await this.run(["reindex", ".", "--json"], { cwd: root }, options);
      const { failed, walkErrors } = indexCompletion(response, true);
      if (failed > 0 || walkErrors) {
        throw new RuntimeError(
          "INDEX_UPDATE_INCOMPLETE",
          "ast-sgrep did not complete the incompatible-index rebuild",
          { failed, walkErrors, force: true },
        );
      }
      if (inspectIndexFile(indexPath) !== "ready") {
        throw new RuntimeError("INDEX_REBUILD_INVALID", "Rebuilt index has an incompatible format", { expected: INDEX_FORMAT_VERSION });
      }
      return response;
    } catch (cause) {
      throwIndexRebuildFailed(cause, indexPath, quarantinesBefore);
    }
  }


  async run(args: readonly string[], context: RuntimeContext, options: RunOptions = {}): Promise<MachineEnvelope> {
    if (!Array.isArray(args) || args.some((arg) => typeof arg !== "string")) throw new RuntimeError("INVALID_ARGUMENTS", "Arguments must be a string array");
    if (options.signal?.aborted) throw new RuntimeError("CANCELLED", "ast-sgrep execution was cancelled");
    const root = await this.resolveRoot(context);
    const timeout = finitePositive(options.timeoutMs, this.config.timeoutMs, "timeoutMs");
    const env: NodeJS.ProcessEnv = { ...this.#environment, ...this.config.env, ...options.env, NO_COLOR: "1" };
    const binary = getBinary(this.config, env, this.#resolver);
    try {
      const execOptions: ExecOptions = { cwd: root, env, timeout };
      if (options.signal) execOptions.signal = options.signal;
      const result = await this.pi.exec(binary, Object.freeze([...args]), execOptions);
      return parseEnvelope(result, this.config.maxOutputBytes);
    } catch (cause) {
      rethrowExecFailure(cause, options, timeout);
    }
  }

  /** Absolute path to the native binary (for sticky serve / stdin batch spawn). */
  resolveBinaryPath(options: { env?: NodeJS.ProcessEnv } = {}): string {
    const env: NodeJS.ProcessEnv = { ...this.#environment, ...this.config.env, ...options.env, NO_COLOR: "1" };
    return getBinary(this.config, env, this.#resolver);
  }

  /** Merged process env for native Code Mode workers. */
  nativeEnv(options: { env?: NodeJS.ProcessEnv } = {}): NodeJS.ProcessEnv {
    return { ...this.#environment, ...this.config.env, ...options.env, NO_COLOR: "1" };
  }

  async checkCompatibility(context: RuntimeContext, options: RunOptions = {}): Promise<MachineEnvelope> {
    const value = await this.run(["version", "--json"], context, options);
    assertVersionTriple(value, true);
    return value;
  }
}
