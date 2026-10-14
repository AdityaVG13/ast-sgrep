/**
 * p100 harness for the paths a pi-ast-sgrep user actually hits.
 *
 *   node scripts/p100-bench.mjs      (or: npm run bench:p100)
 *
 * Reports p50/p99/p100 per path. Two regressions this exists to catch: a read
 * queueing behind an index write (one serialized warm session), and a cold
 * checkout making the first search wait for the build.
 */
/** p100 for the pathological paths: cold big repo, reindex in flight. */
import { spawn } from "node:child_process";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const extensionUrl = process.env.ASGREP_BENCH_EXTENSION_DIR
  ? pathToFileURL(join(process.env.ASGREP_BENCH_EXTENSION_DIR, "dist") + "/").href
  : new URL("../dist/", import.meta.url).href;
const { registerAstSgrepTools } = await import(new URL("index.js", extensionUrl).href);
const { AstSgrepRuntime, FreshnessCoordinator } = await import(new URL("runtime/runtime.js", extensionUrl).href);

const BINARY = process.env.ASGREP_BIN ?? fileURLToPath(new URL("../../../../target/release/asgrep", import.meta.url));
const keepFixtures = process.env.ASGREP_BENCH_KEEP_FIXTURES === "1";
const measurements = { binary: BINARY, fixtures: [], samples: {} };
const benchmarkEnv = Object.fromEntries(["PATH", "HOME", "TMPDIR", "RAYON_NUM_THREADS", "ASGREP_CODEMODE_NAPI_PATH"].filter((key) => process.env[key] !== undefined).map((key) => [key, process.env[key]]));
const pct = (s, p) => { const x = [...s].sort((a, b) => a - b); return x[Math.min(x.length - 1, Math.max(0, Math.ceil((p / 100) * x.length) - 1))]; };
const piExec = {
  exec(command, args, options) {
    return new Promise((resolve) => {
      const child = spawn(command, [...args], { cwd: options.cwd, env: options.env, signal: options.signal, stdio: ["ignore", "pipe", "pipe"] });
      let stdout = "", stderr = "";
      const timer = setTimeout(() => child.kill("SIGKILL"), options.timeout ?? 30000);
      child.stdout.on("data", (c) => { stdout += String(c); });
      child.stderr.on("data", (c) => { stderr += String(c); });
      child.on("close", (code, signal) => { clearTimeout(timer); resolve({ stdout, stderr, exitCode: code ?? -1, signal }); });
      child.on("error", (e) => { clearTimeout(timer); resolve({ stdout: "", stderr: String(e), exitCode: -1 }); });
    });
  },
};

const root = await mkdtemp(join(tmpdir(), "asgrep-cold-"));
await mkdir(join(root, ".git"));
measurements.fixtures.push(root);
const FILES = 3000;
for (let i = 0; i < FILES; i += 1) {
  const dir = join(root, "src", `mod${i % 20}`);
  await mkdir(dir, { recursive: true });
  await writeFile(join(dir, `f${i}.rs`), `pub fn symbol_${i}(x: &str) -> usize { x.len() + ${i} }\npub fn helper_${i}() -> usize { ${i} }\n`);
}

const tools = [];
const rootHandlers = new Map();
const api = { registerTool(t) { tools.push(t); }, on(event, handler) { rootHandlers.set(event, handler); }, exec: (...a) => piExec.exec(...a), getActiveTools: () => ["read", "edit"], setActiveTools() {} };
const runtime = new AstSgrepRuntime(api, { environment: benchmarkEnv, explicitProjectConfig: { root, allowOutsideProject: true } }, { resolveBinary: () => BINARY });
registerAstSgrepTools(api, runtime, new FreshnessCoordinator({ refreshIntervalMs: 30_000 }));
const byName = (name) => tools.find((t) => t.name === name);
const call = async (tool, params) => {
  const started = performance.now();
  const out = await tool.execute("c", params, new AbortController().signal, () => {}, { cwd: root });
  return { ms: performance.now() - started, out };
};

