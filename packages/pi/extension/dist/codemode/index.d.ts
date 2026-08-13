/**
 * Code Mode for ast-sgrep in Pi.
 *
 * Pattern (Cloudflare / Anthropic PTC / OpenAI PTC / OpenCode):
 * the model writes JavaScript that calls typed `asgrep.*` methods. Intermediate
 * results stay in the program; only the shaped return value re-enters the model
 * context. Parallel calls use `Promise.all` against one warm in-process session.
 *
 * Code Mode and MCP are sibling front ends on the same core — pick one per
 * client. They never import each other. Do not install both for the same agent.
 */
export { createAsgrepConnector, type AsgrepConnector, type ConnectorHost, type DispatchSurface, type ConnectorBundle, } from "./connector.js";
export { runCodemode, normalizeCode, type CodemodeRunResult, type CodemodeRunSuccess, type CodemodeRunFailure } from "./runner.js";
export { CODEMODE_TYPES_FOR_MODEL, type SearchArgs, type ChainArgs } from "./types.js";
export { createCodemodeDispatcher, runNativeBatch, argvFor, asEnvelope, type DispatchStats, type BatchCapableHost, type StickyWorker, type BatchResult, } from "./dispatch.js";
export { startStickyWorker, runBatchViaStdin } from "./worker.js";
export { NativeSessionPool, sharedNativePool } from "./session-pool.js";
export { loadCodemodeNative, nativeAvailable, resetNativeCache, type CodemodeNativeBinding, type NativeSession, } from "./native.js";
