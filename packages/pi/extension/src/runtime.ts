import { mkdir, mkdtemp, realpath, rename, rm } from "node:fs/promises";
import { constants, accessSync, existsSync, realpathSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { DatabaseSync } from "node:sqlite";
import { basename, dirname, extname, isAbsolute, join, relative, resolve } from "node:path";
import { resolveBinary } from "ast-sgrep";

export const RUNTIME_VERSION = "1.3.2";
export const MACHINE_SCHEMA_VERSION = "1.0.0";
export const CONFIG_SCHEMA_VERSION = 1 as const;
export const INDEX_FORMAT_VERSION = 5 as const;
export const DEFAULT_TIMEOUT_MS = 30_000;
export const DEFAULT_MAX_OUTPUT_BYTES = 4 * 1024 * 1024;
export const DEFAULT_REFRESH_INTERVAL_MS = 30_000;

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
  const timeoutMs = sameSetting(value.timeoutMs, legacy.timeout, "timeoutMs", "timeout");
  const maxOutputBytes = sameSetting(value.maxOutputBytes, legacy.maxOutput, "maxOutputBytes", "maxOutput");
  const refreshIntervalMs = sameSetting(value.refreshIntervalMs, legacy.refreshInterval, "refreshIntervalMs", "refreshInterval");
  if (timeoutMs !== undefined) migrated.timeoutMs = timeoutMs as number;
  if (maxOutputBytes !== undefined) migrated.maxOutputBytes = maxOutputBytes as number;
  if (refreshIntervalMs !== undefined) migrated.refreshIntervalMs = refreshIntervalMs as number;
  delete (migrated as Record<string, unknown>).timeout;
  delete (migrated as Record<string, unknown>).maxOutput;
  delete (migrated as Record<string, unknown>).refreshInterval;
  return migrated;
}

