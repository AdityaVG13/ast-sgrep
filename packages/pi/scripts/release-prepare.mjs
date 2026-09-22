import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// release:prepare X.Y.Z — mechanical version bump for the canonical family.
//
// Covers version identity only: Cargo workspace + inter-crate path deps +
// lockfile, the npm workspace/launcher/platform/extension manifests (the
// extension rides lockstep; the pi-v lane remains for out-of-band revs),
// the release contract canonical fields, the embedded source constants (+
// tsc dist rebuild), and the lockstep satellite manifests (agent-plugin,
// vscode, napi helper). Then runs the contract, workflow, gate, and plugin
// checks. The lockfile is deliberately NOT regenerated here: the new
// version is unpublished, so regenerating would record unresolvable
// {"optional": true} stubs (br-zvh) — the release workflow re-pins after
// publication.
//
// Deliberately human: CHANGELOG prose, README (banner/link/status),
// RELEASING.md release_version blocks, docs/pi-package.md pointer,
// launcherRange judgment, and the post-tag Homebrew re-pin. The script
// prints that remainder checklist at the end.

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const fail = (code, message) => { throw new Error(`${code}: ${message}`); };
const run = (command, args) => {
  const result = spawnSync(command, args, { cwd: root, encoding: 'utf8', stdio: 'inherit', windowsHide: true });
  if (result.status !== 0) fail('ASGREP_PREPARE_COMMAND', `${command} ${args.join(' ')} failed (${result.status})`);
};
const runCapture = (command, args) => {
  const result = spawnSync(command, args, { cwd: root, encoding: 'utf8', windowsHide: true });
  if (result.status !== 0) fail('ASGREP_PREPARE_COMMAND', `${command} ${args.join(' ')} failed (${result.status})`);
  return (result.stdout ?? '').trim();
};

const next = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(next ?? '')) fail('ASGREP_PREPARE_USAGE', 'usage: release:prepare X.Y.Z (plain semver, no prerelease tag)');
if (!process.argv.includes('--allow-dirty') && runCapture('git', ['status', '--porcelain']) !== '') {
  fail('ASGREP_PREPARE_DIRTY', 'working tree must be clean (or pass --allow-dirty for a local round-trip test)');
}

const contractPath = path.join(root, 'packages/pi/release-contract.json');
const contractText = readFileSync(contractPath, 'utf8');
const contract = JSON.parse(contractText);
const current = contract.canonicalVersion.version;
const extensionCurrent = contract.packages.extension.version;
if (next === current) fail('ASGREP_PREPARE_USAGE', `${next} is already the canonical version`);

// Exact-count line replacement: every bump asserts how many lines it must
// hit, so contract/manifest drift fails loudly instead of half-bumping.
const swap = (relativePath, predicate, expected, label, from = current) => {
  const file = path.join(root, relativePath);
  const lines = readFileSync(file, 'utf8').split('\n');
  let hits = 0;
  const updated = lines.map((line) => {
    if (!predicate(line)) return line;
    hits += 1;
    return line.split(from).join(next);
  });
  if (hits !== expected) fail('ASGREP_PREPARE_DRIFT', `${relativePath}: ${label} hit ${hits} lines, expected ${expected}`);
  writeFileSync(file, updated.join('\n'));
  console.log(`[prepare] ${relativePath}: ${label} (${hits})`);
};
const versionLine = (line) => line.includes(`"version": "${current}"`);
const extensionVersionLine = (line) => line.includes(`"version": "${extensionCurrent}"`);

