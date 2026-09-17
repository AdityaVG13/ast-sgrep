/**
 * pi-ast-sgrep extension entry: constructs the runtime + freshness
 * coordinator and registers tools + commands. Implementation lives in
 * tools.ts / commands.ts; result plumbing in envelope.ts.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
export { registerAstSgrepTools } from "./tools.js";
export { registerAstSgrepCommands } from "./commands.js";
export default function astSgrepExtension(pi: ExtensionAPI): void;
