import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const launcherDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(launcherDir, "../../..");
const extensionDir = join(repoRoot, "packages/pi/extension");
const targets = JSON.parse(readFileSync(join(repoRoot, "packages/pi/release/targets.json"), "utf8")).targets;
const contract = JSON.parse(readFileSync(join(repoRoot, "packages/pi/release-contract.json"), "utf8"));
const canonicalVersion = contract.canonicalVersion.version;
const repositoryUrl = "git+https://github.com/AdityaVG13/ast-sgrep.git";

const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));
const productionDependencies = (manifest) => ({
  ...manifest.dependencies,
  ...manifest.optionalDependencies,
});

test("every public npm package carries license and source provenance", () => {
  const packages = [
    [extensionDir, "packages/pi/extension"],
    [launcherDir, "packages/pi/launcher"],
    ...targets.map((target) => [join(repoRoot, "packages/pi/platforms", target.id), "packages/pi/platforms/" + target.id]),
  ];
  for (const [directory, repositoryDirectory] of packages) {
    const manifest = readJson(join(directory, "package.json"));
    assert.equal(manifest.version, canonicalVersion, manifest.name);
    assert.equal(manifest.license, "MIT", manifest.name);
    assert.equal(existsSync(join(directory, "LICENSE")), true, manifest.name + " must ship a package-local license");
    assert.deepEqual(manifest.repository, {
      type: "git",
      url: repositoryUrl,
      directory: repositoryDirectory,
    }, manifest.name);
  }
});

test("launcher native dependency family is exact and extension launcher dependency is exact", () => {
  const launcher = readJson(join(launcherDir, "package.json"));
  const extension = readJson(join(extensionDir, "package.json"));
  assert.deepEqual(Object.keys(launcher.optionalDependencies).sort(), targets.map((target) => target.package).sort());
  for (const dependency of Object.values(launcher.optionalDependencies)) assert.equal(dependency, canonicalVersion);
  assert.equal(extension.dependencies[launcher.name], canonicalVersion);
});

test("package runtime has no telemetry, credential integration, or network downloader", () => {
  const manifests = [
    readJson(join(extensionDir, "package.json")),
    readJson(join(launcherDir, "package.json")),
  ];
  const forbiddenDependency = /(telemetry|analytics|sentry|opentelemetry|credential|keychain|oauth)/iu;
  for (const manifest of manifests) {
    for (const name of Object.keys(productionDependencies(manifest))) {
      assert.doesNotMatch(name, forbiddenDependency, manifest.name + " dependency " + name);
    }
  }
  const runtimeFiles = [
    join(extensionDir, "src/index.ts"),
    join(extensionDir, "src/runtime.ts"),
    join(launcherDir, "src/index.js"),
    join(launcherDir, "bin/asgrep.js"),
  ];
  const forbiddenRuntime = /(fetch\s*\(|https?:\/\/|API_KEY|PASSWORD|SECRET|process\.env\.(?:TOKEN|KEY|CREDENTIAL)|telemetry|analytics|sentry|opentelemetry)/iu;
  for (const path of runtimeFiles) assert.doesNotMatch(readFileSync(path, "utf8"), forbiddenRuntime, path);
});

test("provenance gate and user-facing security disclosures are explicit", () => {
  assert.equal(contract.firstPublication.provenanceRequired, true);
  assert.equal(contract.firstPublication.trustedPublishingRequired, true);
  assert.deepEqual(contract.registries.sharedAnchor, [
    "signed official tag",
    "commit SHA",
    "canonical workspace version",
    "artifact checksums",
  ]);
  const docs = readFileSync(join(repoRoot, "docs/pi-package.md"), "utf8");
  for (const disclosure of [
    /full-system access as the OS user running Pi/iu,
    /<project-root>\/\.asgrep/iu,
    /leaves `\.asgrep` behind/iu,
    /sends no telemetry/iu,
    /does not inspect Pi\/provider credential APIs/iu,
    /source text and queries needed for embeddings may be sent/iu,
  ]) assert.match(docs, disclosure);
});
