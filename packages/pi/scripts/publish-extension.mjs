#!/usr/bin/env node
/**
 * Dogfood lane: one-command pi-ast-sgrep publish from a committed tree.
 *
 *   npm run publish:pi-extension            # publish packages.extension.version
 *   npm run publish:pi-extension -- 2.1.1   # bump contract+manifest, then publish
 *
 * Gates (same rules the pi-v tag lane enforces, minus OIDC): contract and
 * manifest versions must agree, dependency range must match the contract,
 * the extension tree must be fully committed (packed content = what is on the
 * branch), contract check + build must pass. Publication itself is local
 * `npm publish` — provenance/OIDC remains exclusive to the pi-v tag lane.
 */
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const extensionDir = path.join(root, 'packages/pi/extension');
const contractPath = path.join(root, 'packages/pi/release-contract.json');
const manifestPath = path.join(extensionDir, 'package.json');
const fail = (message) => { console.error('publish:pi-extension: ' + message); process.exit(1); };
const run = (command, args, options = {}) => {
  const result = spawnSync(command, args, { cwd: options.cwd ?? root, encoding: 'utf8', stdio: options.inherit ? 'inherit' : 'pipe', windowsHide: true });
  if (result.status !== 0) fail(`${command} ${args.join(' ')} failed: ${String(result.stderr ?? result.stdout ?? '').trim().slice(-300)}`);
  return result.stdout ?? '';
};
const readJson = (file) => JSON.parse(readFileSync(file, 'utf8'));

const bump = process.argv[2];
if (bump !== undefined && !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(bump)) fail(`--bump must be semver, got "${bump}"`);

const contract = readJson(contractPath);
const spec = contract.packages?.extension ?? {};
const manifest = readJson(manifestPath);

if (bump) {
  manifest.version = bump;
  spec.version = bump;
  writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + '\n');
  writeFileSync(contractPath, JSON.stringify(contract, null, 2) + '\n');
  run('git', ['add', 'packages/pi/extension/package.json', 'packages/pi/release-contract.json']);
  run('git', ['commit', '-m', `chore: bump pi-ast-sgrep to ${bump}`]);
  console.log(`bumped + committed pi-ast-sgrep -> ${bump}`);
}

const version = manifest.version;
if (version !== spec.version) fail(`version skew: manifest ${version} vs contract packages.extension.version ${spec.version} — bump both or pass a version`);
if (manifest.dependencies?.['ast-sgrep'] !== spec.launcherRange) fail(`dependency skew: ast-sgrep must be "${spec.launcherRange}"`);

// Packed content must be committed content — the lane publishes off the branch,
// never off an uncommitted worktree.
const dirty = run('git', ['status', '--porcelain', '--', 'packages/pi/extension', 'packages/pi/release-contract.json']).trim();
if (dirty) fail('uncommitted changes under packages/pi/extension or the release contract; commit first\n' + dirty);

run('npm', ['run', 'check:pi-contract']);
run('npm', ['run', 'build'], { cwd: extensionDir });
const distDrift = run('git', ['status', '--porcelain', '--', 'packages/pi/extension/dist']).trim();
if (distDrift) fail('dist drift after build — commit the rebuilt dist first\n' + distDrift);

console.log(`publishing pi-ast-sgrep@${version} (local dogfood lane; the pi-v tag lane remains the attested path)`);
run('npm', ['publish', '--access', 'public'], { cwd: extensionDir, inherit: true });
