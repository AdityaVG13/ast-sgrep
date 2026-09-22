import assert from "node:assert/strict";
import test from "node:test";
import { realpathSync } from "node:fs";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createRequire } from "node:module";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  CODEMODE_BINDING_VERSION,
  loadCodemodeNative,
  resetNativeCache,
  nativeAvailable,
} from "../../../packages/pi/extension/dist/codemode/native.js";
import { NativeSessionPool } from "../../../packages/pi/extension/dist/codemode/session-pool.js";
import { createAsgrepConnector } from "../../../packages/pi/extension/dist/codemode/connector.js";
import { runCodemode } from "../../../packages/pi/extension/dist/codemode/runner.js";

const here = dirname(fileURLToPath(import.meta.url));
const sample = realpathSync(join(here, "../../../tests/fixtures/sample"));

function requireNative() {
  delete process.env.ASGREP_CODEMODE_BACKEND;
  resetNativeCache();
  const binding = loadCodemodeNative();
  const override = process.env.ASGREP_CODEMODE_NAPI_PATH;
  if (override) {
    const expected = createRequire(import.meta.url)(resolve(override));
    assert.equal(binding, expected, "the suite must test the explicitly requested addon, not a fallback");
    assert.ok(binding, "the explicitly requested native addon must load, not silently skip the suite");
  }
  if (!binding) {
    return null;
  }
  return binding;
}

async function indexedNative(
  binding: NonNullable<ReturnType<typeof requireNative>>,
  useEmbed = false,
): Promise<{ dir: string; indexPath: string }> {
  const dir = await mkdtemp(join(tmpdir(), "asgrep-napi-index-"));
  const indexPath = join(dir, "index.db");
  const session = new binding.Session({ root: sample, indexPath, useEmbed, limit: 8 });
  await session.call("index_repo", { force: false });
  return { dir, indexPath };
}

test("native discovery probes the workspace release directory", () => {
  const output = execFileSync(process.execPath, ["--input-type=module", "-e", `
    import fs from 'node:fs';
    import { syncBuiltinESMExports } from 'node:module';
    const seen = [];
    fs.existsSync = path => { seen.push(String(path)); return false; };
    syncBuiltinESMExports();
    delete process.env.ASGREP_CODEMODE_BACKEND;
    delete process.env.ASGREP_CODEMODE_NAPI_PATH;
    delete process.env.CARGO_TARGET_DIR;
    const { loadCodemodeNative } = await import(process.argv[1]);
    loadCodemodeNative();
    console.log(JSON.stringify(seen));
  `, new URL("../../../packages/pi/extension/dist/codemode/native.js", import.meta.url).href], { encoding: "utf8" });
  const searched = JSON.parse(output) as string[];
  assert.ok(searched.includes(resolve(here, "../../..", "target/release/ast-sgrep-codemode.node")), output);
});

test("NAPI addon loads and reports version", (t) => {
  const binding = requireNative();
  if (!binding) {
    t.skip("native addon not built (npm run build:native)");
    return;
  }
  assert.equal(binding.isNative(), true);
  assert.equal(binding.bindingVersion(), CODEMODE_BINDING_VERSION);
  assert.equal(binding.asyncApiVersion(), 1);
});

