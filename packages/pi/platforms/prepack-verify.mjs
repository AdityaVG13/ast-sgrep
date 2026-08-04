import { createHash } from "node:crypto";
import { readFileSync, statSync } from "node:fs";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";

function fail(code, path, message) {
  const error = new Error(code + ": " + message + ": " + path);
  error.code = code;
  error.path = path;
  throw error;
}

function parseSha256Sums(text) {
  const map = new Map();
  for (const line of text.replace(/\r\n/gu, "\n").split("\n")) {
    if (!line) continue;
    const match = line.match(/^([a-f0-9]{64})  (.+)$/u);
    if (!match) return null;
    if (map.has(match[2])) return null;
    map.set(match[2], match[1]);
  }
  return map;
}

const packageDir = process.cwd();
const id = basename(packageDir);
const targetsPath = fileURLToPath(new URL("../release/targets.json", import.meta.url));
const contractPath = fileURLToPath(new URL("../release-contract.json", import.meta.url));
const matrix = JSON.parse(readFileSync(targetsPath, "utf8"));
const targets = matrix.targets;
const version = JSON.parse(readFileSync(contractPath, "utf8")).canonicalVersion.version;
const target = targets.find(candidate => candidate.id === id);
const napiAddon = matrix.napiAddon;
const manifestPath = join(packageDir, "package.json");
if (!target) fail("ASGREP_PREPACK_UNKNOWN_TARGET", packageDir, "native package directory is not in the release target matrix");
if (typeof napiAddon !== "string" || !napiAddon) fail("ASGREP_PREPACK_MATRIX", targetsPath, "target matrix is missing napiAddon");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const expectedLibc = target.libc === null ? [] : [target.libc];
if (manifest.name !== target.package || manifest.version !== version ||
    JSON.stringify(manifest.os) !== JSON.stringify([target.os]) ||
    JSON.stringify(manifest.cpu) !== JSON.stringify([target.cpu]) ||
    JSON.stringify(manifest.libc ?? []) !== JSON.stringify(expectedLibc)) {
  fail("ASGREP_PREPACK_METADATA_MISMATCH", manifestPath, "package metadata does not match the release contract");
}
const expectedFiles = [target.executable, napiAddon, "checksum.sha256", "LICENSE"].sort();
if (JSON.stringify([...(manifest.files ?? [])].sort()) !== JSON.stringify(expectedFiles)) {
  fail("ASGREP_PREPACK_METADATA_MISMATCH", manifestPath, "package files inventory must list CLI, NAPI addon, checksum, and LICENSE");
}

for (const [label, fileName, requireExecute] of [
  ["executable", target.executable, target.os !== "win32"],
  ["napi addon", napiAddon, false]
]) {
  const filePath = join(packageDir, fileName);
  let fileStat;
  try { fileStat = statSync(filePath); } catch { fail("ASGREP_PREPACK_EXECUTABLE_MISSING", filePath, "native " + label + " is missing"); }
  if (!fileStat.isFile() || fileStat.size === 0) fail("ASGREP_PREPACK_EXECUTABLE_EMPTY", filePath, "native " + label + " must contain staged artifact bytes");
  if (requireExecute && (fileStat.mode & 0o111) === 0) fail("ASGREP_PREPACK_EXECUTABLE_MODE", filePath, "native executable must have an execute mode");
}

const checksumPath = join(packageDir, "checksum.sha256");
let checksumText;
try { checksumText = readFileSync(checksumPath, "utf8"); } catch { fail("ASGREP_PREPACK_CHECKSUM_MISSING", checksumPath, "checksum file is missing"); }
const digests = parseSha256Sums(checksumText);
if (!digests || digests.size !== 2 || !digests.has(target.executable) || !digests.has(napiAddon)) {
  fail("ASGREP_PREPACK_CHECKSUM_INVALID", checksumPath, "checksum must name the CLI and NAPI addon with one SHA-256 digest each");
}
for (const fileName of [target.executable, napiAddon]) {
  const actual = createHash("sha256").update(readFileSync(join(packageDir, fileName))).digest("hex");
  if (actual !== digests.get(fileName)) fail("ASGREP_PREPACK_CHECKSUM_MISMATCH", join(packageDir, fileName), "staged file does not match checksum.sha256");
}
