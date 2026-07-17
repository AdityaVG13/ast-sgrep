import { createHash } from "node:crypto";
import { accessSync, constants, readFileSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";

const VERSION = "1.1.0-alpha.1";
const HOSTS = new Map([
  ["darwin:arm64:", ["ast-sgrep-darwin-arm64", "asgrep", "darwin", "arm64", null]],
  ["darwin:x64:", ["ast-sgrep-darwin-x64", "asgrep", "darwin", "x64", null]],
  ["linux:arm64:glibc", ["ast-sgrep-linux-arm64-gnu", "asgrep", "linux", "arm64", "glibc"]],
  ["linux:x64:glibc", ["ast-sgrep-linux-x64-gnu", "asgrep", "linux", "x64", "glibc"]],
  ["win32:x64:", ["ast-sgrep-win32-x64-msvc", "asgrep.exe", "win32", "x64", null]]
]);
const nativeRequire = createRequire(import.meta.url);

export class AstSgrepBinaryError extends Error {
  constructor(code, message, path, cause) {
    super(message, cause === undefined ? undefined : { cause });
    this.name = "AstSgrepBinaryError";
    this.code = code;
    if (path !== undefined) this.path = path;
  }
}
function fail(code, message, path, cause) { throw new AstSgrepBinaryError(code, message, path, cause); }
function defaultLibc(platform) {
  if (platform !== "linux") return "";
  return process.report?.getReport?.().header?.glibcVersionRuntime ? "glibc" : "musl";
}
function validateExecutable(path, fs, checkAccess) {
  let stat;
  try { stat = fs.statSync(path); } catch (cause) { fail("ASGREP_EXECUTABLE_MISSING", "ast-sgrep executable is missing at " + path, path, cause); }
  if (!stat.isFile()) fail("ASGREP_EXECUTABLE_INVALID", "ast-sgrep executable is not a regular file: " + path, path);
  if (checkAccess && (stat.mode & 0o111) === 0) fail("ASGREP_EXECUTABLE_NOT_EXECUTABLE", "ast-sgrep executable lacks an execute mode: " + path, path);
  if (checkAccess) {
    try { fs.accessSync(path, constants.X_OK); } catch (cause) { fail("ASGREP_EXECUTABLE_NOT_EXECUTABLE", "ast-sgrep executable is not executable: " + path, path, cause); }
  }
  return path;
}
export function resolveBinary(options = {}) {
  const fs = options.fs ?? { accessSync, readFileSync, statSync };
  const env = options.env ?? process.env;
  const platform = options.platform ?? process.platform;
  const override = options.binaryPath ?? env.ASGREP_BIN ?? env.AST_SGREP_BINARY;
  if (override) return validateExecutable(resolve(override), fs, platform !== "win32");
  const arch = options.arch ?? process.arch;
  const libc = options.libc ?? defaultLibc(platform);
  const key = platform + ":" + arch + ":" + (platform === "linux" ? libc : "");
  const mapping = HOSTS.get(key);
  const host = platform + "/" + arch + (platform === "linux" ? "/" + libc : "");
  if (!mapping) fail("ASGREP_UNSUPPORTED_PLATFORM", "ast-sgrep has no native package for " + host);
  const [packageName, executableName, expectedOs, expectedCpu, expectedLibc] = mapping;
  const requireResolve = options.requireResolve ?? nativeRequire.resolve;
  let manifestPath;
  try { manifestPath = requireResolve(packageName + "/package.json"); } catch (cause) { fail("ASGREP_PLATFORM_PACKAGE_MISSING", "Optional native package " + packageName + "@" + VERSION + " is not installed for " + host, packageName, cause); }
  let manifest;
  try { manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")); } catch (cause) { fail("ASGREP_PLATFORM_METADATA_CORRUPT", "Cannot read native package metadata: " + manifestPath, manifestPath, cause); }
  const os = Array.isArray(manifest.os) ? manifest.os : [];
  const cpu = Array.isArray(manifest.cpu) ? manifest.cpu : [];
  const libcMetadata = Array.isArray(manifest.libc) ? manifest.libc : [];
  if (manifest.name !== packageName || !os.includes(expectedOs) || !cpu.includes(expectedCpu) || (expectedLibc !== null && !libcMetadata.includes(expectedLibc))) fail("ASGREP_PLATFORM_METADATA_CORRUPT", "Native package metadata does not match " + host + ": " + manifestPath, manifestPath);
  if (manifest.version !== VERSION) fail("ASGREP_PLATFORM_VERSION_MISMATCH", "Native package " + packageName + " version " + (manifest.version ?? "unknown") + " does not match launcher " + VERSION, manifestPath);
  const executablePath = join(dirname(manifestPath), executableName);
  validateExecutable(executablePath, fs, platform !== "win32");
  const checksumPath = join(dirname(manifestPath), "checksum.sha256");
  let expected;
  try { expected = fs.readFileSync(checksumPath, "utf8").trim().split(/\s+/u)[0]; } catch (cause) { fail("ASGREP_CHECKSUM_MISSING", "Native package checksum is missing: " + checksumPath, checksumPath, cause); }
  if (!/^[a-f0-9]{64}$/u.test(expected)) fail("ASGREP_CHECKSUM_CORRUPT", "Native package checksum is invalid: " + checksumPath, checksumPath);
  let actual;
  try { actual = createHash("sha256").update(fs.readFileSync(executablePath)).digest("hex"); } catch (cause) { fail("ASGREP_EXECUTABLE_MISSING", "Cannot read native executable: " + executablePath, executablePath, cause); }
  if (actual !== expected) fail("ASGREP_CHECKSUM_MISMATCH", "Native executable checksum mismatch at " + executablePath + "; reinstall " + packageName + "@" + VERSION, executablePath);
  return executablePath;
}
