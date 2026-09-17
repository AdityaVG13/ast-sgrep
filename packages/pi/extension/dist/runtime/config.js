/**
 * Runtime configuration: schema, legacy migration, env overlay, precedence.
 * Leaf module (imports types only).
 */
import { CONFIG_SCHEMA_VERSION, DEFAULT_MAX_OUTPUT_BYTES, DEFAULT_REFRESH_INTERVAL_MS, DEFAULT_TIMEOUT_MS, RuntimeError, } from "./types.js";
export function finitePositive(value, fallback, name) {
    if (value === undefined)
        return fallback;
    if (typeof value !== "number" || !Number.isSafeInteger(value) || value <= 0) {
        throw new RuntimeError("INVALID_CONFIG", `${name} must be a positive integer`);
    }
    return value;
}
function sameSetting(current, legacy, currentName, legacyName) {
    if (current !== undefined && legacy !== undefined && current !== legacy) {
        throw new RuntimeError("CONFIG_MIGRATION_CONFLICT", `Conflicting ${currentName} and legacy ${legacyName} values`, { currentName, legacyName });
    }
    return current ?? legacy;
}
const LEGACY_NUMBER_FIELDS = [
    ["timeoutMs", "timeout"],
    ["maxOutputBytes", "maxOutput"],
    ["refreshIntervalMs", "refreshInterval"],
];
/** Convert schema 0/unversioned settings without mutating the rollback source. */
export function migrateConfig(input = {}) {
    const value = { ...input };
    const schema = value.schemaVersion ?? 0;
    if (schema !== 0 && schema !== CONFIG_SCHEMA_VERSION) {
        throw new RuntimeError("CONFIG_VERSION_MISMATCH", "Unsupported ast-sgrep configuration schema", { supported: [0, CONFIG_SCHEMA_VERSION], actual: schema, rollbackSafe: true });
    }
    if (schema === CONFIG_SCHEMA_VERSION)
        return value;
    const legacy = value;
    const migrated = { ...legacy, schemaVersion: CONFIG_SCHEMA_VERSION };
    for (const [currentName, legacyName] of LEGACY_NUMBER_FIELDS) {
        const next = sameSetting(value[currentName], legacy[legacyName], currentName, legacyName);
        if (next !== undefined)
            migrated[currentName] = next;
    }
    for (const [, legacyName] of LEGACY_NUMBER_FIELDS) {
        delete migrated[legacyName];
    }
    return migrated;
}
/** Serialize current settings for a schema-0 rollback without mutating the current value. */
export function rollbackConfig(input) {
    const current = migrateConfig(input);
    const legacy = { ...current, schemaVersion: 0 };
    for (const [currentName, legacyName] of LEGACY_NUMBER_FIELDS) {
        const value = current[currentName];
        if (value !== undefined)
            legacy[legacyName] = value;
        delete legacy[currentName];
    }
    return legacy;
}
/** env var → config key. ASGREP_BIN also accepts the launcher's legacy alias. */
const ENV_VARS = [
    ["root", "ASGREP_ROOT", (v) => v],
    ["timeoutMs", "ASGREP_TIMEOUT_MS", Number],
    ["maxOutputBytes", "ASGREP_MAX_OUTPUT_BYTES", Number],
    ["refreshIntervalMs", "ASGREP_REFRESH_INTERVAL_MS", Number],
];
function envConfig(env = {}) {
    const result = {};
    // Canonical: ASGREP_BIN; alias AST_SGREP_BINARY (launcher historical name).
    const bin = env.ASGREP_BIN || env.AST_SGREP_BINARY;
    if (bin)
        result.binaryPath = bin;
    for (const [key, name, parse] of ENV_VARS) {
        const raw = env[name];
        if (raw !== undefined && raw !== "") {
            result[key] = parse(raw);
        }
    }
    return result;
}
/** Merge each setting independently, from the documented lowest to highest priority. */
export function resolveConfig(sources = {}) {
    const merged = {
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
    return merged;
}
