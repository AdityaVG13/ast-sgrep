import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { chmod, cp, mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const oldVersion = '1.0.0-alpha';
const newVersion = '1.1.0-alpha.1';
const newNativeVersion = '1.1.0-alpha';
const oldCommit = '1f7ba20';
const machineSchema = '1.0.0';
const hosts = new Map([
  ['darwin:arm64', { directory: 'darwin-arm64', packageName: 'ast-sgrep-darwin-arm64', executable: 'asgrep' }],
  ['darwin:x64', { directory: 'darwin-x64', packageName: 'ast-sgrep-darwin-x64', executable: 'asgrep' }],
  ['linux:arm64', { directory: 'linux-arm64-gnu', packageName: 'ast-sgrep-linux-arm64-gnu', executable: 'asgrep' }],
  ['linux:x64', { directory: 'linux-x64-gnu', packageName: 'ast-sgrep-linux-x64-gnu', executable: 'asgrep' }],
  ['win32:x64', { directory: 'win32-x64-msvc', packageName: 'ast-sgrep-win32-x64-msvc', executable: 'asgrep.exe' }],
]);
const host = hosts.get(`${process.platform}:${process.arch}`);
if (!host) throw new Error(`two-version E2E has no local artifact target for ${process.platform}:${process.arch}`);

const temporary = await mkdtemp(path.join(tmpdir(), 'asgrep-pi-upgrade-'));
const project = path.join(temporary, 'project');
const agentDir = path.join(temporary, 'agent');
const artifacts = path.join(temporary, 'artifacts');
const piCli = path.join(root, 'node_modules', '@earendil-works', 'pi-coding-agent', 'dist', 'cli.js');
const currentArtifact = process.env.ASGREP_CURRENT_ARTIFACT ? path.resolve(process.env.ASGREP_CURRENT_ARTIFACT) : undefined;
const source = 'npm:pi-ast-sgrep@file:' + (currentArtifact ?? path.join(artifacts, 'extension.tgz'));
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

async function setJson(pathname, mutate) {
  const value = await json(pathname);
  mutate(value);
  await writeFile(pathname, `${JSON.stringify(value, null, 2)}\n`);
}

async function replace(pathname, from, to) {
  const before = await readFile(pathname, 'utf8');
  assert.ok(before.includes(from), `${pathname} does not contain ${from}`);
  await writeFile(pathname, before.replaceAll(from, to));
}

function pack(directory, destination) {
  const filename = run('npm', ['pack', '--ignore-scripts', '--json', '--pack-destination', path.dirname(destination), directory]);
  const parsed = JSON.parse(filename);
  assert.equal(parsed.length, 1);
  const generated = path.join(path.dirname(destination), parsed[0].filename);
  if (generated !== destination) run(process.execPath, ['-e', 'require("fs").renameSync(process.argv[1], process.argv[2])', generated, destination]);
}

async function stageArtifacts(version, binary) {
  const staging = path.join(temporary, `stage-${version}`);
  const native = path.join(staging, 'native');
  const launcher = path.join(staging, 'launcher');
  const extension = path.join(staging, 'extension');
  const nativeArtifact = path.join(artifacts, `native-${version}.tgz`);
  const launcherArtifact = path.join(artifacts, `launcher-${version}.tgz`);
  const extensionArtifact = path.join(artifacts, `extension.tgz`);
  await rm(staging, { recursive: true, force: true });
  await mkdir(staging, { recursive: true });
  await cp(path.join(root, 'packages', 'pi', 'platforms', host.directory), native, { recursive: true });
  await cp(path.join(root, 'packages', 'pi', 'launcher'), launcher, { recursive: true });
  await cp(path.join(root, 'packages', 'pi', 'extension'), extension, { recursive: true });

  const nativeBinary = path.join(native, host.executable);
  await cp(binary, nativeBinary);
  if (process.platform !== 'win32') await chmod(nativeBinary, 0o755);
  const checksum = createHash('sha256').update(await readFile(nativeBinary)).digest('hex');
  await writeFile(path.join(native, 'checksum.sha256'), `${checksum}  ${host.executable}\n`);
  await setJson(path.join(native, 'package.json'), (manifest) => {
    manifest.version = version;
    delete manifest.scripts;
  });
  pack(native, nativeArtifact);

  await setJson(path.join(launcher, 'package.json'), (manifest) => {
    manifest.version = version;
    manifest.optionalDependencies = { [host.packageName]: `file:${nativeArtifact}` };
  });
  await replace(path.join(launcher, 'src', 'index.js'), 'const VERSION = "1.1.0-alpha.1";', `const VERSION = "${version}";`);
  pack(launcher, launcherArtifact);

  await setJson(path.join(extension, 'package.json'), (manifest) => {
    manifest.version = version;
    manifest.dependencies['ast-sgrep'] = `file:${launcherArtifact}`;
    delete manifest.scripts;
  });
  if (version !== newVersion) {
    for (const relative of ['dist/runtime.js', 'dist/runtime.d.ts']) {
      await replace(path.join(extension, relative), newNativeVersion, oldVersion);
    }
  }
  pack(extension, extensionArtifact);
}

async function assertInstalled(version) {
  const installRoot = path.join(project, '.pi', 'npm', 'node_modules');
  const extensionManifest = await json(path.join(installRoot, 'pi-ast-sgrep', 'package.json'));
  const launcherManifest = await json(path.join(installRoot, 'ast-sgrep', 'package.json'));
  const nativeManifest = await json(path.join(installRoot, host.packageName, 'package.json'));
  assert.equal(extensionManifest.version, version);
  assert.equal(launcherManifest.version, version);
  assert.equal(nativeManifest.version, version);
  const launcherUrl = pathToFileURL(path.join(installRoot, 'ast-sgrep', 'src', 'index.js')).href;
  const { resolveBinary } = await import(`${launcherUrl}?version=${encodeURIComponent(version)}`);
  const binary = resolveBinary();
  if (version === newVersion) {
    const reported = runJson(binary, ['version', '--json'], { cwd: project });
    assert.equal(reported.version, newNativeVersion);
    assert.equal(reported.machine_schema_version, machineSchema);
  } else {
    const reported = run(binary, ['--version'], { cwd: project });
    assert.match(reported, /^(?:asgrep|ast-sgrep) 1\.0\.0-alpha$/);
  }
  return binary;
}

if (currentArtifact) {
  try {
    await mkdir(project, { recursive: true });
    await writeFile(path.join(project, 'source.ts'), 'export function currentArtifactNeedle() { return "current"; }\n');
    await stage('pi-install-current-artifact', async () => run(process.execPath, [piCli, 'install', source, '-l', '--approve'], { cwd: project }));
    let binary = await stage('assert-current-artifact-alignment', () => assertInstalled(newVersion));
    await stage('current-artifact-index', async () => runJson(binary, ['--root', project, '--no-embed', '--json', 'index'], { cwd: project }));
    const search = await stage('current-artifact-search', async () => runJson(binary, ['--root', project, '--no-embed', '--json', 'currentArtifactNeedle'], { cwd: project }));
    assert.ok(JSON.stringify(search).includes('currentArtifactNeedle'));
    await stage('pi-update-current-artifact', async () => run(process.execPath, [piCli, 'update', '--extension', source, '--approve'], { cwd: project }));
    binary = await stage('assert-updated-current-alignment', () => assertInstalled(newVersion));
    const indexPath = path.join(project, '.asgrep', 'index.db');
    const beforeRemove = await stat(indexPath);
    assert.ok(run(process.execPath, [piCli, 'list', '--approve'], { cwd: project }).includes(source));
    await stage('pi-remove-current-artifact', async () => run(process.execPath, [piCli, 'remove', source, '-l', '--approve'], { cwd: project }));
    const afterRemove = await stat(indexPath);
    assert.equal(afterRemove.size, beforeRemove.size);
    assert.ok(existsSync(path.join(project, '.asgrep')));
    assert.ok(!run(process.execPath, [piCli, 'list', '--approve'], { cwd: project }).includes(source));
    assert.ok(!existsSync(path.join(project, '.pi', 'npm', 'node_modules', 'pi-ast-sgrep')));
    console.log(JSON.stringify({ ok: true, currentArtifactLifecycle: true, version: newVersion, machineSchema, host: process.platform + '-' + process.arch, install: 'Pi local npm artifact', update: 'Pi update --extension', remove: 'Pi remove -l', projectIndexPreserved: true }));
  } finally {
    if (process.env.ASGREP_KEEP_E2E !== '1') await rm(temporary, { recursive: true, force: true });
    else console.error('kept current-artifact fixture at ' + temporary);
  }
  process.exit(0);
}

try {
  await mkdir(project, { recursive: true });
  await mkdir(artifacts, { recursive: true });
  await writeFile(path.join(project, 'source.ts'), 'export function versionNNeedle() { return "N"; }\n');

  const archive = path.join(temporary, 'old-source.tar');
  const oldSource = path.join(temporary, 'old-source');
  await mkdir(oldSource);
  run('git', ['archive', '--format=tar', `--output=${archive}`, oldCommit], { cwd: root });
  run('tar', ['-xf', archive, '-C', oldSource]);
  await stage('build-old-binary', async () => run('cargo', ['build', '--offline', '--locked', '-p', 'ast-sgrep-cli', '--bin', 'asgrep', '--target-dir', path.join(temporary, 'old-target')], { cwd: oldSource, timeout: 1_200_000 }));
  const oldBinary = path.join(temporary, 'old-target', 'debug', host.executable);
  const currentBinary = path.join(root, 'target', 'debug', host.executable);
  assert.ok(existsSync(currentBinary), `current binary is missing: ${currentBinary}`);

  await stage('pack-old-artifacts', () => stageArtifacts(oldVersion, oldBinary));
  await stage('pi-install-old', async () => run(process.execPath, [piCli, 'install', source, '-l', '--approve'], { cwd: project }));
  let binary = await stage('assert-old-alignment', () => assertInstalled(oldVersion));
  await stage('old-index', async () => runJson(binary, ['--root', project, '--no-embed', '--json', 'index'], { cwd: project }, false));
  const oldSearch = await stage('old-search', async () => runJson(binary, ['--root', project, '--no-embed', '--json', 'versionNNeedle'], { cwd: project }, false));
  assert.ok(JSON.stringify(oldSearch).includes('versionNNeedle'));

  await writeFile(path.join(project, 'source.ts'), 'export function versionNPlusOneNeedle() { return "N+1"; }\n');
  await stage('pack-current-artifacts', () => stageArtifacts(newVersion, currentBinary));
  await stage('pi-update-current', async () => run(process.execPath, [piCli, 'update', '--extension', source, '--approve'], { cwd: project }));
  binary = await stage('assert-current-alignment', () => assertInstalled(newVersion));
  await stage('current-index', async () => runJson(binary, ['--root', project, '--no-embed', '--json', 'index'], { cwd: project }));
  const currentSearch = await stage('current-search', async () => runJson(binary, ['--root', project, '--no-embed', '--json', 'versionNPlusOneNeedle'], { cwd: project }));
  assert.ok(JSON.stringify(currentSearch).includes('versionNPlusOneNeedle'));

  const indexPath = path.join(project, '.asgrep', 'index.db');
  const beforeRemove = await stat(indexPath);
  const listed = run(process.execPath, [piCli, 'list', '--approve'], { cwd: project });
  assert.ok(listed.includes(source));
  await stage('pi-remove', async () => run(process.execPath, [piCli, 'remove', source, '-l', '--approve'], { cwd: project }));
  const afterRemove = await stat(indexPath);
  assert.equal(afterRemove.size, beforeRemove.size);
  assert.ok(existsSync(path.join(project, '.asgrep')));
  const afterList = run(process.execPath, [piCli, 'list', '--approve'], { cwd: project });
  assert.ok(!afterList.includes(source));
  assert.ok(!existsSync(path.join(project, '.pi', 'npm', 'node_modules', 'pi-ast-sgrep')));
  console.error('[stage:preserve-project-index] PASS');

  console.log(JSON.stringify({
    ok: true,
    versions: [oldVersion, newVersion],
    machineSchema,
    host: `${process.platform}-${process.arch}`,
    install: 'Pi local npm artifact',
    update: 'Pi update --extension',
    remove: 'Pi remove -l',
    projectIndexPreserved: true,
  }));
} finally {
  if (process.env.ASGREP_KEEP_E2E !== '1') await rm(temporary, { recursive: true, force: true });
  else console.error(`kept two-version fixture at ${temporary}`);
}
