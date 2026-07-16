import assert from "node:assert/strict";
import { mkdtemp, mkdir, realpath, rm, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test } from "node:test";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Check } from "typebox/value";
import { registerAstSgrepTools } from "../src/index.js";
import { FreshnessCoordinator, RuntimeError, resolveConfig, resolveRuntimeRoot, type MachineEnvelope, type RunOptions, type RuntimeContext } from "../src/runtime.js";

const temporary: string[] = [];
afterEach(async () => {
  await Promise.all(temporary.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});

async function rootFixture(): Promise<{ project: string; outside: string }> {
  const base = await mkdtemp(join(tmpdir(), "pi-asgrep-security-"));
  temporary.push(base);
  const project = join(base, "project");
  const outside = join(base, "outside");
  await mkdir(project);
  await mkdir(outside);
  return { project, outside };
}

async function expectRuntimeCode(action: () => Promise<unknown>, code: string): Promise<void> {
  await assert.rejects(action, (error) => error instanceof RuntimeError && error.code === code);
}

test("canonical containment rejects traversal and symlink escape", async () => {
  const { project, outside } = await rootFixture();
  await symlink(outside, join(project, "escape"));
  assert.equal(await resolveRuntimeRoot(project), await realpath(project));
  await expectRuntimeCode(() => resolveRuntimeRoot(project, "../outside"), "ROOT_OUTSIDE_PROJECT");
  await expectRuntimeCode(() => resolveRuntimeRoot(project, "escape"), "ROOT_OUTSIDE_PROJECT");
});

test("malformed numeric configuration never falls through to defaults", () => {
  for (const sources of [
    { environment: { ASGREP_TIMEOUT_MS: "NaN" } },
    { environment: { ASGREP_MAX_OUTPUT_BYTES: "0" } },
    { environment: { ASGREP_REFRESH_INTERVAL_MS: "1.5" } },
    { explicitProjectConfig: { timeoutMs: Number.POSITIVE_INFINITY } },
    { projectSettings: { maxOutputBytes: "4096" as unknown as number } },
    { globalSettings: { refreshIntervalMs: -1 } },
  ]) assert.throws(() => resolveConfig(sources), { code: "INVALID_CONFIG" });
});

test("concurrent refresh failure rejects every waiter, clears in-flight state, and retries", async () => {
  const gate = Promise.withResolvers<void>();
  let indexCalls = 0;
  let fail = true;
  const runtime = {
    async resolveRoot(context: RuntimeContext) { return context.cwd; },
    async run(args: readonly string[], _context: RuntimeContext, _options: RunOptions = {}): Promise<MachineEnvelope> {
      if (args[0] === "status") return { tool: "asgrep", schema_version: "1.0.0", ok: true, index: { exists: false, compatible: true, status: "missing" } };
      indexCalls += 1;
      if (fail) {
        await gate.promise;
        throw new Error("concurrent index failure");
      }
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, index: { exists: true, compatible: true, status: "ready" } };
    },
  };
  const freshness = new FreshnessCoordinator();
  const first = freshness.ensureFresh(runtime, { cwd: "/root" });
  const second = freshness.ensureFresh(runtime, { cwd: "/root" });
  gate.resolve();
  const failures = await Promise.allSettled([first, second]);
  assert.deepEqual(failures.map(({ status }) => status), ["rejected", "rejected"]);
  for (const result of failures) if (result.status === "rejected") assert.match(String(result.reason), /concurrent index failure/u);
  assert.equal(indexCalls, 1, "same-root concurrent work must be coalesced");
  fail = false;
  await freshness.ensureFresh(runtime, { cwd: "/root" });
  assert.equal(indexCalls, 2, "failed in-flight work must be cleared for retry");
});

test("registered TypeBox boundaries reject malformed model inputs", () => {
  const tools: Array<{ name: string; parameters: Parameters<typeof Check>[0] }> = [];
  const pi = {
    registerTool(tool: { name: string; parameters: Parameters<typeof Check>[0] }) { tools.push(tool); },
    on() {},
  } as unknown as ExtensionAPI;
  const runtime = {
    async resolveRoot(context: RuntimeContext) { return context.cwd; },
    async run(): Promise<MachineEnvelope> { return { tool: "asgrep", schema_version: "1.0.0", ok: true }; },
  };
  registerAstSgrepTools(pi, runtime);
  const schema = (name: string) => tools.find((tool) => tool.name === name)!.parameters;
  assert.equal(Check(schema("asgrep_search"), { query: "symbol", limit: 8, excerptLines: 0 }), true);
  for (const malformed of [
    {}, { query: "" }, { query: 42 }, { query: "x".repeat(4097) }, { query: "x", limit: 0 },
    { query: "x", limit: 101 }, { query: "x", limit: 1.5 }, { query: "x", excerptLines: -1 },
    { query: "x", mode: "shell" }, { query: "x", unexpected: true },
  ]) assert.equal(Check(schema("asgrep_search"), malformed), false, JSON.stringify(malformed));
  for (const malformed of [{ force: "true" }, { force: false, unexpected: true }]) {
    assert.equal(Check(schema("asgrep_index"), malformed), false, JSON.stringify(malformed));
  }
  assert.equal(Check(schema("asgrep_status"), { unexpected: true }), false);
});
