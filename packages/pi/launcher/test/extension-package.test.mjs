import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const extensionDir = resolve(dirname(fileURLToPath(import.meta.url)), "../../extension");

test("packed extension inventory is exact and carries registry integrity", () => {
  const result = spawnSync("npm", ["pack", "--json", "--dry-run"], { cwd: extensionDir, encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  const packed = JSON.parse(result.stdout)[0];
  assert.deepEqual(packed.files.map((file) => file.path).sort(), [
    "LICENSE",
    "README.md",
    "assets/preview.png",
    "dist/index.d.ts",
    "dist/index.js",
    "dist/runtime.d.ts",
    "dist/runtime.js",
    "package.json",
    "skills/ast-sgrep/SKILL.md",
    "skills/ast-sgrep/references/query-guide.md",
  ].sort());
  assert.match(packed.integrity, /^sha512-[A-Za-z0-9+/]+={0,2}$/u);
  assert.match(packed.shasum, /^[0-9a-f]{40}$/u);
});
