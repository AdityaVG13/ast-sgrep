/**
 * The four registered pi tools: asgrep (Code Mode), asgrep_search,
 * asgrep_index, asgrep_status — plus the session pool, freshness wiring,
 * and workspace event hooks that serve them.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { type FreshnessLike, type RuntimeLike } from "./results.js";
export declare const DEFAULT_LIMIT = 8;
export declare function registerAstSgrepTools(pi: ExtensionAPI, runtime?: RuntimeLike, freshness?: FreshnessLike): void;
