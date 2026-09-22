import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { assertLauncherRangeResolves, launcherRangeSatisfies, resolvingVersions } from "../../../packages/pi/scripts/check-launcher-resolves.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../../..");
const guard = join(root, "packages/pi/scripts/check-launcher-resolves.mjs");
const contract = JSON.parse(readFileSync(join(root, "packages/pi/release-contract.json"), "utf8"));
const liveRange = contract.packages.extension.launcherRange;
const liveFloor = liveRange.match(/^>=(\d+\.\d+\.\d+) <\d+$/u)?.[1];
assert.ok(liveFloor, `contract launcherRange has unexpected shape: ${liveRange}`);

const runGuard = (snapshot) => {
  const directory = mkdtempSync(join(tmpdir(), "ast-sgrep-launcher-resolves-"));
  const file = join(directory, "versions.json");
  writeFileSync(file, JSON.stringify(snapshot));
  return spawnSync(process.execPath, [guard, "--snapshot", file], { cwd: root, encoding: "utf8" });
};

test("range satisfaction is npm-consistent: floor, cap, prerelease, and garbage", () => {
  assert.equal(launcherRangeSatisfies(">=2.1.0 <3", "2.1.0"), true);
  assert.equal(launcherRangeSatisfies(">=2.1.0 <3", "2.9.7"), true);
  assert.equal(launcherRangeSatisfies(">=2.1.0 <3", "2.0.0"), false);
  assert.equal(launcherRangeSatisfies(">=2.1.0 <3", "3.0.0"), false);
  assert.equal(launcherRangeSatisfies(">=2.1.0 <3", "2.1.0-alpha"), false);
  assert.equal(launcherRangeSatisfies(">=2.1.0 <3", "not-a-version"), false);
  assert.deepEqual(resolvingVersions(">=2.1.0 <3", ["1.3.2", "1.4.0", "2.0.0", "2.1.0", "3.0.0"]), ["2.1.0"]);
  assert.throws(() => resolvingVersions("^2.1.0", ["2.1.0"]), /ASGREP_RELEASE_LAUNCHER_RANGE_SHAPE/);
});

test("guard rejects the 2026-09-19 incident shape: floor above every published launcher", () => {
  // pi-ast-sgrep@2.2.0 declared >=2.1.0 <3 while the registry held only
  // 1.3.2/1.4.0/2.0.0 — npm ETARGET and bun "failed to resolve" on install.
  assert.throws(
    () => assertLauncherRangeResolves({ launcher: "ast-sgrep", range: ">=2.1.0 <3", extension: "pi-ast-sgrep@2.2.0", canonical: "2.1.0", versions: ["1.3.2", "1.4.0", "2.0.0"] }),
    /ASGREP_RELEASE_LAUNCHER_UNRESOLVED.*pi-ast-sgrep@2\.2\.0 requires ast-sgrep@">=2\.1\.0 <3"/
  );
  assert.deepEqual(
    assertLauncherRangeResolves({ launcher: "ast-sgrep", range: ">=2.1.0 <3", extension: "pi-ast-sgrep@2.2.0", canonical: "2.1.0", versions: ["2.0.0", "2.1.0"] }),
    ["2.1.0"]
  );
});

test("Pi updates require the 2.5.4 native boundary fixes", () => {
  const extension = JSON.parse(readFileSync(join(root, "packages/pi/extension/package.json"), "utf8"));
  assert.equal(launcherRangeSatisfies(extension.dependencies["ast-sgrep"], "2.0.0"), false);
  assert.equal(launcherRangeSatisfies(extension.dependencies["ast-sgrep"], "2.5.2"), false);
  assert.equal(launcherRangeSatisfies(extension.dependencies["ast-sgrep"], "2.5.4"), true);
});

test("guard CLI rejects an unpublished launcher family against the live contract range", () => {
  const result = runGuard([]);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /ASGREP_RELEASE_LAUNCHER_UNRESOLVED/);
  assert.match(result.stderr, /no published ast-sgrep version satisfies it/);
  assert.match(result.stderr, /Publish the canonical family first/);
});

test("guard CLI accepts once the live floor is published", () => {
  const result = runGuard([liveFloor]);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, new RegExp(`launcherRange .* resolves: ast-sgrep@${liveFloor.replace(/\./gu, "\\.")}`));
});

test("guard CLI fails closed on a malformed snapshot", () => {
  const result = runGuard({ nope: true });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /ASGREP_RELEASE_LAUNCHER_SNAPSHOT/);
});
