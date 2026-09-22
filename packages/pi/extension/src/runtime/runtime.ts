import { realpath } from "node:fs/promises";
import { constants, accessSync, readFileSync } from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { createRequire } from "node:module";
import { resolveBinary } from "ast-sgrep";
import {
  CONFIG_SCHEMA_VERSION,
  DEFAULT_MAX_OUTPUT_BYTES,
  DEFAULT_REFRESH_INTERVAL_MS,
  DEFAULT_TIMEOUT_MS,
  INDEX_FORMAT_VERSION,
  MACHINE_SCHEMA_VERSION,
  RUNTIME_VERSION,
  RuntimeError,
  type InternalRuntimeContext,
  type ExecOptions,
  type ExecResult,
  type MachineEnvelope,
  type PiExec,
  RESOLVED_ROOT,
  type RunOptions,
  type RuntimeContext,
} from "./types.js";
import {
  finitePositive,
  migrateConfig,
  resolveConfig,
  rollbackConfig,
  type ConfigSources,
  type RuntimeConfig,
} from "./config.js";
import {
  indexCompletion,
  indexPathFor,
  indexQuarantines,
  inspectIndexFile,
  pathContained,
  record,
  throwIndexRebuildFailed,
  type IndexHealth,
} from "./index-health.js";

// The public surface of ./runtime is a contract (package.json exports +
// tests): re-export the symbols that moved to their own modules.
export {
  CONFIG_SCHEMA_VERSION,
  DEFAULT_FRESHNESS_WAIT_MS,
  DEFAULT_MAX_OUTPUT_BYTES,
  DEFAULT_REFRESH_INTERVAL_MS,
  DEFAULT_TIMEOUT_MS,
  INDEX_FORMAT_VERSION,
  MACHINE_SCHEMA_VERSION,
  RUNTIME_VERSION,
  RuntimeError,
} from "./types.js";
export type {
  ExecOptions,
  ExecResult,
  MachineEnvelope,
  PiExec,
  RunOptions,
  RuntimeContext,
} from "./types.js";
export { migrateConfig, resolveConfig, rollbackConfig } from "./config.js";
export type {
  ConfigSources,
  LegacyRuntimeConfig,
  RuntimeConfig,
  RuntimeConfigInput,
} from "./config.js";
export {
  FreshnessCoordinator,
  type FreshnessCoordinatorOptions,
  type FreshnessRuntime,
  type FreshnessWatchFactory,
} from "./freshness.js";
export type { IndexHealth } from "./index-health.js";

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

function getBinary(config: RuntimeConfig, env: NodeJS.ProcessEnv, resolver: BinaryResolver, onRecovery?: (warning: string) => void): string {
  let binary: string;
  let warning: string | undefined;
  try {
    const options = config.binaryPath ? { binaryPath: config.binaryPath, env } : { env };
    binary = resolver(options);
  } catch (cause) {
    const message = cause instanceof Error ? cause.message : String(cause);
    const failure = cause as { code?: string; cause?: { code?: string } } | null;
    // Missing environment-only dev builds may recover, never explicit settings,
    // permissions, empty artifacts, checksum failures, PATH, or downloads.
    if (onRecovery && (env.ASGREP_BIN || env.AST_SGREP_BINARY) && failure?.code === "ASGREP_EXECUTABLE_MISSING" &&
        (failure.cause?.code === "ENOENT" || failure.cause?.code === "ENOTDIR")) {
      const variable = env.ASGREP_BIN ? "ASGREP_BIN" : "AST_SGREP_BINARY";
      try {
        binary = resolver({ env: { ...env, ASGREP_BIN: undefined, AST_SGREP_BINARY: undefined } });
      } catch (fallbackCause) {
        const fallback = fallbackCause instanceof Error ? fallbackCause.message : String(fallbackCause);
        throw new RuntimeError("BINARY_RESOLUTION_FAILED", `Missing ${variable} override (${config.binaryPath ?? env[variable]}); bundled binary unavailable: ${fallback}. Run /asgrep-doctor and reinstall pi-ast-sgrep with optional dependencies enabled.`, { cause: message, fallbackCause: fallback });
      }
      warning = `Ignored missing ${variable}=${config.binaryPath ?? env[variable]}; using bundled binary ${binary}. Remove the stale export from your shell configuration.`;
    } else {
      const hint = "Correct binaryPath or unset ASGREP_BIN/AST_SGREP_BINARY; run /asgrep-doctor.";
      if (config.binaryPath) {
        throw new RuntimeError("BINARY_NOT_FOUND", `Configured ast-sgrep binary is unavailable: ${config.binaryPath}. ${message}. ${hint}`, { binaryPath: config.binaryPath, cause: message, hint });
      }
      throw new RuntimeError("BINARY_RESOLUTION_FAILED", `Unable to resolve an ast-sgrep binary for this platform: ${message}. Run /asgrep-doctor.`, { cause: message });
    }
  }
  try {
    accessSync(binary, constants.X_OK);
  } catch (cause) {
    throw new RuntimeError("BINARY_NOT_EXECUTABLE", `ast-sgrep binary is not executable: ${binary}`, { binaryPath: binary, cause: cause instanceof Error ? cause.message : String(cause) });
  }
  if (warning) onRecovery?.(warning);
  return binary;
}

