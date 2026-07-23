import { spawnSync } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const commit = 'f'.repeat(40);
const readJson = async (file) => JSON.parse(await readFile(file, 'utf8'));
const sh = (command, args, options = {}) => spawnSync(command, args, { cwd: root, encoding: 'utf8', windowsHide: true, ...options });
const results = [];
const record = (name, ok, detail = '') => { results.push({ name, ok, detail }); console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? ' — ' + detail : ''}`); };
const runStep = (name, command, args) => {
  const result = sh(command, args);
  const ok = result.status === 0;
  record(name, ok, ok ? '' : String(result.stderr || result.stdout || '').trim().split('\n').slice(-1)[0]);
  return ok;
};

const state = await readJson(path.join(root, 'packages/pi/release-contract.json'));
const version = state.canonicalVersion.version;
const launcher = await readJson(path.join(root, 'packages/pi/launcher/package.json'));
const extension = await readJson(path.join(root, 'packages/pi/extension/package.json'));
const targets = (await readJson(path.join(root, 'packages/pi/release/targets.json'))).targets;
const packageNames = [...targets.map((target) => target.package), launcher.name, extension.name];

console.log(`[preflight] validating release ${version} (tag ${state.canonicalVersion.tag}) for ${packageNames.length} packages\n`);

const errorLine = (result) => String(result.stderr || result.stdout || '').split('\n').filter((line) => /npm error/u.test(line) && !/A complete log|debug-0\.log/u.test(line)).slice(-1)[0]?.trim() || String(result.stderr || result.stdout || '').trim().split('\n').slice(-1)[0];

runStep('contract consistency', process.execPath, ['packages/pi/scripts/check-contract.mjs']);
runStep('workflow structure', process.execPath, ['packages/pi/scripts/check-native-workflow.mjs']);
runStep('gate self-test', process.execPath, ['packages/pi/scripts/release-acceptance.mjs', 'self-test']);

const live = [];
const fresh = [];
let registryOk = true;
for (const name of packageNames) {
  const view = sh('npm', ['view', `${name}@${version}`, 'version', '--json']);
  if (view.status === 0) live.push(name);
  else if (/E404|404 Not Found|is not in this registry/u.test(view.stderr + view.stdout)) fresh.push(name);
  else { registryOk = false; console.log(`      registry query failed for ${name}@${version}: ${errorLine(view)}`); }
}
record('registry reachable', registryOk);

const temporary = await mkdtemp(path.join(tmpdir(), 'ast-sgrep-preflight-'));
const nativeDir = path.join(temporary, 'native');
const packsDir = path.join(temporary, 'packs');
try {
  const fixture = runStep('structural native fixtures', process.execPath, ['packages/pi/scripts/release-acceptance.mjs', 'fixture-native', '--output', nativeDir]);
  const packed = fixture && runStep('pack seven-package family', process.execPath, ['packages/pi/scripts/release-acceptance.mjs', 'pack', '--native-root', nativeDir, '--output', packsDir, '--commit', commit]);
  if (packed) {
    runStep('verify packed family', process.execPath, ['packages/pi/scripts/release-acceptance.mjs', 'verify', '--artifacts', packsDir]);
    const manifest = await readJson(path.join(packsDir, 'release-manifest.json'));
    const toDryRun = manifest.artifacts.filter((artifact) => fresh.includes(artifact.name));
    if (toDryRun.length === 0) {
      record('npm publish --dry-run', true, `skipped — all ${live.length} already live at ${version}`);
    } else {
      let dryOk = true;
      for (const artifact of toDryRun) {
        const dry = sh('npm', ['publish', '--dry-run', '--access', 'public', path.join(packsDir, artifact.filename)]);
        if (dry.status !== 0) { dryOk = false; console.log(`      dry-run failed for ${artifact.name}: ${errorLine(dry)}`); }
      }
      record('npm publish --dry-run', dryOk, `${toDryRun.length} unpublished package(s)`);
    }
  }
} finally {
  await rm(temporary, { recursive: true, force: true });
}

console.log(`\n[preflight] registry state at ${version}: ${fresh.length} to publish, ${live.length} already live`);
if (fresh.length) console.log(`  will publish: ${fresh.join(', ')}`);
if (live.length) console.log(`  idempotent skip: ${live.join(', ')}`);
if (live.length === packageNames.length) console.log(`  NOTE: version ${version} is fully published; bump the canonical version before dispatching a new release.`);

const failed = results.filter((entry) => !entry.ok);
console.log(`\n[preflight] ${results.length - failed.length}/${results.length} checks passed`);
if (failed.length) { console.error(`[preflight] FAILED: ${failed.map((entry) => entry.name).join(', ')}`); process.exit(1); }
console.log('[preflight] OK — safe to sign the tag and dispatch pi-npm-release.yml');
