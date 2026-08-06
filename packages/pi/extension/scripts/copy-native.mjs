#!/usr/bin/env node
/**
 * Copy the release cdylib into packages/pi/extension/native/ with a platform
 * triple name so the Pi loader can find it without napi-cli.
 */
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const extensionDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const workspaceRoot = join(extensionDir, "..", "..", "..");
const outDir = join(extensionDir, "native");
mkdirSync(outDir, { recursive: true });

const { platform, arch } = process;
let triple = null;
if (platform === "linux" && arch === "x64") triple = "linux-x64-gnu";
else if (platform === "linux" && arch === "arm64") triple = "linux-arm64-gnu";
else if (platform === "darwin" && arch === "arm64") triple = "darwin-arm64";
else if (platform === "darwin" && arch === "x64") triple = "darwin-x64";
else if (platform === "win32" && arch === "x64") triple = "win32-x64-msvc";

const candidates = [
  join(workspaceRoot, "target/release/libast_sgrep_codemode_napi.so"),
  join(workspaceRoot, "target/release/libast_sgrep_codemode_napi.dylib"),
  join(workspaceRoot, "target/release/ast_sgrep_codemode_napi.dll"),
];
const src = candidates.find((p) => existsSync(p));
if (!src) {
  console.error("ast-sgrep-codemode-napi cdylib not found; run cargo build -p ast-sgrep-codemode-napi --release");
  process.exit(1);
}
const destName = triple ? `ast-sgrep-codemode.${triple}.node` : "ast-sgrep-codemode.node";
const dest = join(outDir, destName);
copyFileSync(src, dest);
console.log(`copied ${src} -> ${dest}`);
