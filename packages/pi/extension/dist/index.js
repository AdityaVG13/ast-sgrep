import { AstSgrepRuntime, FreshnessCoordinator } from "./runtime.js";
import { registerAstSgrepTools } from "./tools.js";
import { registerAstSgrepCommands } from "./commands.js";
export { registerAstSgrepTools } from "./tools.js";
export { registerAstSgrepCommands } from "./commands.js";
export default function astSgrepExtension(pi) {
    const runtime = new AstSgrepRuntime(pi);
    const freshness = new FreshnessCoordinator({ refreshIntervalMs: runtime.config.refreshIntervalMs });
    registerAstSgrepTools(pi, runtime, freshness);
    registerAstSgrepCommands(pi, runtime);
}
