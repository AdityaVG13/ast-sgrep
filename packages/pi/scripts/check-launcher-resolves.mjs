#!/usr/bin/env node
/**
 * Extension-lane guard: the extension's launcherRange must resolve to at least
 * one PUBLISHED launcher version before the extension may publish.
 *
 * Incident (2026-09-19): pi-ast-sgrep@2.2.0 declared ast-sgrep@">=2.1.0 <3"
 * while the registry's newest launcher was 2.0.0 — the canonical v2.1.0 family
 * (platforms, then launcher) had never been published. Every install failed:
 * npm with ETARGET ("No matching version found for ast-sgrep@>=2.1.0 <3") and
 * bun/omp with "ast-sgrep@>=2.1.0 <3 failed to resolve". Both publish lanes
 * only checked manifest<->contract agreement, never registry resolution.
 *
 * Both extension lanes run this guard before any npm side effect:
 *   - packages/pi/scripts/publish-extension.mjs (local dogfood lane)
 *   - release-acceptance.mjs gate --lane extension (signed pi-v tag lane)
 *
 * Usage:
 *   node packages/pi/scripts/check-launcher-resolves.mjs
 *   node packages/pi/scripts/check-launcher-resolves.mjs --snapshot <file>
 * Hermetic mode: <file> holds a JSON array of published launcher versions
 * (or {"versions": [...]}); the registry is not contacted.
 */
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const RANGE_SHAPE = /^>=(\d+)\.(\d+)\.(\d+) <(\d+)$/u;
const VERSION_SHAPE = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/u;
const NOT_FOUND = /E404|404 Not Found|is not in this registry/u;

/** Parse the contract-constrained `>=<floor> <<major>` range. Throws on any other shape. */
export function parseLauncherRange(range) {
  const match = RANGE_SHAPE.exec(range ?? '');
  if (!match) throw new Error(`ASGREP_RELEASE_LAUNCHER_RANGE_SHAPE: launcherRange must be a bounded >=<floor> <<major> range, got ${JSON.stringify(range)}`);
  return { floor: [Number(match[1]), Number(match[2]), Number(match[3])], capMajor: Number(match[4]) };
}

function satisfiesParsed({ floor, capMajor }, version) {
  const match = VERSION_SHAPE.exec(version ?? '');
  // Prereleases never satisfy: npm excludes them unless the range itself
  // carries a prerelease on the same tuple, which this shape forbids.
  // Unparseable versions never match either.
  if (!match || match[4] !== undefined) return false;
  const tuple = [Number(match[1]), Number(match[2]), Number(match[3])];
  if (tuple[0] >= capMajor) return false;
  for (let index = 0; index < 3; index += 1) {
    if (tuple[index] !== floor[index]) return tuple[index] > floor[index];
  }
  return true;
}

/**
 * npm-consistent satisfaction for this range shape: numeric tuple >= floor,
 * major < cap. Throws on a malformed range.
 */
export function launcherRangeSatisfies(range, version) {
  return satisfiesParsed(parseLauncherRange(range), version);
}

/** Published versions satisfying the range, in registry order. Throws on a malformed range. */
export function resolvingVersions(range, versions) {
  const parsed = parseLauncherRange(range);
  return (versions ?? []).filter((version) => satisfiesParsed(parsed, version));
}

/** Fail closed unless at least one published launcher version satisfies the range. Returns the hits. */
export function assertLauncherRangeResolves({ launcher, range, extension, canonical, versions }) {
  const hits = resolvingVersions(range, versions);
  if (hits.length > 0) return hits;
  const have = versions?.length ? versions.join(', ') : 'none';
  throw new Error(
    `ASGREP_RELEASE_LAUNCHER_UNRESOLVED: extension ${extension} requires ${launcher}@"${range}" but no published ${launcher} version satisfies it (registry has: ${have}). ` +
    `Publish the canonical family first — a signed v${canonical} tag publishes platforms, then launcher — and re-run this lane.`
  );
}

/** Published launcher versions from the registry. [] when the name was never published. */
export function fetchPublishedVersions(name) {
  const result = spawnSync('npm', ['view', name, 'versions', '--json'], { cwd: root, encoding: 'utf8', windowsHide: true });
  if (result.status === 0) {
    const parsed = JSON.parse(result.stdout || 'null');
    // npm prints a bare string when exactly one version exists.
    if (typeof parsed === 'string') return [parsed];
    if (Array.isArray(parsed) && parsed.every((entry) => typeof entry === 'string')) return parsed;
    throw new Error(`ASGREP_RELEASE_LAUNCHER_REGISTRY: unexpected versions payload for ${name}: ${(result.stdout ?? '').trim().slice(-200)}`);
  }
  const diagnostic = `${result.stderr ?? ''}${result.stdout ?? ''}`;
  if (NOT_FOUND.test(diagnostic)) return [];
  throw new Error(`ASGREP_RELEASE_LAUNCHER_REGISTRY: could not list published ${name} versions: ${diagnostic.trim().slice(-300)}`);
}

function readSnapshot(file) {
  if (!file) throw new Error('ASGREP_RELEASE_LAUNCHER_SNAPSHOT: missing value for --snapshot');
  const raw = JSON.parse(readFileSync(path.resolve(root, file), 'utf8'));
  const versions = Array.isArray(raw) ? raw : raw?.versions;
  if (!Array.isArray(versions) || !versions.every((entry) => typeof entry === 'string')) {
    throw new Error(`ASGREP_RELEASE_LAUNCHER_SNAPSHOT: ${file} must be a JSON array of version strings or {"versions": [...]}`);
  }
  return versions;
}

const invokedAsMain = process.argv[1] !== undefined && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedAsMain) {
  try {
    const contract = JSON.parse(readFileSync(path.join(root, 'packages/pi/release-contract.json'), 'utf8'));
    const manifest = JSON.parse(readFileSync(path.join(root, 'packages/pi/extension/package.json'), 'utf8'));
    const launcher = contract.packages?.launcher?.name ?? 'ast-sgrep';
    const range = contract.packages?.extension?.launcherRange;
    const extension = `${manifest.name}@${manifest.version}`;
    if (manifest.dependencies?.[launcher] !== range) {
      throw new Error(`ASGREP_RELEASE_VERSION_SKEW: dependency skew: ${launcher} must be "${range}" (manifest declares "${manifest.dependencies?.[launcher]}")`);
    }
    const flag = process.argv.indexOf('--snapshot');
    const versions = flag < 0 ? fetchPublishedVersions(launcher) : readSnapshot(process.argv[flag + 1]);
    const hits = assertLauncherRangeResolves({ launcher, range, extension, canonical: contract.canonicalVersion?.version, versions });
    console.log(`[pi-release] launcherRange ${range} resolves: ${hits.map((version) => `${launcher}@${version}`).join(', ')}`);
  } catch (error) {
    console.error(`check-launcher-resolves: ${error instanceof Error ? error.message : String(error)}`);
    process.exit(1);
  }
}
