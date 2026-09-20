import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const extensionDir = resolve(dirname(fileURLToPath(import.meta.url)), "../../../packages/pi/extension");

test("packed extension inventory is exact and carries registry integrity", () => {
  const result = spawnSync("npm", ["pack", "--json", "--dry-run"], { cwd: extensionDir, encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  const packed = JSON.parse(result.stdout)[0];
  assert.deepEqual(packed.files.map((file) => file.path).sort(), [
    "LICENSE",
    "README.md",
    "assets/preview.png",
    "dist/code-mode.d.ts",
    "dist/code-mode.js",
    "dist/codemode/connector.d.ts",
    "dist/codemode/connector.js",
    "dist/codemode/dispatch.d.ts",
    "dist/codemode/dispatch.js",
    "dist/codemode/fallback.d.ts",
    "dist/codemode/fallback.js",
    "dist/codemode/guest-api.d.ts",
    "dist/codemode/guest-api.js",
    "dist/codemode/guest-worker.mjs",
    "dist/codemode/index.d.ts",
    "dist/codemode/index.js",
    "dist/codemode/native.d.ts",
    "dist/codemode/native.js",
    "dist/codemode/runner.d.ts",
    "dist/codemode/runner.js",
    "dist/codemode/session-pool.d.ts",
    "dist/codemode/session-pool.js",
    "dist/codemode/types.d.ts",
    "dist/codemode/types.js",
    "dist/codemode/worker.d.ts",
    "dist/codemode/worker.js",
    "dist/host/commands.d.ts",
    "dist/host/commands.js",
    "dist/host/results.d.ts",
    "dist/host/results.js",
    "dist/host/tools.d.ts",
    "dist/host/tools.js",
    "dist/index.d.ts",
    "dist/index.js",
    "dist/runtime/config.d.ts",
    "dist/runtime/config.js",
    "dist/runtime/freshness.d.ts",
    "dist/runtime/freshness.js",
    "dist/runtime/index-health.d.ts",
    "dist/runtime/index-health.js",
    "dist/runtime/runtime.d.ts",
    "dist/runtime/runtime.js",
    "dist/runtime/sqlite.d.ts",
    "dist/runtime/sqlite.js",
    "dist/runtime/types.d.ts",
    "dist/runtime/types.js",
    "dist/ui/card.d.ts",
    "dist/ui/card.js",
    "dist/ui/present.d.ts",
    "dist/ui/present.js",
    "native/.gitignore",
    "native/README.md",
    "package.json",
  ].sort());
  assert.match(packed.integrity, /^sha512-[A-Za-z0-9+/]+={0,2}$/u);
  assert.match(packed.shasum, /^[0-9a-f]{40}$/u);
});
