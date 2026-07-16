import assert from "node:assert/strict";
import { mkdtemp, mkdir, realpath, rm, symlink } from "node:fs/promises";
import { DatabaseSync } from "node:sqlite";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { afterEach, describe, it } from "node:test";
import { AstSgrepRuntime, CONFIG_SCHEMA_VERSION, DEFAULT_MAX_OUTPUT_BYTES, DEFAULT_REFRESH_INTERVAL_MS, DEFAULT_TIMEOUT_MS, FreshnessCoordinator, INDEX_FORMAT_VERSION, MACHINE_SCHEMA_VERSION, RUNTIME_VERSION, RuntimeError, migrateConfig, resolveConfig, resolveRuntimeRoot, rollbackConfig, type ExecOptions, type ExecResult, type MachineEnvelope, type PiExec, type RunOptions, type RuntimeContext } from "../src/runtime.js";

const temporary: string[] = [];
afterEach(async () => { await Promise.all(temporary.splice(0).map((path) => rm(path, { recursive: true, force: true }))); });
async function fixture(): Promise<{ project: string; outside: string }> {
  const base = await mkdtemp(join(tmpdir(), "pi-asgrep-")); temporary.push(base);
  const project = join(base, "project"); const outside = join(base, "outside");
  await mkdir(project); await mkdir(outside); return { project, outside };
}
const valid = (extra: Record<string, unknown> = {}): ExecResult => ({ stdout: JSON.stringify({ tool: "asgrep", schema_version: MACHINE_SCHEMA_VERSION, ok: true, ...extra }), stderr: "", exitCode: 0 });
class FakePi implements PiExec {
  calls: Array<{ command: string; args: readonly string[]; options: ExecOptions }> = [];
  constructor(private readonly result: ExecResult | ((options: ExecOptions, args: readonly string[]) => Promise<ExecResult>) = valid()) {}
  async exec(command: string, args: readonly string[], options: ExecOptions): Promise<ExecResult> {
    this.calls.push({ command, args, options });
    return typeof this.result === "function" ? this.result(options, args) : this.result;
  }
}
function runtime(pi: PiExec, project: string, config: Parameters<typeof resolveConfig>[0] = {}): AstSgrepRuntime {
  return new AstSgrepRuntime(pi, { ...config, explicitProjectConfig: { root: project, ...config.explicitProjectConfig } }, { resolveBinary: (() => process.execPath) as never });
}
async function errorCode(action: () => Promise<unknown>, code: string): Promise<RuntimeError> {
  try {
    await action();
  } catch (error) {
    assert.ok(error instanceof RuntimeError);
    assert.equal(error.code, code);
    return error;
  }
  assert.fail(`Expected ${code}`);
}
async function createIndex(path: string, version: number, marker: string): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  const database = new DatabaseSync(path);
  try {
    database.exec(`PRAGMA user_version = ${version}; CREATE TABLE marker (value TEXT NOT NULL);`);
    database.prepare("INSERT INTO marker (value) VALUES (?)").run(marker);
  } finally {
    database.close();
  }
}

function readMarker(path: string): string {
  const database = new DatabaseSync(path, { readOnly: true });
  try {
    const row: unknown = database.prepare("SELECT value FROM marker").get();
    if (!row || typeof row !== "object" || !("value" in row) || typeof row.value !== "string") assert.fail("marker row is invalid");
    return row.value;
  } finally {
    database.close();
  }
}


