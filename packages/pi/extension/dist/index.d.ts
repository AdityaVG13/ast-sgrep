import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { FreshnessCoordinator, type FreshnessRuntime } from "./runtime.js";
type RuntimeLike = FreshnessRuntime;
type FreshnessLike = Pick<FreshnessCoordinator, "ensureFresh" | "markAffectedPath">;
export declare function registerAstSgrepTools(pi: ExtensionAPI, runtime?: RuntimeLike, freshness?: FreshnessLike): void;
export declare function registerAstSgrepCommands(pi: ExtensionAPI, runtime?: RuntimeLike): void;
export default function astSgrepExtension(pi: ExtensionAPI): void;
export {};
