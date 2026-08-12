import assert from "node:assert/strict";
import test from "node:test";
import { realpathSync } from "node:fs";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  loadCodemodeNative,
  resetNativeCache,
  nativeAvailable,
} from "../../../packages/pi/extension/src/codemode/native.js";
import { NativeSessionPool } from "../../../packages/pi/extension/src/codemode/session-pool.js";
import { createAsgrepConnector } from "../../../packages/pi/extension/src/codemode/connector.js";
import { runCodemode } from "../../../packages/pi/extension/src/codemode/runner.js";

const here = dirname(fileURLToPath(import.meta.url));
const sample = realpathSync(join(here, "../../../tests/fixtures/sample"));

function requireNative() {
  delete process.env.ASGREP_CODEMODE_BACKEND;
  resetNativeCache();
  const binding = loadCodemodeNative();
  if (!binding) {
    return null;
  }
  return binding;
}

async function indexedNative(binding: NonNullable<ReturnType<typeof requireNative>>): Promise<{ dir: string; indexPath: string }> {
  const dir = await mkdtemp(join(tmpdir(), "asgrep-napi-index-"));
  const indexPath = join(dir, "index.db");
  const session = new binding.Session({ root: sample, indexPath, useEmbed: false, limit: 8 });
  await session.call("index_repo", { force: false });
  return { dir, indexPath };
}

test("NAPI addon loads and reports version", (t) => {
  const binding = requireNative();
  if (!binding) {
    t.skip("native addon not built (npm run build:native)");
    return;
  }
  assert.equal(binding.isNative(), true);
  assert.equal(binding.bindingVersion(), "1.4.0");
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
    assert.equal(status.index_path, join(root, "custom-index", "index.db"));
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
