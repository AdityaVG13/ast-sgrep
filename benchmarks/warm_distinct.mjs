/**
 * Warm distinct-query latency for ast-sgrep search.
 *
 *   node benchmarks/warm_distinct.mjs <asgrep-binary> [rounds] [--no-embed]
 *
 * Self-contained: samples ~240 distinct identifiers from `git ls-files crates`,
 * then drives them through `asgrep codemode-serve` over NDJSON -- the warm path
 * the Pi package uses (one process, warm Searcher, no per-call spawn). Prints
 * p10/p50/p90/p99/mean/max per round so an interleaved A/B (two binaries,
 * alternating rounds) can be compared without process-start noise.
 *
 * Warm-up calls are excluded, and the round is measured from request write to
 * response line, so the number is the search path plus protocol cost.
 */
import { spawn, spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const args = process.argv.slice(2);
const positional = args.filter((a) => !a.startsWith("--"));
const flags = args.filter((a) => a.startsWith("--"));
const binary = positional[0];
const rounds = Number(positional[1] ?? 1);
const root = process.env.ASGREP_BENCH_ROOT ?? process.cwd();
const jsonOutput = process.env.ASGREP_BENCH_JSON === "1";
if (!binary) {
  console.error("usage: node benchmarks/warm_distinct.mjs <asgrep-binary> [rounds] [--no-embed]");
  process.exit(2);
}

function battery() {
  const listed = process.env.ASGREP_BENCH_MANIFEST
    ? { status: 0, stdout: JSON.parse(readFileSync(process.env.ASGREP_BENCH_MANIFEST, "utf8")).map((file) => file.path).filter((path) => path.startsWith("crates/")).join("\n") }
    : spawnSync("git", ["ls-files", "crates"], { cwd: root, encoding: "utf8" });
  if (listed.status !== 0) throw new Error("git ls-files crates failed: " + listed.stderr);
  const names = [];
  const seen = new Set();
  for (const rel of listed.stdout.split("\n")) {
    if (!rel.endsWith(".rs") || names.length >= 240) continue;
    let text;
    try {
      text = readFileSync(join(root, rel), "utf8");
    } catch {
      continue;
    }
    for (const match of text.matchAll(/^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:fn|struct|enum|trait)\s+([A-Za-z_][A-Za-z0-9_]{6,})/gm)) {
      const name = match[1];
      if (seen.has(name) || name.startsWith("_")) continue;
      seen.add(name);
      names.push(name);
      if (names.length >= 240) break;
    }
  }
  return names;
}

function startServe() {
  const indexFlags = process.env.ASGREP_BENCH_INDEX_PATH ? ["--index-path", process.env.ASGREP_BENCH_INDEX_PATH] : [];
  const child = spawn(binary, ["--root", root, ...indexFlags, ...flags, "codemode-serve"], {
    cwd: root,
    env: { ...process.env, NO_COLOR: "1" },
    stdio: ["pipe", "pipe", "pipe"],
  });
  let buffer = "";
  const waiters = new Map();
  child.stdout.on("data", (chunk) => {
    buffer += String(chunk);
    let index;
    while ((index = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, index).trim();
      buffer = buffer.slice(index + 1);
      if (!line) continue;
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        continue;
      }
      const waiter = waiters.get(message.id);
      if (waiter) {
        waiters.delete(message.id);
        waiter.resolve(message);
      }
    }
  });
  let stderr = "";
  child.stderr.on("data", (chunk) => { stderr += String(chunk); });
  const fail = (error) => {
    for (const waiter of waiters.values()) waiter.reject(error);
    waiters.clear();
  };
  child.on("error", fail);
  child.on("close", (code) => fail(new Error(`worker closed (${code}): ${stderr}`)));
  let nextId = 0;
  const call = (tool, callArgs) =>
    new Promise((resolve, reject) => {
      const id = String(nextId++);
      const timer = setTimeout(() => { waiters.delete(id); child.kill(); reject(new Error("worker response timeout")); }, 30000);
      waiters.set(id, {
        resolve: (message) => { clearTimeout(timer); message.error || message.ok === false ? reject(new Error(JSON.stringify(message))) : resolve(message); },
        reject: (error) => { clearTimeout(timer); reject(error); },
      });
      child.stdin.write(JSON.stringify({ id, type: "call", tool, args: callArgs }) + "\n", (error) => {
        if (error) reject(error);
      });
    });
  return { child, call };
}

const terms = battery();
if (terms.length === 0) {
  console.error("no battery terms found under crates/");
  process.exit(2);
}
const percentile = (sorted, p) => sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1))];

const { child, call } = startServe();
for (const term of terms.slice(0, 10)) await call("search", { query: term, limit: 8 });
if (!jsonOutput) console.log("battery: " + terms.length + " distinct identifiers" + (flags.length ? " " + flags.join(" ") : ""));
for (let round = 1; round <= rounds; round += 1) {
  const samples = [];
  for (const term of terms) {
    const started = process.hrtime.bigint();
    await call("search", { query: term, limit: 8 });
    samples.push(Number(process.hrtime.bigint() - started) / 1e6);
  }
  const sorted = [...samples].sort((a, b) => a - b);
  const mean = samples.reduce((sum, value) => sum + value, 0) / samples.length;
  if (jsonOutput) {
    console.log(JSON.stringify({ round, terms, samples_ms: samples, p50_ms: percentile(sorted, 50), p95_ms: percentile(sorted, 95), p99_ms: percentile(sorted, 99), max_ms: sorted.at(-1), mean_ms: mean }));
    continue;
  }
  console.log(
    "round " + round,
    "n=" + samples.length,
    "p10=" + percentile(sorted, 10).toFixed(3),
    "p50=" + percentile(sorted, 50).toFixed(3),
    "p90=" + percentile(sorted, 90).toFixed(3),
    "p99=" + percentile(sorted, 99).toFixed(3),
    "mean=" + mean.toFixed(3),
    "max=" + sorted[sorted.length - 1].toFixed(2),
  );
}
child.kill("SIGTERM");