/** Serialize current settings for a schema-0 rollback without mutating the current value. */
export function rollbackConfig(input: RuntimeConfig): LegacyRuntimeConfig {
  const current = migrateConfig(input);
  const legacy: LegacyRuntimeConfig = { ...current, schemaVersion: 0 };
  if (current.timeoutMs !== undefined) legacy.timeout = current.timeoutMs;
  if (current.maxOutputBytes !== undefined) legacy.maxOutput = current.maxOutputBytes;
  if (current.refreshIntervalMs !== undefined) legacy.refreshInterval = current.refreshIntervalMs;
  delete (legacy as Record<string, unknown>).timeoutMs;
  delete (legacy as Record<string, unknown>).maxOutputBytes;
  delete (legacy as Record<string, unknown>).refreshIntervalMs;
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

function isContained(parent: string, child: string): boolean {
  const rel = relative(parent, child);
  return rel === "" || (!rel.startsWith("..") && !isAbsolute(rel));
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
  if (!allowOutsideProject && !isContained(project, candidate)) {
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
}

export interface FreshnessCoordinatorOptions {
  refreshIntervalMs?: number;
  now?: () => number;
}

export type IndexHealth = "ready" | "missing" | "incompatible";
type RootFreshness = {
  dirtyGeneration: number;
  cleanGeneration: number;
  initialized: boolean;
  lastRefreshAt: number;
  inFlight: Promise<void> | undefined;
};

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}

function indexHealth(status: MachineEnvelope): IndexHealth {
  const index = record(status.index);
  const state = typeof index?.status === "string" ? index.status :
    typeof status.index_status === "string" ? status.index_status : undefined;
  if (state === "incompatible" || index?.compatible === false || status.index_compatible === false) return "incompatible";
  if (state === "missing" || index?.exists === false || status.indexed === false) return "missing";
  if (state === "ready" || state === "current" || index?.exists === true || status.indexed === true) return "ready";
  if (typeof status.index_path === "string" && typeof status.file_count === "number") {
    return status.file_count === 0 ? "missing" : "ready";
  }
  throw new RuntimeError("INDEX_STATUS_UNKNOWN", "ast-sgrep status did not report index freshness", { index: status.index, index_status: status.index_status });
}

function incompatibleStatusFailure(cause: unknown): boolean {
  if (!(cause instanceof RuntimeError) || (cause.code !== "OPERATIONAL_ERROR" && cause.code !== "PROCESS_FAILED")) return false;
  const text = `${cause.message} ${JSON.stringify(cause.details)}`;
  return /incompatib|unsupported.{0,24}schema|schema.{0,24}(version|mismatch)/i.test(text);
}

function pathContained(root: string, path: string): boolean {
  const rel = relative(root, path);
  return rel === "" || (!rel.startsWith("..") && !isAbsolute(rel));
}

function canonicalizeAffectedPath(path: string): string {
  const unresolved: string[] = [];
  let existing = resolve(path);
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

export class FreshnessCoordinator {
  readonly #states = new Map<string, RootFreshness>();
  readonly #pendingPaths = new Set<string>();
  readonly #interval: number;
  readonly #now: () => number;

  constructor(options: FreshnessCoordinatorOptions = {}) {
    this.#interval = finitePositive(options.refreshIntervalMs, DEFAULT_REFRESH_INTERVAL_MS, "refreshIntervalMs");
    this.#now = options.now ?? Date.now;
  }

  markAffectedPath(path: string, cwd: string): void {
    const affected = canonicalizeAffectedPath(isAbsolute(path) ? path : resolve(canonicalizeAffectedPath(cwd), path));
    this.#pendingPaths.add(affected);
    for (const [root, state] of this.#states) {
      if (pathContained(root, affected)) state.dirtyGeneration += 1;
    }
  }

  markRootDirty(root: string): void {
    const canonical = canonicalizeAffectedPath(root);
    const state = this.#states.get(canonical);
    if (state) state.dirtyGeneration += 1;
    else this.#pendingPaths.add(canonical);
  }

  async ensureFresh(runtime: FreshnessRuntime, context: RuntimeContext, options: RunOptions = {}): Promise<string> {
    const root = await runtime.resolveRoot(context);
    const rootContext: InternalRuntimeContext = { cwd: root, [RESOLVED_ROOT]: true };
    let state = this.#states.get(root);
    if (!state) {
      state = { dirtyGeneration: 0, cleanGeneration: 0, initialized: false, lastRefreshAt: 0, inFlight: undefined };
      this.#states.set(root, state);
    }
    for (const path of this.#pendingPaths) {
      if (!pathContained(root, path)) continue;
      state.dirtyGeneration += 1;
      this.#pendingPaths.delete(path);
    }
    if (state.inFlight) {
      await state.inFlight;
      return this.ensureFresh(runtime, rootContext, options);
    }
    const now = this.#now();
    const elapsed = now - state.lastRefreshAt;
    const expired = state.initialized && (elapsed < 0 || elapsed >= this.#interval);
    if (state.initialized && state.cleanGeneration === state.dirtyGeneration && !expired) return root;

    const refreshGeneration = state.dirtyGeneration;
    const wasInitialized = state.initialized;
    const refresh = (async () => {
      let health = await runtime.inspectIndexCompatibility?.(rootContext);
      if (health !== "incompatible") {
        try {
          const status = await runtime.run(["status", ".", "--json"], rootContext, options);
          health = indexHealth(status);
        } catch (cause) {
          if (!incompatibleStatusFailure(cause)) throw cause;
          health = "incompatible";
        }
      }
      const dirty = refreshGeneration > state!.cleanGeneration;
      if (health === "incompatible") {
        if (runtime.rebuildIncompatibleIndex) await runtime.rebuildIncompatibleIndex(rootContext, options);
        else await runtime.run(["reindex", ".", "--json"], rootContext, options);
      } else if (health === "missing" || !wasInitialized || dirty) {
        await runtime.run(["index", ".", "--json"], rootContext, options);
      } else if (expired) {
        // Lease expired without dirty marks: incremental index (not force reindex)
        // so external create/modify/delete are reconciled without rebuild thrash (5du.9).
        await runtime.run(["index", ".", "--json"], rootContext, options);
      }
      state!.initialized = true;
      state!.cleanGeneration = refreshGeneration;
      state!.lastRefreshAt = this.#now();
    })();
    state.inFlight = refresh;
    try {
      await refresh;
      return root;
    } finally {
      if (state.inFlight === refresh) state.inFlight = undefined;
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

function parseEnvelope(result: ExecResult, limit: number): MachineEnvelope {
  const stdoutBytes = byteLength(result.stdout);
  const stderrBytes = byteLength(result.stderr);
  if (stdoutBytes > limit || stderrBytes > limit || stdoutBytes + stderrBytes > limit) {
    throw new RuntimeError("OUTPUT_LIMIT", "ast-sgrep output exceeded the configured limit", { limit, stdoutBytes, stderrBytes });
  }
  const code = result.exitCode ?? result.code ?? 0;
  if (code !== 0) {
    try {
      const value = JSON.parse(result.stdout) as Partial<MachineEnvelope>;
      if (value && typeof value === "object" && value.tool === "asgrep" && value.schema_version === MACHINE_SCHEMA_VERSION && value.ok === false) {
        const failure = record(value.error);
        const message = typeof failure?.message === "string" ? failure.message : "ast-sgrep reported an operational failure";
        throw new RuntimeError("OPERATIONAL_ERROR", message, { command: value.command, error: failure, exitCode: code });
      }
    } catch (cause) {
      if (cause instanceof RuntimeError) throw cause;
    }
    throw new RuntimeError("PROCESS_FAILED", `ast-sgrep exited with code ${code}`, { exitCode: code, signal: result.signal ?? undefined, stderr: result.stderr.slice(0, 1024) });
  }
  let value: unknown;
  try { value = JSON.parse(result.stdout); }
  catch (cause) { throw new RuntimeError("MALFORMED_OUTPUT", "ast-sgrep returned malformed JSON", { cause: cause instanceof Error ? cause.message : String(cause) }); }
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new RuntimeError("MALFORMED_OUTPUT", "ast-sgrep returned a non-object JSON payload");
  const envelope = value as Partial<MachineEnvelope>;
  if (envelope.tool !== "asgrep") throw new RuntimeError("TOOL_MISMATCH", "Response is not from ast-sgrep", { actual: envelope.tool });
  if (envelope.schema_version !== MACHINE_SCHEMA_VERSION) throw new RuntimeError("PROTOCOL_MISMATCH", "Unsupported ast-sgrep machine protocol", { expected: MACHINE_SCHEMA_VERSION, actual: envelope.schema_version });
  if (typeof envelope.ok !== "boolean") throw new RuntimeError("MALFORMED_OUTPUT", "ast-sgrep response is missing boolean ok");
  if (!envelope.ok) {
    const failure = envelope.error && typeof envelope.error === "object" ? envelope.error as Record<string, unknown> : undefined;
    const message = typeof failure?.message === "string" ? failure.message : "ast-sgrep reported an operational failure";
    throw new RuntimeError("OPERATIONAL_ERROR", message, { command: envelope.command, error: failure });
  }
  if (envelope.version !== undefined && envelope.version !== RUNTIME_VERSION) throw new RuntimeError("VERSION_MISMATCH", "ast-sgrep binary version does not match the extension", { expected: RUNTIME_VERSION, actual: envelope.version });
  if (envelope.machine_schema_version !== undefined && envelope.machine_schema_version !== MACHINE_SCHEMA_VERSION) throw new RuntimeError("PROTOCOL_MISMATCH", "ast-sgrep binary reports an incompatible machine protocol", { expected: MACHINE_SCHEMA_VERSION, actual: envelope.machine_schema_version });
  return envelope as MachineEnvelope;
}
function indexPathFor(root: string, env: NodeJS.ProcessEnv): string {
  const configured = env.ASGREP_INDEX_PATH;
  if (!configured) return join(root, ".asgrep", "index.db");
  const resolved = resolve(root, configured);
  return extname(resolved) === ".db" ? resolved : join(resolved, "index.db");
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
  async inspectIndexCompatibility(context: RuntimeContext): Promise<IndexHealth> {
    const root = await this.resolveRoot(context);
    return inspectIndexFile(indexPathFor(root, { ...this.#environment, ...this.config.env }));
  }

  async rebuildIncompatibleIndex(context: RuntimeContext, options: RunOptions = {}): Promise<MachineEnvelope> {
    const root = await this.resolveRoot(context);
    const env = { ...this.#environment, ...this.config.env, ...options.env };
    const indexPath = indexPathFor(root, env);
    const parent = dirname(indexPath);
    await mkdir(parent, { recursive: true });
    const temporaryDirectory = await mkdtemp(join(parent, ".rebuild-"));
    const replacementPath = join(temporaryDirectory, "index.db");
    const backupPath = `${indexPath}.backup-${randomUUID()}`;
    let priorMoved = false;
    try {
      const response = await this.run(["--index-path", replacementPath, "index", ".", "--json"], { cwd: root }, options);
      if (inspectIndexFile(replacementPath) !== "ready") {
        throw new RuntimeError("INDEX_REBUILD_INVALID", "Replacement index has an incompatible format", { expected: INDEX_FORMAT_VERSION });
      }
      if (existsSync(indexPath)) {
        await rename(indexPath, backupPath);
        priorMoved = true;
      }
      try {
        await rename(replacementPath, indexPath);
      } catch (cause) {
        if (priorMoved) await rename(backupPath, indexPath);
        throw cause;
      }
      if (priorMoved) await rm(backupPath, { force: true });
      return response;
    } catch (cause) {
      let recoveryPath = indexPath;
      let priorIndexPreserved = existsSync(indexPath);
      if (priorMoved && !priorIndexPreserved && existsSync(backupPath)) {
        recoveryPath = backupPath;
        priorIndexPreserved = true;
      }
      throw new RuntimeError("INDEX_REBUILD_FAILED", "Incompatible index rebuild failed; the prior index remains recoverable", {
        indexPath,
        recoveryPath,
        priorIndexPreserved,
        expectedIndexFormat: INDEX_FORMAT_VERSION,
        cause: cause instanceof Error ? cause.message : String(cause),
      });
    } finally {
      await rm(temporaryDirectory, { recursive: true, force: true });
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
      if (cause instanceof RuntimeError) throw cause;
      if (options.signal?.aborted || (cause instanceof Error && cause.name === "AbortError")) throw new RuntimeError("CANCELLED", "ast-sgrep execution was cancelled");
      const message = cause instanceof Error ? cause.message : String(cause);
      if (/timeout|timed out/i.test(message)) throw new RuntimeError("TIMEOUT", `ast-sgrep exceeded ${timeout}ms`, { timeoutMs: timeout });
      throw new RuntimeError("EXEC_FAILED", "Unable to execute ast-sgrep", { cause: message });
    }
  }

  async checkCompatibility(context: RuntimeContext, options: RunOptions = {}): Promise<MachineEnvelope> {
    const value = await this.run(["version", "--json"], context, options);
    if (value.version !== RUNTIME_VERSION) throw new RuntimeError("VERSION_MISMATCH", "ast-sgrep binary version does not match the extension", { expected: RUNTIME_VERSION, actual: value.version });
    if (value.machine_schema_version !== MACHINE_SCHEMA_VERSION) throw new RuntimeError("PROTOCOL_MISMATCH", "ast-sgrep binary reports an incompatible machine protocol", { expected: MACHINE_SCHEMA_VERSION, actual: value.machine_schema_version });
    return value;
  }
}
