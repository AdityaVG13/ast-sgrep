/**
 * Slash commands: /asgrep-doctor /asgrep-status /asgrep-index /asgrep-reindex.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { type RuntimeLike } from "./envelope.js";
export declare function registerAstSgrepCommands(pi: ExtensionAPI, runtime?: RuntimeLike): void;
