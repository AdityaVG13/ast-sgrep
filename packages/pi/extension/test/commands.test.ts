import assert from "node:assert/strict";
import test from "node:test";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerAstSgrepCommands } from "../src/index.js";
import { RuntimeError, type MachineEnvelope } from "../src/runtime.js";

type Command = {
  description: string;
  handler(args: string, ctx: { cwd: string; hasUI: boolean; ui: { notify(message: string, type?: string): void } }): Promise<void>;
};

function fixture(run: (args: readonly string[], context: { cwd: string }) => Promise<MachineEnvelope>) {
  const commands = new Map<string, Command>();
  const pi = { registerCommand(name: string, command: Command) { commands.set(name, command); } } as unknown as ExtensionAPI;
  registerAstSgrepCommands(pi, { run, async resolveRoot(context) { return context.cwd; } });
  return commands;
}

async function invoke(command: Command, args = "", hasUI = false) {
  const notifications: Array<{ message: string; type?: string }> = [];
  await command.handler(args, {
    cwd: "/fixture",
    hasUI,
    ui: { notify(message, type) { notifications.push({ message, type }); } },
  });
  return notifications;
}

test("registers exact official slash command names and descriptions", () => {
  const commands = fixture(async () => ({ tool: "asgrep", schema_version: "1.0.0", ok: true }));
  assert.deepEqual([...commands.keys()], ["asgrep-doctor", "asgrep-status", "asgrep-index", "asgrep-reindex"]);
  for (const command of commands.values()) assert.ok(command.description.length > 20);
});

test("maps commands to safe argv arrays without a shell", async () => {
  const calls: Array<{ args: readonly string[]; cwd: string }> = [];
  const commands = fixture(async (args, context) => {
    calls.push({ args, cwd: context.cwd });
    return { tool: "asgrep", schema_version: "1.0.0", ok: true, command: args[0] };
  });
  for (const command of commands.values()) await invoke(command);
  assert.deepEqual(calls, [
    { args: ["doctor", ".", "--json"], cwd: "/fixture" },
    { args: ["status", ".", "--json"], cwd: "/fixture" },
    { args: ["index", ".", "--json"], cwd: "/fixture" },
    { args: ["reindex", ".", "--json"], cwd: "/fixture" },
  ]);
});

test("headless doctor emits the complete machine envelope as JSON", async () => {
  const response: MachineEnvelope = {
    tool: "asgrep", schema_version: "1.0.0", ok: true, command: "doctor", status: "healthy",
    version: "1.1.0-alpha.1", root: "/fixture", binary: { available: true, path: "/fixture/asgrep" },
    index: { exists: true, compatible: true }, capabilities: ["exact", "graph", "semantic"],
  };
  const command = fixture(async () => response).get("asgrep-doctor")!;
  const [notification] = await invoke(command);
  assert.equal(notification?.type, "info");
  assert.deepEqual(JSON.parse(notification!.message), { ok: true, command: "asgrep-doctor", response });
});

test("interactive status renders a compact summary rather than machine JSON", async () => {
  const command = fixture(async () => ({
    tool: "asgrep", schema_version: "1.0.0", ok: true, status: "ready", counts: { files: 12, symbols: 34 },
  })).get("asgrep-status")!;
  const [notification] = await invoke(command, "", true);
  assert.equal(notification?.message, "asgrep-status: ready · files=12 symbols=34");
});

test("runtime and argument failures remain structured in headless mode", async () => {
  const commands = fixture(async () => { throw new RuntimeError("BINARY_NOT_FOUND", "native binary is unavailable", { platform: "fixture" }); });
  const [runtimeFailure] = await invoke(commands.get("asgrep-doctor")!);
  assert.deepEqual(JSON.parse(runtimeFailure!.message), {
    ok: false, command: "asgrep-doctor",
    error: { code: "BINARY_NOT_FOUND", message: "native binary is unavailable", details: { platform: "fixture" } },
  });
  assert.equal(runtimeFailure?.type, "error");

  let called = false;
  const invalid = fixture(async () => { called = true; return { tool: "asgrep", schema_version: "1.0.0", ok: true }; });
  const [argumentFailure] = await invoke(invalid.get("asgrep-index")!, "unexpected");
  assert.equal(called, false);
  assert.equal(JSON.parse(argumentFailure!.message).error.code, "INVALID_ARGUMENTS");
});
