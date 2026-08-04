/**
 * Code Mode for ast-sgrep in Pi.
 *
 * Pattern (Cloudflare / Anthropic PTC / OpenAI PTC):
 * the model writes JavaScript that calls typed `asgrep.*` methods inside a
 * restricted executor. Intermediate results stay in the sandbox; only the
 * shaped return value re-enters the model context. Parallel calls use
 * `Promise.all` and are coalesced into sticky serve / one warm batch process.
 *
 * This package is intentionally independent of MCP. Both MCP and Code Mode
 * sit on the native ast-sgrep binary / core; they never import each other.
 */
export { createAsgrepConnector, } from "./connector.js";
export { runCodemode, normalizeCode } from "./sandbox.js";
export { CODEMODE_TYPES_FOR_MODEL } from "./types.js";
export { createCodemodeDispatcher, runNativeBatch, argvFor, asEnvelope, } from "./dispatch.js";
export { startStickyWorker, runBatchViaStdin } from "./worker.js";
export { NativeSessionPool, sharedNativePool } from "./session-pool.js";
export { loadCodemodeNative, nativeAvailable, resetNativeCache, } from "./native.js";