// 1. Cargo workspace version + inter-crate path deps (ast-sgrep path lines only).
swap('Cargo.toml', (line) => /^\s*version\s*=\s*"[^"]+"\s*$/.test(line) && line.includes(`"${current}"`), 1, 'workspace version');
for (const manifest of ['ast-sgrep-cli', 'ast-sgrep-codemode-napi', 'ast-sgrep-codemode', 'ast-sgrep-core', 'ast-sgrep-lsp', 'ast-sgrep-mcp', 'ast-sgrep-plugins', 'ast-sgrep-testkit', 'ast-sgrep-watch']) {
  const file = `crates/${manifest}/Cargo.toml`;
  const text = readFileSync(path.join(root, file), 'utf8');
  const expected = text.split('\n').filter((line) => line.includes('path = "../ast-sgrep-') && line.includes(`version = "${current}"`)).length;
  if (expected === 0) fail('ASGREP_PREPARE_DRIFT', `${file}: no inter-crate path deps at ${current}`);
  swap(file, (line) => line.includes('path = "../ast-sgrep-') && line.includes(`version = "${current}"`), expected, 'path dep versions');
}

// 2. npm manifests: "version" fields + launcher optionalDependencies.
const manifests = [
  'package.json',
  'packages/pi/launcher/package.json',
  'packages/pi/extension/package.json',
  'packages/pi/platforms/darwin-arm64/package.json',
  'packages/pi/platforms/darwin-x64/package.json',
  'packages/pi/platforms/linux-arm64-gnu/package.json',
  'packages/pi/platforms/linux-x64-gnu/package.json',
  'packages/pi/platforms/win32-x64-msvc/package.json',
  'packages/agent-plugin/package.json',
  'packages/agent-plugin/plugin.json',
  'editors/vscode/package.json',
  'crates/ast-sgrep-codemode-napi/package.json',
];
for (const manifest of manifests) {
  const extension = manifest === 'packages/pi/extension/package.json';
  swap(manifest, extension ? extensionVersionLine : versionLine, 1, 'manifest version', extension ? extensionCurrent : current);
}
swap('packages/pi/launcher/package.json', (line) => line.includes('"@ast-sgrep/') && line.includes(`"${current}"`), 5, 'platform optionalDependencies');

// 3. Release contract canonical fields (launcherRange untouched).
swap('packages/pi/release-contract.json', versionLine, extensionCurrent === current ? 5 : 4, 'canonical versions');
if (extensionCurrent !== current) {
  swap('packages/pi/release-contract.json', extensionVersionLine, 1, 'extension version', extensionCurrent);
}
swap('packages/pi/release-contract.json', (line) => line.includes(`"tag": "v${current}"`), 1, 'official tag');
swap('packages/pi/release-contract.json', (line) => line.includes(`"nativeCliVersion": "${current}"`), 1, 'native CLI version');
swap('packages/pi/release-contract.json', (line) => line.includes(`"optionalDependencyVersion": "${current}"`), 5, 'platform pins');

// 4. Embedded source constants.
swap('packages/pi/launcher/src/index.js', (line) => line.includes(`const VERSION = "${current}";`), 1, 'launcher VERSION');
swap('packages/pi/extension/src/codemode/native.ts', (line) => line.includes(`CODEMODE_BINDING_VERSION = "${current}"`), 1, 'binding version');
swap('packages/pi/extension/src/runtime/types.ts', (line) => line.includes(`RUNTIME_VERSION = "${current}"`), 1, 'runtime version');

// 5. Regenerate lockfile + dist, then run the check battery.
console.log('[prepare] regenerating Cargo.lock');
run('cargo', ['check', '-p', 'ast-sgrep-lang']);
console.log('[prepare] rebuilding extension dist');
run('npm', ['run', 'build', '--workspace', 'pi-ast-sgrep']);
for (const check of ['check:pi-contract', 'check:pi-release', 'test:pi-release-gate', 'check:agent-plugin']) {
  console.log(`[prepare] ${check}`);
  run('npm', ['run', check]);
}

console.log(`[prepare] mechanical bump ${current} -> ${next} complete; human remainder:`);
for (const item of [
  'CHANGELOG.md entry (prose)',
  'README.md banner + release link + status paragraph',
  'docs/RELEASING.md release_version blocks',
  'docs/pi-package.md contract pointer',
  'launcherRange judgment (extension floor/cap)',
  'commit + push, then npm run release (signs tag, dispatches, one approval); Homebrew re-pin after the tag',
]) console.log(`[prepare]   - ${item}`);
