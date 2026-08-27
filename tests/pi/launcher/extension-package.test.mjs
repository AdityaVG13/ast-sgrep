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
    "dist/index.d.ts",
    "dist/index.js",
    "dist/present.d.ts",
    "dist/present.js",
    "dist/runtime.d.ts",
    "dist/runtime.js",
    "dist/sqlite.d.ts",
    "dist/sqlite.js",
    "native/.gitignore",
    "native/README.md",
    "package.json",
  ].sort());
  assert.match(packed.integrity, /^sha512-[A-Za-z0-9+/]+={0,2}$/u);
  assert.match(packed.shasum, /^[0-9a-f]{40}$/u);
});