const search = byName("asgrep_search");
console.log("cold repo:", FILES, "files, no index");
const first = await call(search, { query: "symbol_1500" });
if (!first.out.details.ok) throw new Error("cold search failed: " + JSON.stringify(first.out));
measurements.samples.first_search_ms = first.ms;
console.log("  first search      ", first.ms.toFixed(0), "ms | ok=" + first.out.details.ok, "| text:", JSON.stringify(first.out.content[0].text.split("\n").slice(0, 2).join(" | ")).slice(0, 160));
const second = await call(search, { query: "symbol_1500" });
if (!second.out.details.ok) throw new Error("second search failed: " + JSON.stringify(second.out));
measurements.samples.second_search_ms = second.ms;
console.log("  second search     ", second.ms.toFixed(0), "ms | hits line:", JSON.stringify(second.out.content[0].text.split("\n")[0]));
const warm = [];
for (let i = 0; i < 30; i += 1) {
  const reply = await call(search, { query: `symbol_${100 + i}` });
  if (!reply.out.details.ok) throw new Error("warm search failed: " + JSON.stringify(reply.out));
  warm.push(reply.ms);
}
measurements.samples.warm_search_ms = warm;
console.log("  warm p50/p99/p100 ", pct(warm, 50).toFixed(1), "/", pct(warm, 99).toFixed(1), "/", Math.max(...warm).toFixed(1), "ms");

// Full reindex in flight: does a search queue behind it?
const indexTool = byName("asgrep_index");
const reindex = call(indexTool, { force: true });
await new Promise((r) => setTimeout(r, 120));
const concurrent = [];
for (let i = 0; i < 8; i += 1) {
  const reply = await call(search, { query: `helper_${i * 7}` });
  if (!reply.out.details.ok) throw new Error("concurrent search failed: " + JSON.stringify(reply.out));
  concurrent.push(reply.ms);
}
measurements.samples.during_reindex_ms = concurrent;
console.log("search during reindex  p50/p100 ", pct(concurrent, 50).toFixed(1), "/", Math.max(...concurrent).toFixed(1), "ms");
const re = await reindex;
if (!re.out.details.ok) throw new Error("reindex failed: " + JSON.stringify(re.out));
measurements.samples.reindex_ms = re.ms;
console.log("  reindex total     ", re.ms.toFixed(0), "ms | ok=" + re.out.details.ok);

await rootHandlers.get("session_shutdown")?.({}, { cwd: root });
if (!keepFixtures) await rm(root, { recursive: true, force: true });

// Cold checkout with the session-start warmup: the first search must not wait.
for (const files of [300, 3000]) {
  const cold = await mkdtemp(join(tmpdir(), "asgrep-p100-cold-"));
  await mkdir(join(cold, ".git"));
  measurements.fixtures.push(cold);
  for (let i = 0; i < files; i += 1) {
    const dir = join(cold, "src", `mod${i % 10}`);
    await mkdir(dir, { recursive: true });
    await writeFile(join(dir, `file${i}.rs`), `pub fn symbol_${i}() -> usize { ${i} }\n`);
  }
  const tools = [];
  const handlers = new Map();
  const api = {
    registerTool(tool) { tools.push(tool); },
    on(event, handler) { handlers.set(event, handler); },
    exec: (...args) => piExec.exec(...args),
    getActiveTools: () => ["read", "edit"],
    setActiveTools() {},
  };
  const runtime = new AstSgrepRuntime(api, { environment: benchmarkEnv, explicitProjectConfig: { root: cold, allowOutsideProject: true } }, { resolveBinary: () => BINARY });
  registerAstSgrepTools(api, runtime, new FreshnessCoordinator({ refreshIntervalMs: 30_000 }));
  const search = tools.find((tool) => tool.name === "asgrep_search");
  const warmStart = performance.now();
  handlers.get("session_start")?.({}, { cwd: cold });
  await new Promise((resolve) => setTimeout(resolve, 600));
  const firstStart = performance.now();
  const first = await search.execute("c", { query: "symbol_7" }, new AbortController().signal, () => {}, { cwd: cold });
  const firstMs = performance.now() - firstStart;
  if (!first.details.ok) throw new Error("startup search failed: " + JSON.stringify(first));
  measurements.samples[`startup_${files}_first_ms`] = firstMs;
  console.log(
    ("cold " + files + " files, first search").padEnd(34),
    "warmup " + (firstStart - warmStart).toFixed(0) + "ms, first=" + firstMs.toFixed(1) + " ms |",
    first.content[0].text.split("\n")[0],
  );
  await handlers.get("session_shutdown")?.({}, { cwd: cold });
  if (!keepFixtures) await rm(cold, { recursive: true, force: true });
}
if (process.env.ASGREP_BENCH_JSON_PATH) await writeFile(process.env.ASGREP_BENCH_JSON_PATH, JSON.stringify(measurements, null, 2) + "\n");
console.log("retained fixtures:", keepFixtures ? measurements.fixtures.join(", ") : "none");

