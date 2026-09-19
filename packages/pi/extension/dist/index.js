import { AstSgrepRuntime, FreshnessCoordinator } from "./runtime/runtime.js";
import { registerAstSgrepTools } from "./host/tools.js";
import { registerAstSgrepCommands } from "./host/commands.js";
export { registerAstSgrepTools } from "./host/tools.js";
export { registerAstSgrepCommands } from "./host/commands.js";
export default function astSgrepExtension(pi) {
    const runtime = new AstSgrepRuntime(pi);
    const freshness = new FreshnessCoordinator({ refreshIntervalMs: runtime.config.refreshIntervalMs });
    registerAstSgrepTools(pi, runtime, freshness);
    registerAstSgrepCommands(pi, runtime);
}
