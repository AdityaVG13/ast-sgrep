import assert from "node:assert/strict";
import test from "node:test";
import { mkdtempSync, writeFileSync, rmSync, accessSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { resolveBinary } from "../src/index.js";

function makeExe() {
  const dir = mkdtempSync(join(tmpdir(), "asgrep-bin-"));
  const path = join(dir, "fake-asgrep");
  writeFileSync(path, "#!/bin/sh\n", { mode: 0o755 });
  return { dir, path };
}

test("ASGREP_BIN and AST_SGREP_BINARY both resolve override", () => {
  const a = makeExe();
  const b = makeExe();
  try {
    const fs = { accessSync, readFileSync, statSync };
    assert.equal(resolveBinary({ env: { ASGREP_BIN: a.path }, fs, platform: "darwin" }), a.path);
    assert.equal(resolveBinary({ env: { AST_SGREP_BINARY: b.path }, fs, platform: "darwin" }), b.path);
    assert.equal(resolveBinary({ env: { ASGREP_BIN: a.path, AST_SGREP_BINARY: b.path }, fs, platform: "darwin" }), a.path);
  } finally {
    rmSync(a.dir, { recursive: true, force: true });
    rmSync(b.dir, { recursive: true, force: true });
  }
});
