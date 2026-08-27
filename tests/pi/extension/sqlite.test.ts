import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, it } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import { INDEX_FORMAT_VERSION } from "../../../packages/pi/extension/src/runtime.js";
import { openIndexDatabase, sqliteBackend } from "../../../packages/pi/extension/src/sqlite.js";

const temporary: string[] = [];
afterEach(async () => {
  await Promise.all(temporary.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});

const here = dirname(fileURLToPath(import.meta.url));
const runtimeSource = join(here, "../../../packages/pi/extension/src/runtime.ts");
const runtimeDist = join(here, "../../../packages/pi/extension/dist/runtime.js");

describe("sqlite backend", () => {
  it("selects node:sqlite on Node and bun:sqlite when Bun is the host", () => {
    const expected = typeof (process.versions as NodeJS.ProcessVersions & { bun?: string }).bun === "string"
      ? "bun"
      : "node";
    assert.equal(sqliteBackend(), expected);
  });

  it("reads and writes PRAGMA user_version through the shared adapter", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pi-asgrep-sqlite-"));
    temporary.push(dir);
    const path = join(dir, "index.db");
    const written = openIndexDatabase(path);
    try {
      written.exec(`PRAGMA user_version = ${INDEX_FORMAT_VERSION}`);
    } finally {
      written.close();
    }
    const read = openIndexDatabase(path, { readOnly: true });
    try {
      const row = read.prepare("PRAGMA user_version").get() as Record<string, unknown>;
      assert.equal(Number(Object.values(row)[0]), INDEX_FORMAT_VERSION);
    } finally {
      read.close();
    }
  });

  it("does not statically import node:sqlite from the published runtime entry", async () => {
    const sources = [runtimeSource, runtimeDist];
    for (const path of sources) {
      const text = await readFile(path, "utf8");
      assert.doesNotMatch(text, /from ["']node:sqlite["']/u, path);
    }
  });

  it("imports the runtime under Bun when bun is installed", () => {
    const probe = spawnSync("bun", ["--version"], { encoding: "utf8" });
    if (probe.status !== 0) return;
    const href = pathToFileURL(runtimeSource).href;
    const result = spawnSync("bun", ["--eval", `await import(${JSON.stringify(href)});`], {
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr || result.stdout);
  });
});
