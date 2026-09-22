import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, realpath, rm, symlink, writeFile } from "node:fs/promises";
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

test("local reads preserve leading blank lines and report budget truncation", async () => {
  const root = await project();
  await write(root, "blank.ts", "\n\nconst visible = 1;\n\n");
  const old = oldLauncher();
  const bundle = createAsgrepConnector(old.host, { cwd: root }, { localReads: true });
  const response = await bundle.asgrep.read({ path: "blank.ts", start: 1, end: 4 });
  assert.deepEqual(response.windows, [{ path: "blank.ts", ref: "blank.ts#L1-L4", start: 1, end: 4, truncated: false, text: "\n\nconst visible = 1;\n" }]);
  const bounded = await bundle.asgrep.read({ path: "blank.ts", start: 3, end: 3, maxChars: 1 });
  assert.equal((bounded.windows as Array<{ truncated: boolean }>)[0]?.truncated, true);
  const beyond = await bundle.asgrep.read({ path: "blank.ts", start: 20, end: 21 });
  assert.equal((beyond.windows as Array<{ truncated: boolean }>)[0]?.truncated, false);
  assert.deepEqual(old.calls, [], "disk reads must not require a backend");
});

test("local reads normalize CRLF without discarding a lone final carriage return", async () => {
  const root = await project();
  await write(root, "crlf.ts", "one\r\nlast\r");
  const bundle = createAsgrepConnector(oldLauncher().host, { cwd: root }, { localReads: true });
  const response = await bundle.asgrep.read({ path: "crlf.ts", start: 1, end: 2 });
  assert.equal((response.windows as Array<{ text: string }>)[0]?.text, "one\nlast\r");
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

test("local reads clamp zero ranges consistently for paths and object refs", async () => {
  const root = await project();
  await write(root, "a.ts", "one\ntwo\n");
  const bundle = createAsgrepConnector(oldLauncher().host, { cwd: root }, { localReads: true });
  for (const input of [{ path: "a.ts", start: 0, end: 0 }, { refs: [{ path: "a.ts", start: 0, end: 0 }] }]) {
    const response = await bundle.asgrep.read(input as never);
    assert.deepEqual(response.windows, [{ path: "a.ts", ref: "a.ts#L1-L1", start: 1, end: 1, text: "one", truncated: false }]);
  }
});

test("POSIX read refs preserve literal backslashes instead of aliasing a directory", { skip: process.platform === "win32" }, async () => {
  const root = await project();
  await mkdir(join(root, "literal"));
  await write(root, "literal/name.ts", "wrong file\n");
  const bundle = createAsgrepConnector(oldLauncher().host, { cwd: root }, { localReads: true });
  for (const path of ["literal\\name.ts", "literal\\..\\name.ts", "..\\literal.ts"]) {
    await write(root, path, "intended file\n");
    const response = await bundle.asgrep.read({ path, start: 1, end: 1 });
    const window = (response.windows as Array<{ path: string; ref: string; text: string }>)[0]!;
    assert.equal(window.path, path);
    assert.equal(window.ref, `${path}#L1-L1`);
    assert.equal(window.text, "intended file");
    assert.deepEqual((await bundle.asgrep.read({ ref: window.ref })).windows, response.windows);
  }
});

test("filesystem fallback rejects invalid UTF-8 before reads or edits and preserves valid BOMs", async () => {
  const root = await project();
  const path = join(root, "bytes.ts");
  const original = Buffer.from([0xff, 0x6f, 0x6c, 0x64]);
  await writeFile(path, original);
  const bundle = createAsgrepConnector(oldLauncher().host, { cwd: root }, { localReads: true });
  await assert.rejects(bundle.asgrep.read({ path: "bytes.ts", start: 1, end: 1 }), /cannot read|UTF-8/);
  await assert.rejects(bundle.asgrep.edit({ path: "bytes.ts", oldText: "old", newText: "new" }), /cannot read|UTF-8/);
  assert.deepEqual(await readFile(path), original, "invalid source must remain byte-identical");
  await writeFile(path, "\uFEFFconst intact = 1;\n");
  const valid = await bundle.asgrep.read({ path: "bytes.ts", start: 1, end: 1 });
  assert.equal((valid.windows as Array<{ text: string }>)[0]?.text, "\uFEFFconst intact = 1;");
});

test("fallback edits across connectors serialize and reject surrogate halves without corrupting files", async () => {
  const root = await project();
  await write(root, "a.ts", "alpha beta 😀");
  const first = createAsgrepConnector(oldLauncher().host, { cwd: root });
  const second = createAsgrepConnector(oldLauncher().host, { cwd: root });
  await Promise.all([
    first.asgrep.edit({ path: "a.ts", oldText: "alpha", newText: "ALPHA" }),
    second.asgrep.edit({ path: "a.ts", oldText: "beta", newText: "BETA" }),
  ]);
  assert.equal(await readFile(join(root, "a.ts"), "utf8"), "ALPHA BETA 😀");
  for (const replacement of [{ oldText: "\ud83d", newText: "x" }, { oldText: "ALPHA", newText: "\ud800" }]) {
    await assert.rejects(first.asgrep.edit({ path: "a.ts", ...replacement }), /Unicode|UTF/);
  }
  assert.equal(await readFile(join(root, "a.ts"), "utf8"), "ALPHA BETA 😀");
  await first.asgrep.edit({ path: "a.ts", oldText: "😀", newText: "🙂" });
  assert.equal(await readFile(join(root, "a.ts"), "utf8"), "ALPHA BETA 🙂");
});

test("anchored read refs and edit paths round-trip without duplicating the scope", async () => {
  const root = await project();
  await mkdir(join(root, "src"));
  await write(root, "src/a.ts", "intended");
  const bundle = createAsgrepConnector(oldLauncher().host, { cwd: root }, { scope: "src", localReads: true });
  const response = await bundle.asgrep.read({ path: "a.ts" });
  const window = (response.windows as Array<{ path: string; ref: string }>)[0]!;
  assert.deepEqual((await bundle.asgrep.read({ ref: window.ref })).windows, response.windows);
  assert.deepEqual((await bundle.asgrep.read({ refs: [{ path: "a.ts" }] })).windows, response.windows);
  assert.deepEqual((await bundle.asgrep.read({ refs: [{ ref: window.ref }] })).windows, response.windows);
  await bundle.asgrep.edit({ path: window.path, oldText: "intended", newText: "updated" });
  assert.equal(await readFile(join(root, "src/a.ts"), "utf8"), "updated");
});

// APFS and Windows reject these filename bytes before the read boundary.
test("symlinks to non-UTF8 filenames cannot alias a replacement-character filename", { skip: process.platform !== "linux" }, async () => {
  const root = await project();
  const target = Buffer.concat([Buffer.from(root + "/invalid-"), Buffer.from([255]), Buffer.from(".ts")]);
  await writeFile(target, "intended");
  await write(root, "invalid-�.ts", "wrong file");
  await symlink(target, join(root, "alias.ts"));
  const bundle = createAsgrepConnector(oldLauncher().host, { cwd: root }, { localReads: true });
  await assert.rejects(bundle.asgrep.read({ path: "alias.ts" }), /UTF|resolve/);
  await assert.rejects(bundle.asgrep.edit({ path: "alias.ts", oldText: "wrong", newText: "corrupted" }), /UTF|resolve/);
  assert.equal(await readFile(target, "utf8"), "intended");
  assert.equal(await readFile(join(root, "invalid-�.ts"), "utf8"), "wrong file");
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