describe("configuration and resolver", () => {
  it("applies explicit > project > global > environment > defaults fieldwise", () => {
    const value = resolveConfig({ defaults: { binaryPath: "default", root: "default", timeoutMs: 1 }, environment: { ASGREP_BIN: "env", ASGREP_ROOT: "env-root", ASGREP_TIMEOUT_MS: "2" }, globalSettings: { binaryPath: "global", timeoutMs: 3 }, projectSettings: { binaryPath: "project" }, explicitProjectConfig: { binaryPath: "explicit" } });
    assert.equal(value.binaryPath, "explicit"); assert.equal(value.root, "env-root"); assert.equal(value.timeoutMs, 3); assert.equal(value.maxOutputBytes, DEFAULT_MAX_OUTPUT_BYTES);
  });
  it("uses defaults and rejects invalid numeric configuration", () => {
    const value = resolveConfig({ environment: {} }); assert.equal(value.timeoutMs, DEFAULT_TIMEOUT_MS); assert.equal(value.maxOutputBytes, DEFAULT_MAX_OUTPUT_BYTES); assert.equal(value.refreshIntervalMs, DEFAULT_REFRESH_INTERVAL_MS);
    assert.throws(() => resolveConfig({ environment: { ASGREP_TIMEOUT_MS: "NaN" } }), { code: "INVALID_CONFIG" });
    assert.equal(resolveConfig({ environment: { ASGREP_REFRESH_INTERVAL_MS: "17" } }).refreshIntervalMs, 17);
    assert.throws(() => resolveConfig({ environment: { ASGREP_REFRESH_INTERVAL_MS: "0" } }), { code: "INVALID_CONFIG" });
  });
  it("migrates schema 0 settings without mutation and supports lossless rollback", () => {
    const legacy = { schemaVersion: 0 as const, root: "src", timeout: 11, maxOutput: 22, refreshInterval: 33, env: { A: "1" } };
    const snapshot = structuredClone(legacy);
    const current = migrateConfig(legacy);
    assert.deepEqual(legacy, snapshot);
    assert.deepEqual(current, { schemaVersion: CONFIG_SCHEMA_VERSION, root: "src", timeoutMs: 11, maxOutputBytes: 22, refreshIntervalMs: 33, env: { A: "1" } });
    assert.deepEqual(rollbackConfig(current), legacy);
  });
  it("rejects ambiguous or future config while leaving rollback input untouched", () => {
    const ambiguous = { timeoutMs: 10, timeout: 20 };
    const snapshot = structuredClone(ambiguous);
    assert.throws(() => migrateConfig(ambiguous as never), { code: "CONFIG_MIGRATION_CONFLICT" });
    assert.deepEqual(ambiguous, snapshot);
    let futureError: RuntimeError | undefined;
    assert.throws(() => migrateConfig({ schemaVersion: 2 } as never), (error) => {
      assert.ok(error instanceof RuntimeError);
      futureError = error;
      return true;
    });
    assert.equal(futureError?.code, "CONFIG_VERSION_MISMATCH");
    assert.equal(futureError?.details.rollbackSafe, true);
  });
  it("passes binaryPath and environment to the synchronous resolver", async () => {
    const { project } = await fixture(); let seen: unknown; const pi = new FakePi();
    const subject = new AstSgrepRuntime(pi, { environment: { TOKEN: "env" }, explicitProjectConfig: { binaryPath: process.execPath, root: project } }, { resolveBinary: ((options: unknown) => { seen = options; return process.execPath; }) as never });
    await subject.run(["status", "--json"], { cwd: project });
    assert.equal((seen as { binaryPath: string }).binaryPath, process.execPath); assert.equal((seen as { env: NodeJS.ProcessEnv }).env.TOKEN, "env");
  });
  it("reports a configured missing binary path", async () => {
    const { project } = await fixture(); const missing = join(project, "missing-asgrep");
    const subject = new AstSgrepRuntime(new FakePi(), { environment: {}, explicitProjectConfig: { root: project, binaryPath: missing } }, { resolveBinary: (() => { throw new Error("not found"); }) as never });
    const error = await errorCode(() => subject.run([], { cwd: project }), "BINARY_NOT_FOUND"); assert.ok(error.message.includes(missing));
  });
});

describe("canonical roots", () => {
  it("defaults to the real Pi context cwd and accepts contained roots", async () => {
    const { project } = await fixture(); const child = join(project, "src"); await mkdir(child);
    assert.equal(await resolveRuntimeRoot(project), await realpath(project)); assert.equal(await resolveRuntimeRoot(project, "src"), await realpath(child));
  });
  it("rejects traversal and symlink escapes after realpath", async () => {
    const { project, outside } = await fixture(); await symlink(outside, join(project, "escape"));
    await errorCode(() => resolveRuntimeRoot(project, "../outside"), "ROOT_OUTSIDE_PROJECT");
    await errorCode(() => resolveRuntimeRoot(project, "escape"), "ROOT_OUTSIDE_PROJECT");
  });
  it("allows outside roots only from explicit project config", async () => {
    const { project, outside } = await fixture();
    assert.equal(await resolveRuntimeRoot(project, outside, true), await realpath(outside));
    assert.equal(resolveConfig({ environment: {}, globalSettings: { allowOutsideProject: true } }).allowOutsideProject, false);
    assert.equal(resolveConfig({ environment: {}, explicitProjectConfig: { allowOutsideProject: true } }).allowOutsideProject, true);
  });
});

