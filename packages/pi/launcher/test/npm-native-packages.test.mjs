import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { chmodSync, cpSync, existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { resolveBinary } from "../src/index.js";

const launcherDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(launcherDir, "../../..");
const targets = [
  { id: "darwin-arm64", name: "ast-sgrep-darwin-arm64", platform: "darwin", arch: "arm64", libc: "", executable: "asgrep" },
  { id: "darwin-x64", name: "ast-sgrep-darwin-x64", platform: "darwin", arch: "x64", libc: "", executable: "asgrep" },
  { id: "linux-arm64-gnu", name: "ast-sgrep-linux-arm64-gnu", platform: "linux", arch: "arm64", libc: "glibc", executable: "asgrep" },
  { id: "linux-x64-gnu", name: "ast-sgrep-linux-x64-gnu", platform: "linux", arch: "x64", libc: "glibc", executable: "asgrep" },
  { id: "win32-x64-msvc", name: "ast-sgrep-win32-x64-msvc", platform: "win32", arch: "x64", libc: "", executable: "asgrep.exe" }
];
function fixture(target = targets[0], changes = {}) {
  const root = mkdtempSync(join(tmpdir(), "ast-sgrep-native-"));
  const packageDir = join(root, target.name);
  mkdirSync(packageDir);
  const manifest = {
    name: target.name,
    version: changes.version ?? "1.2.0-alpha",
    os: [target.platform],
    cpu: [target.arch],
    ...(target.libc ? { libc: [target.libc] } : {})
  };
  const manifestPath = join(packageDir, "package.json");
  writeFileSync(manifestPath, changes.manifestText ?? JSON.stringify(manifest));
  const executablePath = join(packageDir, target.executable);
  const payload = changes.payload ?? Buffer.from("native fixture");
  if (!changes.missingExecutable) {
    writeFileSync(executablePath, payload);
    chmodSync(executablePath, changes.mode ?? 0o755);
  }
  const digest = createHash("sha256").update(payload).digest("hex");
  if (!changes.missingChecksum) writeFileSync(join(packageDir, "checksum.sha256"), (changes.checksum ?? digest) + "  " + target.executable + "\n");
  return { root, manifestPath, executablePath, options: { platform: target.platform, arch: target.arch, libc: target.libc, requireResolve: () => manifestPath } };
}
function expectCode(code, action, pathPart) {
  assert.throws(action, error => {
    assert.equal(error.code, code);
    if (pathPart) assert.match(error.path, pathPart);
    return true;
  });
}

function stagedPackage(root, target, payload = Buffer.from("staged native executable")) {
  const piDir = join(root, "packages/pi");
  const platformsDir = join(piDir, "platforms");
  const packageDir = join(platformsDir, target.id);
  mkdirSync(join(piDir, "release"), { recursive: true });
  mkdirSync(platformsDir, { recursive: true });
  cpSync(join(repoRoot, "packages/pi/release/targets.json"), join(piDir, "release/targets.json"));
  cpSync(join(repoRoot, "packages/pi/release-contract.json"), join(piDir, "release-contract.json"));
  cpSync(join(repoRoot, "packages/pi/platforms/prepack-verify.mjs"), join(platformsDir, "prepack-verify.mjs"));
  cpSync(join(repoRoot, "packages/pi/platforms", target.id), packageDir, { recursive: true });
  const executablePath = join(packageDir, target.executable);
  writeFileSync(executablePath, payload);
  chmodSync(executablePath, 0o755);
  writeFileSync(join(packageDir, "checksum.sha256"), createHash("sha256").update(payload).digest("hex") + "  " + target.executable + "\n");
  return packageDir;
}

test("resolves every supported host deterministically", () => {
  for (const target of targets) {
    const f = fixture(target);
    try { assert.equal(resolveBinary(f.options), f.executablePath); } finally { rmSync(f.root, { recursive: true, force: true }); }
  }
});

test("reports representative unsupported tuples and omitted packages", () => {
  for (const tuple of [
    { platform: "freebsd", arch: "x64" },
    { platform: "linux", arch: "x64", libc: "musl" },
    { platform: "win32", arch: "arm64" },
    { platform: "darwin", arch: "riscv64" }
  ]) expectCode("ASGREP_UNSUPPORTED_PLATFORM", () => resolveBinary({ ...tuple, env: {} }));
  expectCode("ASGREP_PLATFORM_PACKAGE_MISSING", () => resolveBinary({ platform: "linux", arch: "x64", libc: "glibc", env: {}, requireResolve() { throw new Error("omitted"); } }), /ast-sgrep-linux-x64-gnu/u);
});

test("committed target, contract, package, and checksum metadata do not drift", () => {
  const targetFile = JSON.parse(readFileSync(join(repoRoot, "packages/pi/release/targets.json"), "utf8")).targets;
  const contract = JSON.parse(readFileSync(join(repoRoot, "packages/pi/release-contract.json"), "utf8"));
  const launcher = JSON.parse(readFileSync(join(launcherDir, "package.json"), "utf8"));
  assert.deepEqual(targetFile.map(target => ({
    id: target.id,
    name: target.package,
    platform: target.os,
    arch: target.cpu,
    libc: target.libc ?? "",
    executable: target.executable
  })), targets);
  assert.deepEqual(contract.packages.platforms.map(platform => platform.name), targets.map(target => target.name));
  assert.deepEqual(launcher.repository, {
    type: "git",
    url: "git+https://github.com/AdityaVG13/ast-sgrep.git",
    directory: "packages/pi/launcher"
  });
  assert.deepEqual(Object.keys(launcher.optionalDependencies).sort(), targets.map(target => target.name).sort());
  for (const target of targets) {
    const packageDir = join(repoRoot, "packages/pi/platforms", target.id);
    const manifest = JSON.parse(readFileSync(join(packageDir, "package.json"), "utf8"));
    const contractPackage = contract.packages.platforms.find(platform => platform.name === target.name);
    assert.equal(manifest.name, target.name);
    assert.equal(manifest.version, contract.canonicalVersion.version);
    assert.deepEqual(manifest.os, [target.platform]);
    assert.deepEqual(manifest.cpu, [target.arch]);
    assert.deepEqual(manifest.libc ?? [], target.libc ? [target.libc] : []);
    assert.deepEqual(manifest.repository, {
      type: "git",
      url: "git+https://github.com/AdityaVG13/ast-sgrep.git",
      directory: "packages/pi/platforms/" + target.id
    });
    assert.equal(contractPackage.directory, "packages/pi/platforms/" + target.id);
    assert.equal(contractPackage.executable, target.executable);
    assert.equal(contractPackage.optionalDependencyVersion, contract.canonicalVersion.version);
    assert.equal(launcher.optionalDependencies[target.name], contract.canonicalVersion.version);
    const checksumText = readFileSync(join(packageDir, "checksum.sha256"), "utf8");
    assert.match(checksumText, new RegExp("^[0-9a-f]{64}  " + target.executable.replace(".", "\\.") + "\\n$", "u"));
    const checksum = checksumText.trim().split(/\s+/u);
    assert.equal(checksum[1], target.executable);
    assert.equal(checksum[0], createHash("sha256").update(readFileSync(join(packageDir, target.executable))).digest("hex"));
  }
});

test("validates checksum, executable presence, mode, version, and metadata", () => {
  const cases = [
    ["ASGREP_CHECKSUM_MISMATCH", { checksum: "0".repeat(64) }, /asgrep$/u],
    ["ASGREP_EXECUTABLE_MISSING", { missingExecutable: true }, /asgrep$/u],
    ["ASGREP_EXECUTABLE_NOT_EXECUTABLE", { mode: 0o644 }, /asgrep$/u],
    ["ASGREP_PLATFORM_VERSION_MISMATCH", { version: "1.0.0" }, /package\.json$/u],
    ["ASGREP_PLATFORM_METADATA_CORRUPT", { manifestText: "not json" }, /package\.json$/u]
  ];
  for (const [code, changes, pathPart] of cases) {
    const f = fixture(targets[0], changes);
    try { expectCode(code, () => resolveBinary(f.options), pathPart); } finally { rmSync(f.root, { recursive: true, force: true }); }
  }
});

test("npm omits a wrong-OS local optional package without registry access", () => {
  const root = mkdtempSync(join(tmpdir(), "ast-sgrep-optional-os-"));
  try {
    const nativeDir = join(root, "native");
    const appDir = join(root, "app");
    mkdirSync(nativeDir);
    mkdirSync(appDir);
    writeFileSync(join(nativeDir, "package.json"), JSON.stringify({ name: "ast-sgrep-win32-x64-msvc", version: "1.2.0-alpha", os: ["win32"], cpu: ["x64"] }));
    writeFileSync(join(appDir, "package.json"), JSON.stringify({ private: true, optionalDependencies: { "ast-sgrep-win32-x64-msvc": "file:../native" } }));
    const result = spawnSync("npm", ["install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund", "--os=linux", "--cpu=x64"], { cwd: appDir, encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(existsSync(join(appDir, "node_modules/ast-sgrep-win32-x64-msvc")), false);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("prepack verifier rejects missing binaries, bad checksum, mode, and metadata", () => {
  const target = targets[0];
  const cases = [
    ["ASGREP_PREPACK_EXECUTABLE_MISSING", packageDir => unlinkSync(join(packageDir, target.executable))],
    ["ASGREP_PREPACK_CHECKSUM_INVALID", packageDir => writeFileSync(join(packageDir, "checksum.sha256"), "bad checksum\n")],
    ["ASGREP_PREPACK_EXECUTABLE_MODE", packageDir => chmodSync(join(packageDir, target.executable), 0o644)],
    ["ASGREP_PREPACK_METADATA_MISMATCH", packageDir => {
      const path = join(packageDir, "package.json");
      const manifest = JSON.parse(readFileSync(path, "utf8"));
      manifest.version = "0.0.0";
      writeFileSync(path, JSON.stringify(manifest));
    }]
  ];
  for (const [code, corrupt] of cases) {
    const root = mkdtempSync(join(tmpdir(), "ast-sgrep-prepack-"));
    try {
      const packageDir = stagedPackage(root, target);
      corrupt(packageDir);
      const result = spawnSync(process.execPath, [join(dirname(packageDir), "prepack-verify.mjs")], { cwd: packageDir, encoding: "utf8" });
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, new RegExp(code, "u"));
    } finally { rmSync(root, { recursive: true, force: true }); }
  }
});

test("raw placeholders cannot pack and staged inventories are exact", () => {
  const launcher = JSON.parse(spawnSync("npm", ["pack", "--json", "--dry-run"], { cwd: launcherDir, encoding: "utf8" }).stdout)[0];
  assert.deepEqual(launcher.files.map(file => file.path).sort(), ["LICENSE", "README.md", "bin/asgrep.js", "package.json", "src/index.d.ts", "src/index.js"]);
  assert.match(launcher.integrity, /^sha512-[A-Za-z0-9+/]+={0,2}$/u);
  assert.match(launcher.shasum, /^[0-9a-f]{40}$/u);
  for (const target of targets) {
    const sourceDir = join(repoRoot, "packages/pi/platforms", target.id);
    const rejected = spawnSync("npm", ["pack", "--json", "--dry-run"], { cwd: sourceDir, encoding: "utf8" });
    assert.notEqual(rejected.status, 0);
    assert.match(rejected.stderr, /ASGREP_PREPACK_EXECUTABLE_EMPTY/u);
    const root = mkdtempSync(join(tmpdir(), "ast-sgrep-stage-"));
    try {
      const packageDir = stagedPackage(root, target);
      const result = spawnSync("npm", ["pack", "--json", "--dry-run"], { cwd: packageDir, encoding: "utf8" });
      assert.equal(result.status, 0, result.stderr);
      const packed = JSON.parse(result.stdout)[0];
      const inventory = packed.files.map(file => file.path).sort();
      assert.deepEqual(inventory, ["LICENSE", "checksum.sha256", "package.json", target.executable].sort());
      assert.match(packed.integrity, /^sha512-[A-Za-z0-9+/]+={0,2}$/u);
      assert.match(packed.shasum, /^[0-9a-f]{40}$/u);
    } finally { rmSync(root, { recursive: true, force: true }); }
  }
});

test("packed launcher install executes both aliases and preserves argv", () => {
  const host = targets.find(target => target.platform === process.platform && target.arch === process.arch && (target.platform !== "linux" || target.libc === "glibc"));
  assert.ok(host, "test host must be in the supported release matrix");
  const root = mkdtempSync(join(tmpdir(), "ast-sgrep-install-"));
  try {
    const program = "#!/usr/bin/env node\nprocess.stdout.write(JSON.stringify(process.argv.slice(2)));\n";
    const platformCopy = stagedPackage(root, host, Buffer.from(program));
    const packPlatform = spawnSync("npm", ["pack", "--json", "--pack-destination", root], { cwd: platformCopy, encoding: "utf8" });
    assert.equal(packPlatform.status, 0, packPlatform.stderr);
    const packLauncher = spawnSync("npm", ["pack", "--json", "--pack-destination", root], { cwd: launcherDir, encoding: "utf8" });
    assert.equal(packLauncher.status, 0, packLauncher.stderr);
    const platformTar = join(root, JSON.parse(packPlatform.stdout)[0].filename);
    const launcherTar = join(root, JSON.parse(packLauncher.stdout)[0].filename);
    const fixtureDir = join(root, "fixture");
    mkdirSync(fixtureDir);
    writeFileSync(join(fixtureDir, "package.json"), JSON.stringify({ private: true, dependencies: { "ast-sgrep": "file:" + launcherTar, [host.name]: "file:" + platformTar } }));
    const install = spawnSync("npm", ["install", "--ignore-scripts", "--no-audit", "--no-fund"], { cwd: fixtureDir, encoding: "utf8" });
    assert.equal(install.status, 0, install.stderr);
    for (const alias of ["asgrep", "ast-sgrep"]) {
      const result = spawnSync(join(fixtureDir, "node_modules/.bin", alias), ["space value", "--flag=✓"], { encoding: "utf8", env: { ...process.env, PATH: process.env.PATH } });
      assert.equal(result.status, 0, result.stderr);
      assert.deepEqual(JSON.parse(result.stdout), ["space value", "--flag=✓"]);
    }
  } finally { rmSync(root, { recursive: true, force: true }); }
});
