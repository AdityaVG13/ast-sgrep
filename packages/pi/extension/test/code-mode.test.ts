import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { afterEach, describe, it } from "node:test";
import { createSgrepCodeMode, parseSgrepRef, type SgrepRef } from "../src/code-mode.js";
import { MACHINE_SCHEMA_VERSION, RuntimeError, type MachineEnvelope, type RunOptions, type RuntimeContext } from "../src/runtime.js";

const temporary: string[] = [];
afterEach(async () => {
  await Promise.all(temporary.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});

const hit = {
  kind: "def",
  signal: "structural",
  contributors: ["def", "embed"],
  score: 0.04,
  margin: 0.01,
  file: "src/auth.ts",
  lines: { start: 2, end: 4 },
  ref: "src/auth.ts#L2-L4",
  preview: "export function renew() {",
};

class FakeRuntime {
  readonly calls: Array<{ args: readonly string[]; context: RuntimeContext; options: RunOptions }> = [];

  constructor(readonly root: string, private readonly response: MachineEnvelope = {
    tool: "asgrep",
    schema_version: MACHINE_SCHEMA_VERSION,
    ok: true,
    hits: [hit],
  }) {}

  async resolveRoot(_context: RuntimeContext): Promise<string> {
    return this.root;
  }

  async run(args: readonly string[], context: RuntimeContext, options: RunOptions = {}): Promise<MachineEnvelope> {
    this.calls.push({ args, context, options });
    return this.response;
  }
}

async function project(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "asgrep-code-mode-"));
  temporary.push(root);
  await mkdir(join(root, "src"));
  await writeFile(join(root, "src/auth.ts"), [
    "const token = 1;",
    "export function renew() {",
    "  return token;",
    "}",
    "export const tail = true;",
  ].join("\n"));
  return root;
}

async function runtimeError(action: () => Promise<unknown>, code: string): Promise<void> {
  await assert.rejects(action, (error: unknown) => error instanceof RuntimeError && error.code === code);
}

