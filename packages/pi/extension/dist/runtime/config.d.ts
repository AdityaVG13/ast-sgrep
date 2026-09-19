/**
 * Runtime configuration: schema, legacy migration, env overlay, precedence.
 * Leaf module (imports types only).
 */
import { CONFIG_SCHEMA_VERSION } from "./types.js";
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
export declare function finitePositive(value: unknown, fallback: number, name: string): number;
/** Convert schema 0/unversioned settings without mutating the rollback source. */
export declare function migrateConfig(input?: RuntimeConfigInput): RuntimeConfig;
/** Serialize current settings for a schema-0 rollback without mutating the current value. */
export declare function rollbackConfig(input: RuntimeConfig): LegacyRuntimeConfig;
/** Merge each setting independently, from the documented lowest to highest priority. */
export declare function resolveConfig(sources?: ConfigSources): Required<Pick<RuntimeConfig, "timeoutMs" | "maxOutputBytes">> & RuntimeConfig;
