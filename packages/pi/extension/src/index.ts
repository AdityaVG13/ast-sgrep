/**
 * pi-ast-sgrep extension entry: constructs the runtime + freshness
 * coordinator and registers tools + commands. Implementation lives in
 * tools.ts / commands.ts; result plumbing in envelope.ts.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { AstSgrepRuntime, FreshnessCoordinator } from "./runtime.js";
import { registerAstSgrepTools } from "./tools.js";
import { registerAstSgrepCommands } from "./commands.js";

export { registerAstSgrepTools } from "./tools.js";
export { registerAstSgrepCommands } from "./commands.js";

export default function astSgrepExtension(pi: ExtensionAPI): void {
  const runtime = new AstSgrepRuntime(pi);
  const freshness = new FreshnessCoordinator({ refreshIntervalMs: runtime.config.refreshIntervalMs! });
  registerAstSgrepTools(pi, runtime, freshness);
  registerAstSgrepCommands(pi, runtime);
}