function byteLength(value: string): number { return Buffer.byteLength(value, "utf8"); }

/**
 * Binary-generation check. The extension tracks main ahead of official
 * launcher releases (severed lane), so any binary of the same major is
 * accepted: features gate on envelope content and the index negotiates its
 * schema with the binary. A different major or an unparseable version still
 * fails closed, as does a missing version when identity is required.
 */
function binaryMajor(version: unknown): number | undefined {
  if (typeof version !== "string") return undefined;
  const major = version.match(/^(\d+)\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/)?.[1];
  return major === undefined ? undefined : Number(major);
}

// RUNTIME_VERSION is contract-pinned semver, so this is total. A malformed
// constant (-1) would reject every binary — fail closed.
const RUNTIME_MAJOR: number = binaryMajor(RUNTIME_VERSION) ?? -1;

/** Present-field version checks. Pass `requireIdentity` for version --json. */
function assertVersionTriple(envelope: Partial<MachineEnvelope>, requireIdentity = false): void {
  // Compound guards (same short-circuit as nested if): check only when required or field present.
  if ((requireIdentity || envelope.version !== undefined) && binaryMajor(envelope.version) !== RUNTIME_MAJOR) {
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

export class AstSgrepRuntime {
  readonly watchExternalChanges = true;
  readonly config: ReturnType<typeof resolveConfig>;
  readonly #resolver: BinaryResolver;
  readonly #environment: NodeJS.ProcessEnv;
  readonly #recoverMissingOverride: boolean;
  #binaryWarning: string | undefined;
  constructor(private readonly pi: PiExec, sources: ConfigSources = {}, dependencies: RuntimeDependencies = {}) {
    this.#environment = sources.environment ?? process.env;
    this.config = resolveConfig({ ...sources, environment: this.#environment });
    this.#resolver = dependencies.resolveBinary ?? resolveBinary;
    this.#recoverMissingOverride = ![sources.explicitProjectConfig, sources.projectSettings, sources.globalSettings, sources.defaults]
      .some(source => source?.binaryPath !== undefined);
  }

  async resolveRoot(context: RuntimeContext): Promise<string> {
    return (context as InternalRuntimeContext)[RESOLVED_ROOT]
      ? resolveRuntimeRoot(context.cwd)
      : resolveRuntimeRoot(context.cwd, this.config.root, this.config.allowOutsideProject);
  }
  resolveIndexPath(root: string): string {
    return indexPathFor(root, { ...this.#environment, ...this.config.env });
  }
  /**
   * Index format check. The configured binary is the sole authority on its own
   * schema window (exact-match: it refuses both older and newer), so the local
   * probe is only a pre-filter:
   *   missing/unreadable -> cheap no-spawn health answers;
   *   version == INDEX_FORMAT_VERSION (this release's shipped format) -> ready;
   *   otherwise -> consult the binary's declared index_schema_version once
   *   (cached), because a configured ASGREP_BIN/dev build may be newer than the
   *   shipped constant. Index newer than the binary -> INDEX_VERSION_TOO_NEW
   *   (never modified); older -> "incompatible" and rebuild migrates in place.
   */
  async inspectIndexCompatibility(context: RuntimeContext): Promise<IndexHealth> {
    const root = await this.resolveRoot(context);
    const indexPath = indexPathFor(root, { ...this.#environment, ...this.config.env });
    const version = inspectIndexFile(indexPath);
    if (version === "missing" || version === "incompatible") return version;
    if (version === INDEX_FORMAT_VERSION) return "ready";
    const supported = await this.supportedIndexFormat(context);
    if (version === supported) return "ready";
    if (version > supported) {
      throw new RuntimeError("INDEX_VERSION_TOO_NEW", "Index schema is newer than the configured ast-sgrep binary", {
        actual: version,
        supported,
        rollbackSafe: true,
      });
    }
    return "incompatible";
  }

  /** The configured binary's declared index schema, or this release's shipped floor. */
  #indexFormatProbe: Promise<number> | undefined;
  private supportedIndexFormat(context: RuntimeContext): Promise<number> {
    if (!this.#indexFormatProbe) {
      const probe = this.run(["version", "--json"], context)
        .then((envelope) => {
          const declared = envelope.index_schema_version;
          return typeof declared === "number" && Number.isSafeInteger(declared) && declared > 0
            ? declared
            : INDEX_FORMAT_VERSION;
        });
      this.#indexFormatProbe = probe;
      // A failed probe (binary missing/exec error) must not be cached forever.
      probe.catch(() => {
        if (this.#indexFormatProbe === probe) this.#indexFormatProbe = undefined;
      });
    }
    return this.#indexFormatProbe;
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
      if ((await this.inspectIndexCompatibility(context)) !== "ready") {
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
    const env = this.#mergedEnv(options.env);
    const binary = this.resolveBinaryPath({ env });
    try {
      const execOptions: ExecOptions = { cwd: root, env, timeout };
      if (options.signal) execOptions.signal = options.signal;
      const result = await this.pi.exec(binary, Object.freeze([...args]), execOptions);
      return parseEnvelope(result, this.config.maxOutputBytes);
    } catch (cause) {
      rethrowExecFailure(cause, options, timeout);
    }
  }

  /** environment < config.env < options.env < NO_COLOR — the merge every path shares. */
  #mergedEnv(extra?: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
    return { ...this.#environment, ...this.config.env, ...extra, NO_COLOR: "1" };
  }

  /** Absolute path to the native binary (for sticky serve / stdin batch spawn). */
  resolveBinaryPath(options: { env?: NodeJS.ProcessEnv } = {}): string {
    this.#binaryWarning = undefined;
    return getBinary(this.config, this.#mergedEnv(options.env), this.#resolver,
      this.#recoverMissingOverride ? warning => { this.#binaryWarning = warning; } : undefined);
  }

  binaryWarning(): string | undefined { return this.#binaryWarning; }

  /** Report installed layers even if the project's index cannot be opened. */
  async diagnostics(context: RuntimeContext): Promise<Record<string, unknown>> {
    const versionAt = (path: string | URL): string => {
      try { return JSON.parse(readFileSync(path, "utf8")).version ?? "unknown"; }
      catch { return "unknown"; }
    };
    const result: Record<string, unknown> = {
      extension: versionAt(new URL("../../package.json", import.meta.url)),
    };
    try {
      const launcher = createRequire(import.meta.url).resolve("ast-sgrep");
      result.launcher = versionAt(join(dirname(launcher), "..", "package.json"));
      result.binaryPath = this.resolveBinaryPath();
      result.native = (await this.checkCompatibility(context)).version;
    } catch (cause) {
      result.error = cause instanceof Error ? cause.message : String(cause);
    }
    if (this.#binaryWarning) result.warning = this.#binaryWarning;
    return result;
  }

  /** Merged process env for native Code Mode workers. */
  nativeEnv(options: { env?: NodeJS.ProcessEnv } = {}): NodeJS.ProcessEnv {
    return this.#mergedEnv(options.env);
  }

  async checkCompatibility(context: RuntimeContext, options: RunOptions = {}): Promise<MachineEnvelope> {
    const value = await this.run(["version", "--json"], context, options);
    assertVersionTriple(value, true);
    return value;
  }
}
