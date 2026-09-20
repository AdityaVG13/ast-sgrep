import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test } from "node:test";
import { createAsgrepConnector } from "../../../packages/pi/extension/src/codemode/connector.js";
import type { MachineEnvelope } from "../../../packages/pi/extension/src/runtime/runtime.js";

/**
 * Severed-lane fallback: launchers that predate the read/edit/find native
 * tools answer `unknown tool: <name>`. The connector must serve those calls
 * from an extension-side implementation with native-identical shapes and
 * error text, so extension-ahead dogfooding works on official launchers.
 */

const temporary: string[] = [];
afterEach(async () => { await Promise.all(temporary.splice(0).map((path) => rm(path, { recursive: true, force: true }))); });
async function project(): Promise<string> {
  const base = await mkdtemp(join(tmpdir(), "pi-fallback-")); temporary.push(base);
  const root = join(base, "project"); await mkdir(root, { recursive: true });
  return realpath(root);
}
async function write(root: string, name: string, content: string): Promise<void> {
  await writeFile(join(root, name), content);
}

/** Simulates an official launcher without read/edit/find (e.g. 2.0.0). */
function oldLauncher(failWith?: (tool: string) => Error | undefined): { host: never; calls: string[]; argv: string[][] } {
  const calls: string[] = [];
  const argv: string[][] = [];
  const unknown = (tool: string): Error => new Error(`unknown tool: ${tool}. Use search, semantic, chain, defs, callers, imports.`);
  const host = {
    sticky: {
      async call(tool: string): Promise<MachineEnvelope> {
        calls.push(tool);
        const forced = failWith?.(tool);
        if (forced) throw forced;
        if (tool === "read" || tool === "edit" || tool === "find") throw unknown(tool);
        return { tool: "asgrep", schema_version: "1.0.0", ok: true, command: tool, hits: [] };
      },
      async batch(requests: Array<{ id: string; tool: string }>): Promise<{ results: Array<{ id: string; ok: boolean; value?: unknown; error?: string }> }> {
        return {
          results: requests.map((request) => {
            calls.push(request.tool);
            const forced = failWith?.(request.tool);
            if (forced) return { id: request.id, ok: false, error: forced.message };
            if (request.tool === "read" || request.tool === "edit" || request.tool === "find") {
              return { id: request.id, ok: false, error: unknown(request.tool).message };
            }
            return { id: request.id, ok: true, value: { ok: true, command: request.tool, hits: [] } };
          }),
        };
      },
      async end(): Promise<void> {},
    },
    async run(args: readonly string[]): Promise<MachineEnvelope> {
      argv.push([...args]);
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, command: "search", hits: [] };
    },
  };
  return { host: host as never, calls, argv };
}

test("read falls back to a native-shaped window when the launcher lacks the tool", async () => {
  const root = await project();
  await write(root, "a.ts", "one\ntwo\nthree\nfour\n");
  const old = oldLauncher();
  const bundle = createAsgrepConnector(old.host, { cwd: root });
  const response = await bundle.asgrep.read({ path: "a.ts", start: 2, end: 3 });
  assert.equal(response.ok, true);
  assert.deepEqual(response.windows, [{ path: "a.ts", ref: "a.ts#L2-L3", start: 2, end: 3, truncated: false, text: "two\nthree" }]);
  assert.equal(response.count, 1);
  assert.deepEqual(old.calls, ["read"]);
});

test("read fallback mirrors native ref forms, defaults, and char budget", async () => {
  const root = await project();
  await write(root, "a.ts", "one\ntwo\nthree\nfour\n");
  const old = oldLauncher();
  const bundle = createAsgrepConnector(old.host, { cwd: root });
  const byRef = await bundle.asgrep.read({ ref: "a.ts#L2-L3" });
  assert.deepEqual((byRef.windows as Array<Record<string, unknown>>)[0], { path: "a.ts", ref: "a.ts#L2-L3", start: 2, end: 3, truncated: false, text: "two\nthree" });
  const barePath = await bundle.asgrep.read({ ref: "a.ts" });
  assert.equal((barePath.windows as Array<Record<string, unknown>>)[0]?.ref, "a.ts#L1-L4");
  const budgeted = await bundle.asgrep.read({ path: "a.ts", start: 1, end: 4, maxChars: 5 });
  assert.equal((budgeted.windows as Array<Record<string, unknown>>)[0]?.truncated, true);
});

test("read fallback jails paths exactly like native", async () => {
  const root = await project();
  await write(root, "a.ts", "one\n");
  const old = oldLauncher();
  const bundle = createAsgrepConnector(old.host, { cwd: root });
  await assert.rejects(bundle.asgrep.read({ path: "../outside.ts" }), /path must not contain '\.\.'/);
  await assert.rejects(bundle.asgrep.read({ path: "missing.ts" }), /cannot resolve path missing\.ts/);
  await mkdir(join(root, "adir"));
  await assert.rejects(bundle.asgrep.read({ path: "adir" }), /cannot read adir/);
  // Beyond-EOF is an empty window, not an error (native read_one_window).
  const pastEnd = await bundle.asgrep.read({ path: "a.ts", start: 99, end: 100 });
  assert.deepEqual(pastEnd.windows, [{ path: "a.ts", ref: "a.ts#L99-L99", start: 99, end: 99, truncated: false, text: "" }]);
});

