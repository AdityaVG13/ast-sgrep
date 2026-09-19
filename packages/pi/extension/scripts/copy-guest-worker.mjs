#!/usr/bin/env node
/**
 * Ship the codemode guest worker (plain .mjs, not compiled by tsc) into dist/.
 * runner.ts resolves it via new URL("./guest-worker.mjs", import.meta.url), so
 * a missing copy silently breaks every codemode run in the packed package.
 */
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const extensionDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const dstDir = join(extensionDir, "dist", "codemode");
mkdirSync(dstDir, { recursive: true });
const dst = join(dstDir, "guest-worker.mjs");
copyFileSync(join(extensionDir, "src", "codemode", "guest-worker.mjs"), dst);
console.error("copied guest-worker.mjs -> dist/codemode/");
