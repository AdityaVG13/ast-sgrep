/**
 * Code Mode for ast-sgrep in Pi.
 *
 * Pattern (Cloudflare / Anthropic PTC / OpenAI PTC):
 * the model writes JavaScript that calls typed `asgrep.*` methods inside a
 * restricted executor. Intermediate results stay in the sandbox; only the
 * shaped return value re-enters the model context. Parallel calls use
 * `Promise.all` and are coalesced into one warm batch process when possible.
 *
 * This package is intentionally independent of MCP. Both MCP and Code Mode
 * sit on the native ast-sgrep binary / core; they never import each other.
 */

export {
  createAsgrepConnector,
  type AsgrepConnector,
  type ConnectorHost,
  type ConnectorBundle,
} from "./connector.js";
export { runCodemode, normalizeCode, type CodemodeRunResult } from "./sandbox.js";
export { CODEMODE_TYPES_FOR_MODEL, type SearchArgs, type ChainArgs } from "./types.js";
export {
  createCodemodeDispatcher,
  runNativeBatch,
  type DispatchStats,
  type BatchCapableHost,
} from "./dispatch.js";
