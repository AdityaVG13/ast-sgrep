import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const machineSchema = '1.0.0';
const hosts = new Map([
  ['darwin:arm64', { directory: 'darwin-arm64', packageName: '@ast-sgrep/darwin-arm64', executable: 'asgrep' }],
  ['darwin:x64', { directory: 'darwin-x64', packageName: '@ast-sgrep/darwin-x64', executable: 'asgrep' }],
  ['linux:arm64', { directory: 'linux-arm64-gnu', packageName: '@ast-sgrep/linux-arm64-gnu', executable: 'asgrep' }],
  ['linux:x64', { directory: 'linux-x64-gnu', packageName: '@ast-sgrep/linux-x64-gnu', executable: 'asgrep' }],
  ['win32:x64', { directory: 'win32-x64-msvc', packageName: '@ast-sgrep/win32-x64-msvc', executable: 'asgrep.exe' }],
]);
const host = hosts.get(`${process.platform}:${process.arch}`);
if (!host) throw new Error(`two-version E2E has no local artifact target for ${process.platform}:${process.arch}`);

const temporary = await mkdtemp(path.join(tmpdir(), 'asgrep-pi-upgrade-'));
const project = path.join(temporary, 'project');
const agentDir = path.join(temporary, 'agent');
const piEntry = fileURLToPath(import.meta.resolve('@earendil-works/pi-coding-agent'));
const piCli = path.join(path.dirname(piEntry), 'cli.js');
const currentArtifact = process.env.ASGREP_CURRENT_ARTIFACT ? path.resolve(process.env.ASGREP_CURRENT_ARTIFACT) : undefined;
if (!currentArtifact) throw new Error('two-version-e2e.mjs runs only via npm run test:pi-e2e: ASGREP_CURRENT_ARTIFACT is unset');
const expectedVersions = {
  extension: process.env.ASGREP_EXPECTED_EXTENSION_VERSION,
  launcher: process.env.ASGREP_EXPECTED_LAUNCHER_VERSION,
  native: process.env.ASGREP_EXPECTED_NATIVE_VERSION,
};
if (!expectedVersions.extension || !expectedVersions.launcher || !expectedVersions.native) {
  throw new Error('two-version-e2e.mjs requires ASGREP_EXPECTED_{EXTENSION,LAUNCHER,NATIVE}_VERSION (set by npm run test:pi-e2e)');
}
const source = 'npm:pi-ast-sgrep@file:' + currentArtifact;
const commandEnv = {
  ...process.env,
  PI_CODING_AGENT_DIR: agentDir,
  npm_config_offline: 'true',
  npm_config_audit: 'false',
  npm_config_fund: 'false',
  NO_COLOR: '1',
};

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? root,
    env: { ...commandEnv, ...options.env },
    encoding: 'utf8',
    timeout: options.timeout ?? 600_000,
    windowsHide: true,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed (${result.status})\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`);
  }
  return result.stdout.trim();
}

function runJson(command, args, options, strictEnvelope = true) {
  const output = run(command, args, options);
  const value = JSON.parse(output);
  if (strictEnvelope) {
    assert.equal(value.tool, "asgrep");
    assert.equal(value.schema_version, machineSchema);
    assert.equal(value.ok, true);
  }
  return value;
}
function stage(name, action) {
  const started = Date.now();
  console.error(`[stage:${name}] START`);
  return Promise.resolve().then(action).then(
    (value) => {
      console.error(`[stage:${name}] PASS ${Date.now() - started}ms`);
      return value;
    },
    (error) => {
      console.error(`[stage:${name}] FAIL ${Date.now() - started}ms`);
      throw error;
    },
  );
}

function json(pathname) {
  return readFile(pathname, 'utf8').then((text) => JSON.parse(text));
}

async function assertInstalled() {
  const installRoot = path.join(project, '.pi', 'npm', 'node_modules');
  const extensionManifest = await json(path.join(installRoot, 'pi-ast-sgrep', 'package.json'));
  const launcherManifest = await json(path.join(installRoot, 'ast-sgrep', 'package.json'));
  const nativeManifest = await json(path.join(installRoot, host.packageName, 'package.json'));
  assert.equal(extensionManifest.version, expectedVersions.extension);
  assert.equal(launcherManifest.version, expectedVersions.launcher);
  assert.equal(nativeManifest.version, expectedVersions.native);
  const launcherUrl = pathToFileURL(path.join(installRoot, 'ast-sgrep', 'src', 'index.js')).href;
  const { resolveBinary } = await import(`${launcherUrl}?version=${encodeURIComponent(expectedVersions.launcher)}`);
  const binary = resolveBinary();
  const reported = runJson(binary, ['version', '--json'], { cwd: project });
  assert.equal(reported.version, expectedVersions.launcher);
  assert.equal(reported.machine_schema_version, machineSchema);
  return binary;
}

try {
  await mkdir(project, { recursive: true });
  await writeFile(path.join(project, 'source.ts'), 'export function currentArtifactNeedle() { return "current"; }\n');
  await stage('pi-install-current-artifact', async () => run(process.execPath, [piCli, 'install', source, '-l', '--approve'], { cwd: project }));
  let binary = await stage('assert-current-artifact-alignment', () => assertInstalled());
  await stage('current-artifact-index', async () => runJson(binary, ['--root', project, '--no-embed', '--json', 'index'], { cwd: project }));
  const search = await stage('current-artifact-search', async () => runJson(binary, ['--root', project, '--no-embed', '--json', 'currentArtifactNeedle'], { cwd: project }));
  assert.ok(JSON.stringify(search).includes('currentArtifactNeedle'));
  await stage('pi-update-current-artifact', async () => run(process.execPath, [piCli, 'update', '--extension', source, '--approve'], { cwd: project }));
  binary = await stage('assert-updated-current-alignment', () => assertInstalled());
  const indexPath = path.join(project, '.asgrep', 'index.db');
  const beforeRemove = await stat(indexPath);
  assert.ok(run(process.execPath, [piCli, 'list', '--approve'], { cwd: project }).includes(source));
  await stage('pi-remove-current-artifact', async () => run(process.execPath, [piCli, 'remove', source, '-l', '--approve'], { cwd: project }));
  const afterRemove = await stat(indexPath);
  assert.equal(afterRemove.size, beforeRemove.size);
  assert.ok(existsSync(path.join(project, '.asgrep')));
  assert.ok(!run(process.execPath, [piCli, 'list', '--approve'], { cwd: project }).includes(source));
  assert.ok(!existsSync(path.join(project, '.pi', 'npm', 'node_modules', 'pi-ast-sgrep')));
  console.log(JSON.stringify({ ok: true, currentArtifactLifecycle: true, versions: expectedVersions, machineSchema, host: process.platform + '-' + process.arch, install: 'Pi local npm artifact', update: 'Pi update --extension', remove: 'Pi remove -l', projectIndexPreserved: true }));
} finally {
  if (process.env.ASGREP_KEEP_E2E !== '1') await rm(temporary, { recursive: true, force: true });
  else console.error('kept current-artifact fixture at ' + temporary);
}
