import assert from "node:assert/strict";
import test from "node:test";
import { createAsgrepConnector } from "../src/codemode/connector.js";
import { normalizeCode, runCodemode } from "../src/codemode/sandbox.js";
import type { MachineEnvelope } from "../src/runtime.js";

test("normalizeCode wraps bare bodies and strips fences", () => {
  assert.match(normalizeCode("return 1"), /async \(\) =>/);
  assert.match(normalizeCode("```js\nreturn 2\n```"), /return 2/);
  assert.match(normalizeCode("async () => 3"), /^\(async \(\) => 3\)\(\)$/);
});

test("sandbox runs parallel asgrep calls and returns shaped result", async () => {
  const calls: string[][] = [];
  const host = {
    async run(args: readonly string[]): Promise<MachineEnvelope> {
      calls.push([...args]);
      if (args[0] === "status") {
        return { tool: "asgrep", schema_version: "1.0.0", ok: true, status: "ready" };
      }
      return {
        tool: "asgrep",
        schema_version: "1.0.0",
        ok: true,
        hits: [{ file: "src/a.ts", symbol: "auth_refresh", kind: "embed", score: 1 }],
      };
    },
  };
  const asgrep = createAsgrepConnector(host, { cwd: "/project" });
  const outcome = await runCodemode(
    `
      const [seed, status] = await Promise.all([
        asgrep.search({ query: "auth", limit: 3 }),
        asgrep.indexStatus(),
      ]);
      return {
        symbol: seed.hits[0].symbol,
        status: status.status,
        hitCount: seed.hits.length,
      };
    `,
    asgrep,
  );
  assert.equal(outcome.ok, true, outcome.error);
  assert.deepEqual(outcome.result, { symbol: "auth_refresh", status: "ready", hitCount: 1 });
  assert.equal(calls.length, 2);
  assert.ok(calls.some((args) => args.includes("agent-capsule")));
  assert.ok(calls.some((args) => args[0] === "status"));
});

test("sandbox blocks require and process", async () => {
  const asgrep = createAsgrepConnector({
    async run(): Promise<MachineEnvelope> {
      return { tool: "asgrep", schema_version: "1.0.0", ok: true };
    },
  }, { cwd: "/project" });
  const requireAttempt = await runCodemode(`return typeof require`, asgrep);
  assert.equal(requireAttempt.ok, true);
  assert.equal(requireAttempt.result, "undefined");
  const processAttempt = await runCodemode(`return typeof process`, asgrep);
  assert.equal(processAttempt.ok, true);
  assert.equal(processAttempt.result, "undefined");
});

test("connector maps defs/callers/chain/semantic argv", async () => {
  const calls: string[][] = [];
  const asgrep = createAsgrepConnector({
    async run(args: readonly string[]): Promise<MachineEnvelope> {
      calls.push([...args]);
      return { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [] };
    },
  }, { cwd: "/project" });
  await asgrep.defs({ symbol: "Foo", limit: 4 });
  await asgrep.callers({ symbol: "Foo", limit: 4 });
  await asgrep.chain({ query: "Foo", limit: 4 });
  await asgrep.semantic({ query: "credential renewal", limit: 4 });
  assert.deepEqual(calls[0], ["--json", "--format", "agent-capsule", "--limit", "4", "--excerpt-lines", "0", "defs: Foo", "."]);
  assert.deepEqual(calls[1], ["--json", "--format", "agent-capsule", "--limit", "4", "--excerpt-lines", "0", "callers: Foo", "."]);
  assert.deepEqual(calls[2], ["chain", "Foo", ".", "--json", "--format", "agent-capsule", "--limit", "4", "--excerpt-lines", "0"]);
  assert.deepEqual(calls[3], ["semantic", "credential renewal", ".", "--json", "--format", "agent-capsule", "--limit", "4", "--excerpt-lines", "0"]);
});