describe("execution boundary", () => {
  it("preserves hostile arguments as argv and never constructs a shell command", async () => {
    const { project } = await fixture(); const pi = new FakePi(); const args = ["search", "$(touch pwned); ' \n --", project];
    await runtime(pi, project, { environment: {} }).run(args, { cwd: project });
    assert.equal(pi.calls[0]?.command, process.execPath); assert.deepEqual(pi.calls[0]?.args, args); assert.ok(Object.isFrozen(pi.calls[0]?.args));
  });
  it("merges env, forces NO_COLOR, and forwards cwd and timeout", async () => {
    const { project } = await fixture(); const pi = new FakePi();
    await runtime(pi, project, { environment: {}, explicitProjectConfig: { env: { A: "configured" }, timeoutMs: 77 } }).run([], { cwd: project }, { env: { A: "request", B: "yes" } });
    const options = pi.calls[0]!.options; assert.equal(options.cwd, await realpath(project)); assert.equal(options.timeout, 77); assert.equal(options.env.A, "request"); assert.equal(options.env.B, "yes"); assert.equal(options.env.NO_COLOR, "1");
  });
  it("bounds stdout and stderr before parsing", async () => {
    const { project } = await fixture(); const pi = new FakePi({ stdout: "x".repeat(11), stderr: "", exitCode: 0 });
    const subject = runtime(pi, project, { environment: {}, explicitProjectConfig: { maxOutputBytes: 10 } }); await errorCode(() => subject.run([], { cwd: project }), "OUTPUT_LIMIT");
  });
  it("distinguishes malformed output and nonzero exit", async () => {
    const { project } = await fixture();
    await errorCode(() => runtime(new FakePi({ stdout: "not-json", stderr: "", exitCode: 0 }), project, { environment: {} }).run([], { cwd: project }), "MALFORMED_OUTPUT");
    const error = await errorCode(() => runtime(new FakePi({ stdout: "", stderr: "concise failure", exitCode: 2 }), project, { environment: {} }).run([], { cwd: project }), "PROCESS_FAILED"); assert.equal(error.details.stderr, "concise failure");
  });
  it("maps missing execution and timeout failures", async () => {
    const { project } = await fixture();
    const missing = new FakePi(async () => { throw new Error("ENOENT"); }); await errorCode(() => runtime(missing, project, { environment: {} }).run([], { cwd: project }), "EXEC_FAILED");
    const timeout = new FakePi(async () => { throw new Error("process timed out"); }); await errorCode(() => runtime(timeout, project, { environment: {} }).run([], { cwd: project }), "TIMEOUT");
  });
  it("forwards the exact AbortSignal and delegates cancellation/process-tree cleanup to Pi exec", async () => {
    const { project } = await fixture(); const controller = new AbortController();
    const pi = new FakePi(async (options) => new Promise((_resolve, reject) => { assert.equal(options.signal, controller.signal); if (options.signal!.aborted) reject(new DOMException("aborted", "AbortError")); else options.signal!.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")), { once: true }); }));
    const pending = runtime(pi, project, { environment: {} }).run([], { cwd: project }, { signal: controller.signal }); controller.abort(); await errorCode(() => pending, "CANCELLED");
  });
});

describe("machine compatibility", () => {
  it("rejects tool, protocol, extension-version, and reported-protocol mismatches", async () => {
    const { project } = await fixture();
    await errorCode(() => runtime(new FakePi(valid({ tool: "other" })), project, { environment: {} }).run([], { cwd: project }), "TOOL_MISMATCH");
    await errorCode(() => runtime(new FakePi(valid({ schema_version: "2" })), project, { environment: {} }).run([], { cwd: project }), "PROTOCOL_MISMATCH");
    await errorCode(() => runtime(new FakePi(valid({ version: "0.0.0" })), project, { environment: {} }).run([], { cwd: project }), "VERSION_MISMATCH");
    await errorCode(() => runtime(new FakePi(valid({ machine_schema_version: "2" })), project, { environment: {} }).run([], { cwd: project }), "PROTOCOL_MISMATCH");
  });
  it("runs the version probe and requires version plus machine protocol", async () => {
    const { project } = await fixture(); const pi = new FakePi(valid({ version: RUNTIME_VERSION, machine_schema_version: MACHINE_SCHEMA_VERSION }));
    await runtime(pi, project, { environment: {} }).checkCompatibility({ cwd: project }); assert.deepEqual(pi.calls[0]?.args, ["version", "--json"]);
    await errorCode(() => runtime(new FakePi(valid()), project, { environment: {} }).checkCompatibility({ cwd: project }), "VERSION_MISMATCH");
  });
});
describe("index format upgrades", () => {
  it("atomically replaces an incompatible index only after a valid rebuild", async () => {
    const { project } = await fixture();
    const indexPath = join(project, ".asgrep", "index.db");
    await createIndex(indexPath, INDEX_FORMAT_VERSION - 1, "prior");
    const pi = new FakePi(async (_options, args) => {
      const option = args.indexOf("--index-path");
      assert.notEqual(option, -1);
      await createIndex(args[option + 1]!, INDEX_FORMAT_VERSION, "replacement");
      return valid({ command: "index", files_indexed: 1 });
    });
    const subject = runtime(pi, project, { environment: {} });
    assert.equal(await subject.inspectIndexCompatibility({ cwd: project }), "incompatible");
    await subject.rebuildIncompatibleIndex({ cwd: project });
    assert.equal(await subject.inspectIndexCompatibility({ cwd: project }), "ready");
    assert.equal(readMarker(indexPath), "replacement");
    assert.equal(pi.calls[0]?.args[0], "--index-path");
    assert.match(pi.calls[0]?.args[1] ?? "", /\.asgrep\/\.rebuild-[^/]+\/index\.db$/);
    assert.deepEqual(pi.calls[0]?.args.slice(2), ["index", ".", "--json"]);
  });

  it("preserves the recoverable prior index and returns a structured failure", async () => {
    const { project } = await fixture();
    const indexPath = join(project, ".asgrep", "index.db");
    await createIndex(indexPath, INDEX_FORMAT_VERSION - 1, "prior");
    const subject = runtime(new FakePi(async () => {
      throw new Error("simulated rebuild failure");
    }), project, { environment: {} });
    const error = await errorCode(() => subject.rebuildIncompatibleIndex({ cwd: project }), "INDEX_REBUILD_FAILED");
    assert.equal(error.details.priorIndexPreserved, true);
    assert.equal(error.details.recoveryPath, await realpath(indexPath));
    assert.equal(readMarker(indexPath), "prior");
  });
});



const machine = (extra: Record<string, unknown> = {}): MachineEnvelope => ({ tool: "asgrep", schema_version: MACHINE_SCHEMA_VERSION, ok: true, ...extra });
type FreshCall = { command: string; root: string; signal?: AbortSignal };
class FakeFreshnessRuntime {
  calls: FreshCall[] = [];
  aliases = new Map<string, string>();
  handler: (command: string, root: string, options: RunOptions) => Promise<MachineEnvelope> = async (command) =>
    machine({ command, root: "/root", index_path: "/root/.asgrep/index.db", file_count: 1 });

  async resolveRoot(context: RuntimeContext): Promise<string> { return this.aliases.get(context.cwd) ?? context.cwd; }
  async run(args: readonly string[], context: RuntimeContext, options: RunOptions = {}): Promise<MachineEnvelope> {
    const command = args[0]!;
    this.calls.push({ command, root: context.cwd, signal: options.signal });
    return this.handler(command, context.cwd, options);
  }
}

const commands = (runtime: FakeFreshnessRuntime) => runtime.calls.map(({ command }) => command);

describe("per-root index freshness", () => {
  it("lazily indexes a missing root and deduplicates immediate repeats", async () => {
    const runtime = new FakeFreshnessRuntime();
    runtime.handler = async (command) => machine({ command, root: "/root", index_path: "/root/.asgrep/index.db", file_count: command === "status" ? 0 : 1 });
    const subject = new FreshnessCoordinator({ refreshIntervalMs: 100, now: () => 0 });
    await subject.ensureFresh(runtime, { cwd: "/root" });
    await subject.ensureFresh(runtime, { cwd: "/root" });
    assert.deepEqual(commands(runtime), ["status", "index"]);
  });

  it("uses safe reindex only for an explicitly incompatible index", async () => {
    const runtime = new FakeFreshnessRuntime();
    runtime.handler = async (command) => {
      if (command === "status") throw new RuntimeError("OPERATIONAL_ERROR", "unsupported schema version");
      return machine({ command, root: "/root", index_path: "/root/.asgrep/index.db", file_count: 1 });
    };
    await new FreshnessCoordinator().ensureFresh(runtime, { cwd: "/root" });
    assert.deepEqual(commands(runtime), ["status", "reindex"]);
  });

  it("fully reconciles external create, modify, and delete after interval expiry", async () => {
    let now = 0;
    const runtime = new FakeFreshnessRuntime();
    const subject = new FreshnessCoordinator({ refreshIntervalMs: 10, now: () => now });
    await subject.ensureFresh(runtime, { cwd: "/root" });
    for (const _change of ["create", "modify", "delete"]) {
      now += 10;
      await subject.ensureFresh(runtime, { cwd: "/root" });
    }
    assert.deepEqual(commands(runtime), ["status", "index", "status", "index", "status", "index", "status", "index"]);
  });

  it("refreshes immediately after known write paths and retains pre-first-search dirtying", async () => {
    const runtime = new FakeFreshnessRuntime();
    const subject = new FreshnessCoordinator({ refreshIntervalMs: 1_000, now: () => 0 });
    subject.markAffectedPath("src/created.ts", "/root");
    await subject.ensureFresh(runtime, { cwd: "/root" });
    subject.markAffectedPath("/root/src/modified.ts", "/elsewhere");
    await subject.ensureFresh(runtime, { cwd: "/root" });
    assert.deepEqual(commands(runtime), ["status", "index", "status", "index"]);
  });

  it("coalesces canonical aliases while distinct roots refresh concurrently", async () => {
    const runtime = new FakeFreshnessRuntime();
    runtime.aliases.set("/alias-a", "/root"); runtime.aliases.set("/alias-b", "/root");
    const releases = new Map<string, () => void>();
    runtime.handler = async (command, root) => {
      if (command === "index") await new Promise<void>((resolve) => releases.set(root, resolve));
      return machine({ command, index: { exists: command !== "status", compatible: true, status: command === "status" ? "missing" : "ready" } });
    };
    const subject = new FreshnessCoordinator();
    const sameA = subject.ensureFresh(runtime, { cwd: "/alias-a" });
    const sameB = subject.ensureFresh(runtime, { cwd: "/alias-b" });
    const other = subject.ensureFresh(runtime, { cwd: "/other" });
    while (!releases.has("/root") || !releases.has("/other")) await new Promise((resolve) => setImmediate(resolve));
    assert.deepEqual(runtime.calls.filter(({ command }) => command === "index").map(({ root }) => root).sort(), ["/other", "/root"]);
    releases.get("/root")!(); releases.get("/other")!();
    await Promise.all([sameA, sameB, other]);
    assert.equal(runtime.calls.filter(({ root, command }) => root === "/root" && command === "index").length, 1);
  });

  it("clears failed and cancelled in-flight work, keeps dirty, and retries", async () => {
    const runtime = new FakeFreshnessRuntime();
    let failures = 2;
    runtime.handler = async (command) => {
      if (command === "status") return machine({ command, index: { exists: false, compatible: true, status: "missing" } });
      if (failures-- > 0) throw failures === 1 ? new Error("index failed") : new RuntimeError("CANCELLED", "cancelled");
      return machine({ command, index: { exists: true, compatible: true, status: "ready" } });
    };
    const subject = new FreshnessCoordinator();
    await assert.rejects(subject.ensureFresh(runtime, { cwd: "/root" }), /index failed/);
    await assert.rejects(subject.ensureFresh(runtime, { cwd: "/root" }), { code: "CANCELLED" });
    await subject.ensureFresh(runtime, { cwd: "/root" });
    assert.deepEqual(commands(runtime), ["status", "index", "status", "index", "status", "index"]);
  });


  it("indexes on first use even when status reports ready, then deduplicates", async () => {
    const runtime = new FakeFreshnessRuntime();
    const subject = new FreshnessCoordinator({ refreshIntervalMs: 1_000, now: () => 0 });
    await subject.ensureFresh(runtime, { cwd: "/root" });
    await subject.ensureFresh(runtime, { cwd: "/root" });
    assert.deepEqual(commands(runtime), ["status", "index"]);
  });

  it("preserves dirtiness recorded while a refresh is in flight", async () => {
    const runtime = new FakeFreshnessRuntime();
    let release!: () => void;
    let indexCalls = 0;
    runtime.handler = async (command) => {
      if (command === "index" && indexCalls++ === 0) await new Promise<void>((resolve) => { release = resolve; });
      return machine({ command, index: { exists: true, compatible: true, status: "ready" } });
    };
    const subject = new FreshnessCoordinator({ refreshIntervalMs: 1_000, now: () => 0 });
    const first = subject.ensureFresh(runtime, { cwd: "/root" });
    while (!release) await new Promise((resolve) => setImmediate(resolve));
    subject.markAffectedPath("src/changed.ts", "/root");
    release();
    await first;
    await subject.ensureFresh(runtime, { cwd: "/root" });
    assert.deepEqual(commands(runtime), ["status", "index", "status", "index"]);
  });

  it("canonicalizes symlinked cwd and non-existent affected paths", async () => {
    const { project } = await fixture();
    const alias = join(project, "..", "project-alias");
    await symlink(project, alias);
    const canonical = await realpath(project);
    const runtime = new FakeFreshnessRuntime();
    runtime.aliases.set(project, canonical);
    const subject = new FreshnessCoordinator({ refreshIntervalMs: 1_000, now: () => 0 });
    await subject.ensureFresh(runtime, { cwd: project });
    subject.markAffectedPath(join(alias, "not-created", "file.ts"), alias);
    await subject.ensureFresh(runtime, { cwd: project });
    assert.deepEqual(commands(runtime), ["status", "index", "status", "index"]);
  });
  it("refuses to silently query when status cannot prove index health", async () => {
    const runtime = new FakeFreshnessRuntime();
    runtime.handler = async (command) => machine({ command });
    const error = await errorCode(() => new FreshnessCoordinator().ensureFresh(runtime, { cwd: "/root" }), "INDEX_STATUS_UNKNOWN");
    assert.match(error.message, /freshness/);
    assert.deepEqual(commands(runtime), ["status"]);
  });
});

describe("classified runtime failures", () => {
  it("normalizes default resolver failures", async () => {
    const { project } = await fixture();
    const subject = new AstSgrepRuntime(new FakePi(), { environment: {}, explicitProjectConfig: { root: project } }, { resolveBinary: (() => { throw new Error("unsupported platform"); }) as never });
    const error = await errorCode(() => subject.run([], { cwd: project }), "BINARY_RESOLUTION_FAILED");
    assert.equal(error.details.cause, "unsupported platform");
  });
  it("classifies ok false machine envelopes as operational errors, including nonzero CLI exits", async () => {
    const { project } = await fixture();
    const response = valid({ ok: false, command: "status", error: { kind: "operational", message: "index unavailable" } });
    const error = await errorCode(() => runtime(new FakePi(response), project, { environment: {} }).run([], { cwd: project }), "OPERATIONAL_ERROR");
    assert.equal(error.message, "index unavailable");
    assert.equal(error.details.command, "status");
    const nonzero = { ...response, exitCode: 1 };
    const nonzeroError = await errorCode(() => runtime(new FakePi(nonzero), project, { environment: {} }).run([], { cwd: project }), "OPERATIONAL_ERROR");
    assert.equal(nonzeroError.message, "index unavailable");
  });
});