test("edit falls back to exact-once replace with a native-shaped echo", async () => {
  const root = await project();
  await write(root, "a.ts", "const x = 1;\nconst y = 2;\n");
  const old = oldLauncher();
  const bundle = createAsgrepConnector(old.host, { cwd: root });
  const response = await bundle.asgrep.edit({ path: "a.ts", oldText: "x = 1", newText: "x = 42" });
  assert.equal(response.ok, true);
  assert.equal(response.changed, 1);
  assert.deepEqual(response.edits, [{ path: "a.ts", changed: true, line: 1, removed: ["x = 1"], added: ["x = 42"] }]);
  assert.equal(await readFile(join(root, "a.ts"), "utf8"), "const x = 42;\nconst y = 2;\n");
});

test("edit fallback is phased: a bad entry writes nothing", async () => {
  const root = await project();
  await write(root, "a.ts", "alpha\n");
  await write(root, "b.ts", "beta\n");
  const old = oldLauncher();
  const bundle = createAsgrepConnector(old.host, { cwd: root });
  await assert.rejects(
    bundle.asgrep.edit({ edits: [{ path: "a.ts", oldText: "alpha", newText: "ALPHA" }, { path: "b.ts", oldText: "absent", newText: "BETA" }] }),
    /oldText must match exactly once \(found 0\)/,
  );
  assert.equal(await readFile(join(root, "a.ts"), "utf8"), "alpha\n");
  assert.equal(await readFile(join(root, "b.ts"), "utf8"), "beta\n");
  await assert.rejects(bundle.asgrep.edit({ path: "a.ts", oldText: "a", newText: "b" }), /oldText must match exactly once \(found 2\+\)/);
});

test("edit fallback composes same-file edits on the evolving buffer", async () => {
  const root = await project();
  await write(root, "a.ts", "one two\n");
  const old = oldLauncher();
  const bundle = createAsgrepConnector(old.host, { cwd: root });
  const response = await bundle.asgrep.edit({ edits: [{ path: "a.ts", oldText: "one", newText: "1" }, { path: "a.ts", oldText: "1 two", newText: "1 2" }] });
  assert.equal(response.changed, 2);
  assert.equal(await readFile(join(root, "a.ts"), "utf8"), "1 2\n");
});

test("find falls back to the direct CLI capsule mapping", async () => {
  const root = await project();
  const old = oldLauncher();
  const bundle = createAsgrepConnector(old.host, { cwd: root });
  const response = await bundle.asgrep.find({ query: "needle" });
  assert.equal(response.ok, true);
  assert.deepEqual(old.calls, ["find"]);
  assert.deepEqual(old.argv, [["--json", "--format", "agent-capsule", "--limit", "8", "--excerpt-lines", "0", "word:needle", "."]]);
});

test("batched unknown tools fall back per call", async () => {
  const root = await project();
  await write(root, "a.ts", "one\ntwo\n");
  const old = oldLauncher();
  const bundle = createAsgrepConnector(old.host, { cwd: root });
  const [first, second] = await Promise.all([
    bundle.asgrep.read({ path: "a.ts", start: 1, end: 1 }),
    bundle.asgrep.read({ path: "a.ts", start: 2, end: 2 }),
  ]);
  assert.equal((first.windows as Array<Record<string, unknown>>)[0]?.text, "one");
  assert.equal((second.windows as Array<Record<string, unknown>>)[0]?.text, "two");
});

test("fallback engages once per tool; later calls skip the native attempt", async () => {
  const root = await project();
  await write(root, "a.ts", "one\n");
  const old = oldLauncher();
  const bundle = createAsgrepConnector(old.host, { cwd: root });
  await bundle.asgrep.read({ path: "a.ts", start: 1, end: 1 });
  await bundle.asgrep.read({ path: "a.ts", start: 1, end: 1 });
  assert.deepEqual(old.calls, ["read"]);
});

test("non-unknown native errors are never swallowed by the fallback", async () => {
  const root = await project();
  await write(root, "a.ts", "one\n");
  const old = oldLauncher(() => new Error("session is busy"));
  const bundle = createAsgrepConnector(old.host, { cwd: root });
  await assert.rejects(bundle.asgrep.read({ path: "a.ts", start: 1, end: 1 }), /session is busy/);
  // No memoization on other errors: the next call still tries native first.
  await assert.rejects(bundle.asgrep.read({ path: "a.ts", start: 1, end: 1 }), /session is busy/);
  assert.deepEqual(old.calls, ["read", "read"]);
});

test("tools the launcher supports never touch the fallback", async () => {
  const root = await project();
  const old = oldLauncher();
  const bundle = createAsgrepConnector(old.host, { cwd: root });
  const response = await bundle.asgrep.search({ query: "needle" });
  assert.deepEqual(response.hits, []);
  assert.deepEqual(old.calls, ["search"]);
  assert.deepEqual(old.argv, []);
});
