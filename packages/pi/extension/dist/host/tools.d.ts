/**
 * The four registered pi tools: asgrep (Code Mode), asgrep_search,
 * asgrep_index, asgrep_status — plus the session pool, freshness wiring,
 * and workspace event hooks that serve them.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { type FreshnessLike, type RuntimeLike } from "./results.js";
export declare const DEFAULT_LIMIT = 8;
/**
 * pi ships `read`, `edit`, `write`, `bash`, `grep`, `find`, `ls` built in, so on
 * a normal Pi host our one-shot file tools would be paid for twice and never
 * needed. They stay REGISTERED — an MCP-style host, a `--no-builtin-tools`
 * session, or a host that drops the built-ins still gets them — but they are
 * left out of the active set when the host already provides read+edit. Pi only
 * sends ACTIVE tools (schema, snippet, guidelines) to the model, so this is the
 * difference between ~296 tokens per request and nothing.
 *
 * ASGREP_KEEP_FILE_TOOLS=1 pins them active regardless.
 */
/**
 * Tools that MUTATE the index never ride the warm session: its calls are
 * serialized, so a write there blocks every read queued behind it.
 *
 * Exported so the routing contract is testable without a live session.
 */
export declare function writesOffSession(tool: string): boolean;
export declare function hostProvidesFileTools(pi: ExtensionAPI, env?: NodeJS.ProcessEnv): boolean;
export declare function registerAstSgrepTools(pi: ExtensionAPI, runtime?: RuntimeLike, freshness?: FreshnessLike): void;
