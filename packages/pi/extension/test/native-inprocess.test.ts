import assert from "node:assert/strict";
import test from "node:test";
import { existsSync, realpathSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  loadCodemodeNative,
  resetNativeCache,
  nativeAvailable,
} from "../src/codemode/native.js";
import { NativeSessionPool } from "../src/codemode/session-pool.js";
import { createAsgrepConnector } from "../src/codemode/connector.js";
import { runCodemode } from "../src/codemode/runner.js";

const here = dirname(fileURLToPath(import.meta.url));
const sample = realpathSync(join(here, "../../../../tests/fixtures/sample"));
const indexPath = "/tmp/napi-cm.db";

function requireNative() {
  delete process.env.ASGREP_CODEMODE_BACKEND;
  resetNativeCache();
  const binding = loadCodemodeNative();
  if (!binding) {
    return null;
  }
  return binding;
}

test("NAPI addon loads and reports version", (t) => {
  const binding = requireNative();
  if (!binding) {
    t.skip("native addon not built (npm run build:native)");
    return;
  }
  assert.equal(binding.isNative(), true);
  assert.equal(binding.bindingVersion(), "1.4.0");
});

test("in-process session search is sub-millisecond warm", (t) => {
  const binding = requireNative();
  if (!binding) {
    t.skip("native addon not built");
    return;
  }
  if (!existsSync(indexPath)) {
    t.skip(`index missing at ${indexPath}`);
    return;
  }
  const session = new binding.Session({ root: sample, indexPath, useEmbed: false, limit: 8 });
  session.call("search", { query: "auth", limit: 3, format: "capsule" });
  const t0 = Date.now();
  for (let i = 0; i < 20; i++) {
    session.call("search", { query: "auth", limit: 3, format: "capsule" });
  }
  const ms = Date.now() - t0;
  assert.ok(ms < 50, `20 warm in-process searches should be <50ms, got ${ms}ms`);
});

test("session pool uses napi backend", async (t) => {
  const binding = requireNative();
  if (!binding) {
    t.skip("native addon not built");
    return;
  }
  assert.equal(nativeAvailable(), true);
  const pool = new NativeSessionPool();
  pool.configure({ useEmbed: false, indexPath });
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
  const pool = new NativeSessionPool();
  pool.configure({ useEmbed: false, indexPath });
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
  assert.equal(outcome.ok, true, outcome.error);
  assert.ok((outcome.result as { n: number }).n >= 1);
  assert.ok(bundle.stats().stickyCalls >= 2);
  assert.equal(bundle.stats().parallelSpawnCalls, 0);
  await pool.shutdown();
});
