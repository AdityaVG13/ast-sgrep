/**
 * Runtime configuration: schema, legacy migration, env overlay, precedence.
 * Leaf module (imports types only).
 */
import {
  CONFIG_SCHEMA_VERSION,
  DEFAULT_MAX_OUTPUT_BYTES,
  DEFAULT_REFRESH_INTERVAL_MS,
  DEFAULT_TIMEOUT_MS,
  RuntimeError,
} from "./types.js";

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

export function finitePositive(value: unknown, fallback: number, name: string): number {
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

/** env var → config key. ASGREP_BIN also accepts the launcher's legacy alias. */
const ENV_VARS: ReadonlyArray<readonly [keyof RuntimeConfig, string, (raw: string) => unknown]> = [
  ["root", "ASGREP_ROOT", (v) => v],
  ["timeoutMs", "ASGREP_TIMEOUT_MS", Number],
  ["maxOutputBytes", "ASGREP_MAX_OUTPUT_BYTES", Number],
  ["refreshIntervalMs", "ASGREP_REFRESH_INTERVAL_MS", Number],
];

function envConfig(env: NodeJS.ProcessEnv = {}): RuntimeConfig {
  const result: RuntimeConfig = {};
  // Canonical: ASGREP_BIN; alias AST_SGREP_BINARY (launcher historical name).
  const bin = env.ASGREP_BIN || env.AST_SGREP_BINARY;
  if (bin) result.binaryPath = bin;
  for (const [key, name, parse] of ENV_VARS) {
    const raw = env[name];
    if (raw !== undefined && raw !== "") {
      (result as Record<string, unknown>)[key] = parse(raw);
    }
  }
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
