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

const packageDir = process.cwd();
const id = basename(packageDir);
const targetsPath = fileURLToPath(new URL("../release/targets.json", import.meta.url));
const contractPath = fileURLToPath(new URL("../release-contract.json", import.meta.url));
const targets = JSON.parse(readFileSync(targetsPath, "utf8")).targets;
const version = JSON.parse(readFileSync(contractPath, "utf8")).canonicalVersion.version;
const target = targets.find(candidate => candidate.id === id);
const manifestPath = join(packageDir, "package.json");
if (!target) fail("ASGREP_PREPACK_UNKNOWN_TARGET", packageDir, "native package directory is not in the release target matrix");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const expectedLibc = target.libc === null ? [] : [target.libc];
if (manifest.name !== target.package || manifest.version !== version ||
    JSON.stringify(manifest.os) !== JSON.stringify([target.os]) ||
    JSON.stringify(manifest.cpu) !== JSON.stringify([target.cpu]) ||
    JSON.stringify(manifest.libc ?? []) !== JSON.stringify(expectedLibc)) {
  fail("ASGREP_PREPACK_METADATA_MISMATCH", manifestPath, "package metadata does not match the release contract");
}
const executablePath = join(packageDir, target.executable);
let stat;
try { stat = statSync(executablePath); } catch { fail("ASGREP_PREPACK_EXECUTABLE_MISSING", executablePath, "native executable is missing"); }
if (!stat.isFile() || stat.size === 0) fail("ASGREP_PREPACK_EXECUTABLE_EMPTY", executablePath, "native executable must contain staged artifact bytes");
if (target.os !== "win32" && (stat.mode & 0o111) === 0) fail("ASGREP_PREPACK_EXECUTABLE_MODE", executablePath, "native executable must have an execute mode");
const checksumPath = join(packageDir, "checksum.sha256");
let checksum;
try { checksum = readFileSync(checksumPath, "utf8").trim().split(/\s+/u); } catch { fail("ASGREP_PREPACK_CHECKSUM_MISSING", checksumPath, "checksum file is missing"); }
if (!/^[a-f0-9]{64}$/u.test(checksum[0] ?? "") || checksum[1] !== target.executable) fail("ASGREP_PREPACK_CHECKSUM_INVALID", checksumPath, "checksum must name the target executable and contain one SHA-256 digest");
const actual = createHash("sha256").update(readFileSync(executablePath)).digest("hex");
if (actual !== checksum[0]) fail("ASGREP_PREPACK_CHECKSUM_MISMATCH", executablePath, "staged executable does not match checksum.sha256");