describe("SgrepCodeMode", () => {
  it("executes a typed multi-search plan over CLI JSON", async () => {
    const root = await project();
    const runtime = new FakeRuntime(root);
    const mode = createSgrepCodeMode(runtime, { cwd: root });

    const result = await mode.execute(async (sgrep) => {
      assert.equal(Object.isFrozen(sgrep), true);
      assert.equal("rewrite" in sgrep, false);
      return await Promise.all([
        sgrep.keywordSearch("renew token", { limit: 7 }),
        sgrep.astSearch("function_declaration", { excerptLines: 3 }),
        sgrep.semanticSearch("credential rotation"),
      ]);
    });

    assert.equal(result.length, 3);
    assert.deepEqual(result[0]!.hits[0]!.contributors, ["def", "embed"]);
    assert.equal(result[0]!.hits[0]!.ref, "src/auth.ts#L2-L4");
    assert.equal("file" in result[0]!.hits[0]!, false);
    assert.equal("lines" in result[0]!.hits[0]!, false);
    assert.deepEqual(parseSgrepRef(result[0]!.hits[0]!.ref), { file: "src/auth.ts", start: 2, end: 4 });
    assert.deepEqual(runtime.calls[0]!.args, [
      "--json", "--format", "agent-capsule", "--limit", "7", "--excerpt-lines", "0", "keyword", "--", "renew token", ".",
    ]);
    assert.deepEqual(runtime.calls[1]!.args, [
      "--json", "--format", "agent-capsule", "--limit", "20", "--excerpt-lines", "3", "--", "pattern: function_declaration", ".",
    ]);
    assert.deepEqual(runtime.calls[2]!.args, [
      "--json", "--format", "agent-capsule", "--limit", "20", "--excerpt-lines", "0", "semantic", "--", "credential rotation", ".",
    ]);
    await mode.find("--help");
    assert.deepEqual(runtime.calls[3]!.args.slice(-4), ["keyword", "--", "--help", "."]);
  });

  it("reads bounded refs with optional adjacent context", async () => {
    const root = await project();
    const mode = createSgrepCodeMode(new FakeRuntime(root), { cwd: root });
    const [read] = await mode.codeRead(hit.ref as SgrepRef, { contextLines: 1, maxChars: 48 });
    assert.ok(read);
    assert.equal("file" in read, false);
    assert.equal("lines" in read, false);
    const loc = parseSgrepRef(read.ref);
    assert.equal(loc.file, "src/auth.ts");
    assert.equal(loc.start, 1);
    assert.ok(loc.end <= 5);
    assert.ok(read.content.length <= 48);
    assert.equal(read.truncated, true);
  });

  it("rejects malformed and escaping refs including symlinks", async () => {
    const root = await project();
    const outside = await mkdtemp(join(tmpdir(), "asgrep-code-mode-outside-"));
    temporary.push(outside);
    await writeFile(join(outside, "secret.ts"), "secret");
    await symlink(join(outside, "secret.ts"), join(root, "src/escape.ts"));
    await symlink(outside, join(root, "src/escape-dir"), "dir");
    const mode = createSgrepCodeMode(new FakeRuntime(root), { cwd: root });

    await runtimeError(() => mode.read("../secret.ts#L1-L1" as SgrepRef), "PATH_OUTSIDE_ROOT");
    await runtimeError(() => mode.read("src/escape.ts#L1-L1" as SgrepRef), "PATH_OUTSIDE_ROOT");
    await runtimeError(() => mode.read("src/escape-dir/secret.ts#L1-L1" as SgrepRef), "PATH_OUTSIDE_ROOT");
    await runtimeError(() => mode.read("not-a-ref" as SgrepRef), "INVALID_REF");
  });

  it("bounds aggregate output and rejects EOF, binary, unsafe, and cancelled reads", async () => {
    const root = await project();
    await mkdir(join(root, "..cache"));
    await writeFile(join(root, "..cache/valid.ts"), "valid");
    await writeFile(join(root, "src/binary.ts"), Buffer.from([0xff, 0xfe, 0x00]));
    await writeFile(join(root, "src/emoji.ts"), "😀x");
    await writeFile(join(root, "src/crlf.ts"), "\r\nalpha\r\n");
    await writeFile(join(root, "src/empty.ts"), "");
    await writeFile(join(root, "src/long.ts"), "x".repeat(70_000));
    const mode = createSgrepCodeMode(new FakeRuntime(root), { cwd: root });

    const aggregate = await mode.read([
      "src/auth.ts#L1-L2" as SgrepRef,
      "src/auth.ts#L3-L5" as SgrepRef,
    ], { maxChars: 10 });
    assert.ok(aggregate.reduce((total, item) => total + [...item.content].length, 0) <= 10);
    const tiny = await mode.read([
      "src/auth.ts#L1-L1" as SgrepRef,
      "src/auth.ts#L2-L2" as SgrepRef,
    ], { maxChars: 1 });
    assert.ok(tiny.reduce((total, item) => total + [...item.content].length, 0) <= 1);
    assert.equal((await mode.read("..cache/valid.ts#L1-L1" as SgrepRef))[0]!.content, "valid");
    assert.equal((await mode.read("src/emoji.ts#L1-L1" as SgrepRef, { maxChars: 1 }))[0]!.content, "😀");
    assert.equal((await mode.read("src/crlf.ts#L1-L2" as SgrepRef))[0]!.content, "\nalpha");
    await runtimeError(() => mode.read("src/crlf.ts#L3-L3" as SgrepRef), "RANGE_OUT_OF_BOUNDS");
    assert.equal((await mode.read("src/empty.ts#L1-L1" as SgrepRef))[0]!.content, "");
    const long = (await mode.read("src/long.ts#L1-L1" as SgrepRef, { maxChars: 17 }))[0]!;
    assert.equal(long.content, "x".repeat(17));
    assert.equal(long.truncated, true);
    await runtimeError(() => mode.read("src/auth.ts#L100-L101" as SgrepRef), "RANGE_OUT_OF_BOUNDS");
    await runtimeError(() => mode.read("src/auth.ts#L2-L999" as SgrepRef, { maxChars: 1 }), "RANGE_OUT_OF_BOUNDS");
    await runtimeError(() => mode.read("src/auth.ts#L9007199254740992-L9007199254740992" as SgrepRef), "INVALID_REF");
    await runtimeError(() => mode.read("src/binary.ts#L1-L1" as SgrepRef), "BINARY_FILE");
    const controller = new AbortController();
    controller.abort();
    await runtimeError(() => mode.read("src/auth.ts#L1-L1" as SgrepRef, { signal: controller.signal }), "CANCELLED");
    const inFlight = new AbortController();
    const pending = mode.read("src/auth.ts#L1-L1" as SgrepRef, { signal: inFlight.signal });
    queueMicrotask(() => inFlight.abort());
    await runtimeError(() => pending, "CANCELLED");
  });

  it("publishes a typed code-mode package subpath", async () => {
    const manifest = JSON.parse(await readFile(new URL("../package.json", import.meta.url), "utf8")) as {
      exports: Record<string, { types?: string; import?: string }>;
    };
    assert.deepEqual(manifest.exports["./code-mode"], {
      types: "./dist/code-mode.d.ts",
      import: "./dist/code-mode.js",
    });
  });

  it("rejects malformed CLI envelopes and invalid plans", async () => {
    const root = await project();
    const runtime = new FakeRuntime(root, {
      tool: "asgrep",
      schema_version: MACHINE_SCHEMA_VERSION,
      ok: true,
    });
    const mode = createSgrepCodeMode(runtime, { cwd: root });
    await runtimeError(() => mode.find("query"), "PROTOCOL_MISMATCH");
    const invalidHit = new FakeRuntime(root, {
      tool: "asgrep",
      schema_version: MACHINE_SCHEMA_VERSION,
      ok: true,
      hits: [{ ...hit, score: Number.NaN }],
    });
    await runtimeError(() => createSgrepCodeMode(invalidHit, { cwd: root }).find("query"), "PROTOCOL_MISMATCH");
    const invalidOptional = new FakeRuntime(root, {
      tool: "asgrep",
      schema_version: MACHINE_SCHEMA_VERSION,
      ok: true,
      query: 42,
      hit_count: 99,
      hits: [hit],
    });
    await runtimeError(() => createSgrepCodeMode(invalidOptional, { cwd: root }).find("query"), "PROTOCOL_MISMATCH");
    await runtimeError(() => mode.execute(null as never), "INVALID_PLAN");
  });

  it("parses hit location once from ref and drops wire file/lines dual", async () => {
    const root = await project();
    const inconsistent = new FakeRuntime(root, {
      tool: "asgrep",
      schema_version: MACHINE_SCHEMA_VERSION,
      ok: true,
      hits: [{
        ...hit,
        file: "src/other.ts",
        lines: { start: 9, end: 9 },
        ref: "src/auth.ts#L2-L4",
      }],
    });
    const trusted = await createSgrepCodeMode(inconsistent, { cwd: root }).find("query");
    assert.equal(trusted.hits[0]!.ref, "src/auth.ts#L2-L4");
    assert.equal("file" in trusted.hits[0]!, false);
    assert.equal("lines" in trusted.hits[0]!, false);

    const refOnly = new FakeRuntime(root, {
      tool: "asgrep",
      schema_version: MACHINE_SCHEMA_VERSION,
      ok: true,
      hits: [{
        kind: "def",
        signal: "structural",
        contributors: ["def"],
        score: 1,
        margin: 0,
        ref: "src/auth.ts#L1-L1",
        preview: "const token = 1;",
      }],
    });
    const fromRef = await createSgrepCodeMode(refOnly, { cwd: root }).find("query");
    assert.equal(fromRef.hits[0]!.ref, "src/auth.ts#L1-L1");

    const structuredOnly = new FakeRuntime(root, {
      tool: "asgrep",
      schema_version: MACHINE_SCHEMA_VERSION,
      ok: true,
      hits: [{
        kind: "def",
        signal: "structural",
        contributors: ["def"],
        score: 1,
        margin: 0,
        file: "src/auth.ts",
        lines: { start: 3, end: 4 },
        preview: "  return token;",
      }],
    });
    const fromLines = await createSgrepCodeMode(structuredOnly, { cwd: root }).find("query");
    assert.equal(fromLines.hits[0]!.ref, "src/auth.ts#L3-L4");
    assert.equal("file" in fromLines.hits[0]!, false);
  });
});
