#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { resolveBinary } from "../src/index.js";

try {
  const result = spawnSync(resolveBinary(), process.argv.slice(2), { stdio: "inherit", shell: false });
  if (result.error) throw result.error;
  if (result.signal) process.kill(process.pid, result.signal);
  process.exitCode = result.status ?? 1;
} catch (error) {
  const prefix = error?.code ? error.code + ": " : "";
  console.error(prefix + (error?.message ?? error));
  process.exitCode = 1;
}
