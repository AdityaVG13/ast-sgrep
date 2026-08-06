import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { FreshnessCoordinator, type MachineEnvelope, type RunOptions } from "./runtime.js";
type RuntimeLike = {
    run(args: readonly string[], context: {
        cwd: string;
    }, options?: RunOptions): Promise<MachineEnvelope>;
    resolveRoot?(context: {
        cwd: string;
    }): Promise<string>;
    resolveBinaryPath?(options?: {
        env?: NodeJS.ProcessEnv;
    }): string;
    nativeEnv?(options?: {
        env?: NodeJS.ProcessEnv;
    }): NodeJS.ProcessEnv;
    config?: {
        timeoutMs?: number;
        refreshIntervalMs?: number;
    };
    inspectIndexCompatibility?(context: {
        cwd: string;
    }): Promise<"ready" | "missing" | "incompatible">;
    rebuildIncompatibleIndex?(context: {
        cwd: string;
    }, options?: RunOptions): Promise<MachineEnvelope>;
};
type FreshnessLike = Pick<FreshnessCoordinator, "ensureFresh" | "markAffectedPath">;
export declare function registerAstSgrepTools(pi: ExtensionAPI, runtime?: RuntimeLike, freshness?: FreshnessLike): void;
export declare function registerAstSgrepCommands(pi: ExtensionAPI, runtime?: RuntimeLike): void;
export default function astSgrepExtension(pi: ExtensionAPI): void;
export {};
