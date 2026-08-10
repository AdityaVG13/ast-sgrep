import { createHash } from "node:crypto";
import { accessSync, constants, existsSync, readFileSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";

const VERSION = "1.4.0";
const NAPI_ADDON = "ast-sgrep-codemode.node";
const HOSTS = new Map([
  ["darwin:arm64:", ["@ast-sgrep/darwin-arm64", "asgrep", "darwin", "arm64", null]],
  ["darwin:x64:", ["@ast-sgrep/darwin-x64", "asgrep", "darwin", "x64", null]],
  ["linux:arm64:glibc", ["@ast-sgrep/linux-arm64-gnu", "asgrep", "linux", "arm64", "glibc"]],
  ["linux:x64:glibc", ["@ast-sgrep/linux-x64-gnu", "asgrep", "linux", "x64", "glibc"]],
  ["win32:x64:", ["@ast-sgrep/win32-x64-msvc", "asgrep.exe", "win32", "x64", null]]
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
function digestFor(checksumText, filename, checksumPath) {
  const digests = parseSha256Sums(checksumText);
  if (!digests) fail("ASGREP_CHECKSUM_CORRUPT", "Native package checksum is invalid: " + checksumPath, checksumPath);
  const expected = digests.get(filename);
  if (!expected) fail("ASGREP_CHECKSUM_MISSING", "Native package checksum is missing entry for " + filename + ": " + checksumPath, checksumPath);
  if (!/^[a-f0-9]{64}$/u.test(expected)) fail("ASGREP_CHECKSUM_CORRUPT", "Native package checksum is invalid for " + filename + ": " + checksumPath, checksumPath);
  return expected;
}
function validateExecutable(path, fs, checkAccess) {
  let stat;
  try { stat = fs.statSync(path); } catch (cause) { fail("ASGREP_EXECUTABLE_MISSING", "ast-sgrep executable is missing at " + path, path, cause); }
  if (!stat.isFile()) fail("ASGREP_EXECUTABLE_INVALID", "ast-sgrep executable is not a regular file: " + path, path);
  if (stat.size === 0) fail("ASGREP_EXECUTABLE_EMPTY", "ast-sgrep executable is an empty placeholder at " + path + "; install/build a real native binary", path);
  if (checkAccess && (stat.mode & 0o111) === 0) fail("ASGREP_EXECUTABLE_NOT_EXECUTABLE", "ast-sgrep executable lacks an execute mode: " + path, path);
  if (checkAccess) {
    try { fs.accessSync(path, constants.X_OK); } catch (cause) { fail("ASGREP_EXECUTABLE_NOT_EXECUTABLE", "ast-sgrep executable is not executable: " + path, path, cause); }
  }
  return path;
}

function pathLookup(name, env, fs) {
  const pathEnv = env.PATH || env.Path || "";
  const delimiter = process.platform === "win32" ? ";" : ":";
  for (const dir of pathEnv.split(delimiter)) {
    if (!dir) continue;
    const candidate = join(dir, name);
    try {
      const stat = fs.statSync(candidate);
      if (stat.isFile() && stat.size > 0) return candidate;
    } catch {
      // try next
    }
  }
  return null;
}
function resolveHost(options = {}) {
  const fs = options.fs ?? { accessSync, existsSync, readFileSync, statSync };
  const env = options.env ?? process.env;
  const platform = options.platform ?? process.platform;
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
  return { fs, platform, packageName, executableName, packageDir: dirname(manifestPath), checksumPath: join(dirname(manifestPath), "checksum.sha256") };
}
export function resolveBinary(options = {}) {
  const env = options.env ?? process.env;
  const platform = options.platform ?? process.platform;
  const override = options.binaryPath ?? env.ASGREP_BIN ?? env.AST_SGREP_BINARY;
  const fs = options.fs ?? { accessSync, existsSync, readFileSync, statSync };
  if (override) return validateExecutable(resolve(override), fs, platform !== "win32");
  try {
    const host = resolveHost(options);
    const executablePath = join(host.packageDir, host.executableName);
    validateExecutable(executablePath, host.fs, platform !== "win32");
    let checksumText;
    try { checksumText = host.fs.readFileSync(host.checksumPath, "utf8"); } catch (cause) { fail("ASGREP_CHECKSUM_MISSING", "Native package checksum is missing: " + host.checksumPath, host.checksumPath, cause); }
    const expected = digestFor(checksumText, host.executableName, host.checksumPath);
    let actual;
    try { actual = createHash("sha256").update(host.fs.readFileSync(executablePath)).digest("hex"); } catch (cause) { fail("ASGREP_EXECUTABLE_MISSING", "Cannot read native executable: " + executablePath, executablePath, cause); }
    if (actual !== expected) fail("ASGREP_CHECKSUM_MISMATCH", "Native executable checksum mismatch at " + executablePath + "; reinstall " + host.packageName + "@" + VERSION, executablePath);
    return executablePath;
  } catch (error) {
    if (!(error instanceof AstSgrepBinaryError)) throw error;
    if (error.code !== "ASGREP_PLATFORM_PACKAGE_MISSING" && error.code !== "ASGREP_EXECUTABLE_EMPTY" && error.code !== "ASGREP_UNSUPPORTED_PLATFORM") {
      throw error;
    }
    const exeName = platform === "win32" ? "asgrep.exe" : "asgrep";
    const fromPath = pathLookup(exeName, env, fs);
    if (fromPath) return validateExecutable(fromPath, fs, platform !== "win32");
    throw error;
  }
}
/** Resolve the in-process Code Mode NAPI addon from the platform package, or null if absent. */
export function resolveCodemodeAddon(options = {}) {
  const env = options.env ?? process.env;
  const override = options.addonPath ?? env.ASGREP_CODEMODE_NAPI_PATH;
  const fs = options.fs ?? { accessSync, existsSync, readFileSync, statSync };
  if (override) {
    const path = resolve(override);
    let stat;
    try { stat = fs.statSync(path); } catch (cause) { fail("ASGREP_NAPI_MISSING", "Code Mode NAPI addon is missing at " + path, path, cause); }
    if (!stat.isFile() || stat.size === 0) fail("ASGREP_NAPI_INVALID", "Code Mode NAPI addon is not a non-empty file: " + path, path);
    return path;
  }
  let host;
  try { host = resolveHost({ ...options, fs }); } catch (error) {
    if (error instanceof AstSgrepBinaryError && (error.code === "ASGREP_UNSUPPORTED_PLATFORM" || error.code === "ASGREP_PLATFORM_PACKAGE_MISSING")) return null;
    throw error;
  }
  const addonPath = join(host.packageDir, NAPI_ADDON);
  const exists = typeof host.fs.existsSync === "function" ? host.fs.existsSync(addonPath) : existsSync(addonPath);
  if (!exists) return null;
  try {
    const addonStat = host.fs.statSync(addonPath);
    if (!addonStat.isFile() || addonStat.size === 0) return null;
  } catch {
    return null;
  }
  let checksumText;
  try { checksumText = host.fs.readFileSync(host.checksumPath, "utf8"); } catch (cause) { fail("ASGREP_CHECKSUM_MISSING", "Native package checksum is missing: " + host.checksumPath, host.checksumPath, cause); }
  const expected = digestFor(checksumText, NAPI_ADDON, host.checksumPath);
  let actual;
  try { actual = createHash("sha256").update(host.fs.readFileSync(addonPath)).digest("hex"); } catch (cause) { fail("ASGREP_NAPI_MISSING", "Cannot read Code Mode NAPI addon: " + addonPath, addonPath, cause); }
  if (actual !== expected) fail("ASGREP_CHECKSUM_MISMATCH", "Code Mode NAPI addon checksum mismatch at " + addonPath + "; reinstall " + host.packageName + "@" + VERSION, addonPath);
  return addonPath;
}
export { NAPI_ADDON };