test("native indexing returns a Promise and does not block the event loop", async (t) => {
  const binding = requireNative();
  if (!binding) {
    t.skip("native addon not built");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "asgrep-napi-async-"));
  const source = join(root, "src");
  await mkdir(source);
  try {
    await Promise.all(Array.from({ length: 500 }, (_, index) =>
      writeFile(join(source, `file-${index}.ts`), `export function value${index}() { return ${index}; }\n`, "utf8")));
    const session = new binding.Session({
      root,
      indexPath: join(root, "index.db"),
      useEmbed: false,
      limit: 8,
    });
    let eventLoopAdvanced = false;
    setImmediate(() => { eventLoopAdvanced = true; });
    const operation = session.call("index_repo", { force: false });
    assert.ok(operation instanceof Promise);
    await operation;
    assert.equal(eventLoopAdvanced, true, "native index work must run off the Node event loop");

    const pool = new NativeSessionPool();
    pool.configure({ useEmbed: false, indexPath: join(root, "index.db") });
    const worker = await pool.acquire(root);
    assert.ok(worker);
    let activeSettled = false;
    const active = worker!.call("index_repo", { force: true }).finally(() => { activeSettled = true; });
    const controller = new AbortController();
    const queued = worker!.call("index_status", {}, { signal: controller.signal });
    let followingSettled = false;
    const following = worker!.call("index_status", {}).finally(() => { followingSettled = true; });
    controller.abort();
    await assert.rejects(queued, { name: "AbortError" });
    assert.equal(activeSettled, false, "queued cancellation must reject before active native work finishes");
    assert.equal(followingSettled, false, "later work must remain behind the active native task");
    await active;
    await following;
    await pool.shutdown();
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("pre-aborted native call and batch reject before executing any tools", async (t) => {
  const binding = requireNative();
  if (!binding) { t.skip("native addon not built"); return; }
  const session = new binding.Session({ root: sample, useEmbed: false });
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(session.call("catalog_search", { query: "search" }, controller.signal), /cancel|abort/i);
  await assert.rejects(session.batch([{ id: "blocked", tool: "catalog_search", args: { query: "search" } }], controller.signal), /cancel|abort/i);
  assert.equal(session.callCount, 0, "pre-aborted work must not reach the catalog");
  await session.call("catalog_search", { query: "search" });
  assert.equal(session.callCount, 1, "the same session must recover for later callers");
});

test("aborting an in-flight native call does not leave the session busy", async (t) => {
  const binding = requireNative();
  if (!binding) {
    t.skip("native addon not built");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "asgrep-napi-abort-busy-"));
  const source = join(root, "src");
  await mkdir(source);
  try {
    await Promise.all(Array.from({ length: 200 }, (_, index) =>
      writeFile(join(source, `file-${index}.ts`), `export function value${index}() { return ${index}; }\n`, "utf8")));
    const session = new binding.Session({
      root,
      indexPath: join(root, "index.db"),
      useEmbed: false,
      limit: 8,
    });
    const controller = new AbortController();
    const active = session.call("index_repo", { force: false }, controller.signal);
    controller.abort();
    await assert.rejects(active, (err: unknown) => {
      const message = err instanceof Error ? err.message : String(err);
      return /cancel|abort/iu.test(message);
    });
    try {
      await session.call("index_status", {});
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      assert.doesNotMatch(
        message,
        /session is busy/iu,
        "aborted work must not fail-closed the pooled session with session is busy",
      );
      throw err;
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("aborting index_repo after it starts stops the walk", async (t) => {
  const binding = requireNative();
  if (!binding) {
    t.skip("native addon not built");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "asgrep-napi-abort-walk-"));
  const source = join(root, "src");
  await mkdir(source);
  try {
    await Promise.all(Array.from({ length: 1_200 }, (_, index) =>
      writeFile(join(source, `file-${index}.ts`), `export function value${index}() { return ${index}; }\n`, "utf8")));
    const session = new binding.Session({
      root,
      indexPath: join(root, "index.db"),
      useEmbed: false,
      limit: 8,
    });
    const controller = new AbortController();
    const started = Date.now();
    let settled = false;
    const active = session.call("index_repo", { force: true }, controller.signal)
      .finally(() => { settled = true; });
    await new Promise((resolve) => setTimeout(resolve, 15));
    if (settled) {
      t.skip("index finished before abort could be observed");
      return;
    }
    controller.abort();
    await assert.rejects(active, (err: unknown) => {
      const message = err instanceof Error ? err.message : String(err);
      return /cancel|abort/iu.test(message);
    });
    assert.ok(
      Date.now() - started < 8_000,
      "cancelled index_repo must stop instead of finishing the tree walk",
    );
    await session.call("index_status", {});
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("native relative index paths resolve against the session root", async (t) => {
  const binding = requireNative();
  if (!binding) {
    t.skip("native addon not built");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "asgrep-napi-relative-index-"));
  try {
    await writeFile(join(root, "source.ts"), "export const relativeIndex = true;\n", "utf8");
    const session = new binding.Session({
      root,
      indexPath: "custom-index",
      useEmbed: false,
      limit: 8,
    });
    await session.call("index_repo", { force: false });
    const status = await session.call("index_status", {}) as Record<string, unknown>;
    assert.equal(status.index_path, join(realpathSync(root), "custom-index", "index.db"));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("session pool uses napi backend", async (t) => {
  const binding = requireNative();
  if (!binding) {
    t.skip("native addon not built");
    return;
  }
  const indexed = await indexedNative(binding);
  t.after(() => rm(indexed.dir, { recursive: true, force: true }));
  assert.equal(nativeAvailable(), true);
  const pool = new NativeSessionPool();
  pool.configure({ useEmbed: false, indexPath: indexed.indexPath });
  const worker = await pool.acquire(sample);
  assert.ok(worker);
  assert.equal(pool.backend(), "napi");
  const envelope = await worker!.call("search", { query: "token", limit: 2, format: "capsule" });
  assert.equal(envelope.tool, "asgrep");
  assert.equal(envelope.ok, true);
  await pool.shutdown();
});

test("only bounded warm lookups use callNow and pool search stays async", async (t) => {
  const binding = requireNative();
  if (!binding) {
    t.skip("native addon not built");
    return;
  }
  const indexed = await indexedNative(binding);
  t.after(() => rm(indexed.dir, { recursive: true, force: true }));
  const session = new binding.Session({ root: sample, indexPath: indexed.indexPath, useEmbed: false, limit: 8 });
  assert.equal(typeof session.callNow, "function", "bounded warm lookups need Session.callNow");
  assert.equal(
    session.callNow!("search", { query: "token", limit: 2, format: "capsule" }),
    null,
    "unique search must not run on the JS thread",
  );
  const status = session.callNow!("index_status", {}) as Record<string, unknown>;
  assert.equal(typeof status, "object");
  const defs = session.callNow!("defs", { symbol: "auth_refresh", limit: 2 }) as Record<string, unknown>;
  assert.ok(Array.isArray(defs.hits));

  await session.call("search", { query: "token", limit: 2, format: "capsule" });
  const cached = session.callNow!("search", { query: "token", limit: 2, format: "capsule" }) as Record<string, unknown>;
  assert.ok(Array.isArray(cached.hits), "sticky search cache hits may use callNow");

  const pool = new NativeSessionPool();
  pool.configure({ useEmbed: false, indexPath: indexed.indexPath });
  const worker = await pool.acquire(sample);
  assert.ok(worker);
  let eventLoopAdvanced = false;
  setImmediate(() => { eventLoopAdvanced = true; });
  const search = worker!.call("search", { query: "token", limit: 2, format: "capsule" });
  assert.equal(eventLoopAdvanced, false, "pool search must not complete synchronously");
  await search;
  assert.equal(eventLoopAdvanced, true, "pool search must dispatch through the async native path");
  await pool.shutdown();
});

test("Code Mode Promise.all stays in-process (no spawn)", async (t) => {
  const binding = requireNative();
  if (!binding) {
    t.skip("native addon not built");
    return;
  }
  const indexed = await indexedNative(binding);
  t.after(() => rm(indexed.dir, { recursive: true, force: true }));
  const pool = new NativeSessionPool();
  pool.configure({ useEmbed: false, indexPath: indexed.indexPath });
  const sticky = await pool.acquire(sample);
  assert.ok(sticky);
  const bundle = createAsgrepConnector({
    run: async () => {
      throw new Error("CLI spawn must not be used when NAPI is available");
    },
    sticky,
  }, { cwd: sample });
  const outcome = await runCodemode(
    `async () => {
      const [a, b] = await Promise.all([
        asgrep.search({ query: "auth", limit: 3 }),
        asgrep.defs({ symbol: "auth_refresh", limit: 3 }),
      ]);
      return { n: (a.hits?.length ?? 0) + (b.hits?.length ?? 0), backend: "napi" };
    }`,
    bundle.asgrep,
    { stats: bundle.stats },
  );
  assert.equal(outcome.ok, true, outcome.ok ? undefined : outcome.error);
  assert.ok((outcome.result as { n: number }).n >= 1);
  assert.ok(bundle.stats().stickyCalls >= 2);
  assert.equal(bundle.stats().parallelSpawnCalls, 0);
  await pool.shutdown();
});

test("Pi-visible unique embed-hybrid search latency", async (t) => {
  const binding = requireNative();
  if (!binding) {
    t.skip("native addon not built (npm run build:native)");
    return;
  }
  const indexed = await indexedNative(binding, true);
  t.after(() => rm(indexed.dir, { recursive: true, force: true }));
  const pool = new NativeSessionPool();
  pool.configure({ useEmbed: true, indexPath: indexed.indexPath, limit: 8 });
  const worker = await pool.acquire(sample);
  assert.ok(worker);
  await worker!.call("search", { query: "warmup probe token", limit: 8, format: "capsule" });

  const queries = [
    "how does auth refresh work",
    "credential renewal",
    "sanitize user input",
    "process inbound request",
    "token refresh flow",
    "validate the session cookie",
    "store durable credentials",
    "rank hybrid search results",
    "debounce noisy file events",
    "retry after a timeout",
    "combine two search channels",
    "remember query embeddings",
  ];
  const uniqueNs: number[] = [];
  for (const query of queries) {
    const t0 = process.hrtime.bigint();
    const envelope = await worker!.call("search", { query, limit: 8, format: "capsule" });
    const ns = Number(process.hrtime.bigint() - t0);
    uniqueNs.push(ns);
    assert.equal(envelope.ok, true, envelope.ok ? undefined : String(envelope));
    console.error(`pi napi unique ${JSON.stringify(query)} ${(ns / 1e6).toFixed(3)}ms`);
  }
  uniqueNs.sort((a, b) => a - b);
  const p50 = uniqueNs[Math.floor(uniqueNs.length / 2)] ?? 0;
  const p100 = uniqueNs[uniqueNs.length - 1] ?? 0;
  console.error(
    `pi napi unique n=${uniqueNs.length} p50=${(p50 / 1e6).toFixed(3)}ms p100=${(p100 / 1e6).toFixed(3)}ms`,
  );
  const r0 = process.hrtime.bigint();
  await worker!.call("search", { query: "how does auth refresh work", limit: 8, format: "capsule" });
  console.error(`pi napi repeat ${((Number(process.hrtime.bigint() - r0)) / 1e6).toFixed(3)}ms`);
  await pool.shutdown();
});
