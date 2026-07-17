import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, renameSync } from 'node:fs';
import { chmod, cp, mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const version = '1.1.0-alpha.1';
const machineSchema = '1.0.0';
const piVersion = '0.80.6';
const maxCapturedBytes = 4 * 1024 * 1024;
const hosts = new Map([
  ['darwin:arm64', { directory: 'darwin-arm64', packageName: 'ast-sgrep-darwin-arm64', executable: 'asgrep' }],
  ['darwin:x64', { directory: 'darwin-x64', packageName: 'ast-sgrep-darwin-x64', executable: 'asgrep' }],
  ['linux:arm64', { directory: 'linux-arm64-gnu', packageName: 'ast-sgrep-linux-arm64-gnu', executable: 'asgrep' }],
  ['linux:x64', { directory: 'linux-x64-gnu', packageName: 'ast-sgrep-linux-x64-gnu', executable: 'asgrep' }],
  ['win32:x64', { directory: 'win32-x64-msvc', packageName: 'ast-sgrep-win32-x64-msvc', executable: 'asgrep.exe' }],
]);
const host = hosts.get(process.platform + ':' + process.arch);
if (!host) throw new Error(process.platform + ':' + process.arch + ' is not a packaged ast-sgrep target');
const [nodeMajor, nodeMinor] = process.versions.node.split('.').map(Number);
if (nodeMajor < 22 || (nodeMajor === 22 && nodeMinor < 19)) throw new Error('Node 22.19.0 or newer is required');

const temporary = await mkdtemp(path.join(tmpdir(), 'asgrep-pi-release-gate-'));
const project = path.join(temporary, 'project');
const home = path.join(temporary, 'home');
const agentDir = path.join(home, '.pi-agent');
const artifacts = path.join(temporary, 'artifacts');
const staging = path.join(temporary, 'staging');
const emptyPath = path.join(temporary, 'empty-path');
const piRoot = path.join(root, 'node_modules', '@earendil-works', 'pi-coding-agent');
const piCli = path.join(piRoot, 'dist', 'cli.js');
const nativeSource = path.join(root, 'target', 'debug', host.executable);
const children = new Set();
const stages = [];
const inheritedEnvironment = { ...process.env };

function cleanEnvironment() {
  const env = { ...process.env, HOME: home, PI_CODING_AGENT_DIR: agentDir, PI_OFFLINE: '1', npm_config_offline: 'true', npm_config_audit: 'false', npm_config_fund: 'false', npm_config_cache: path.join(home, '.npm'), NO_COLOR: '1' };
  for (const key of Object.keys(env)) if (/(?:^ASGREP_|API_KEY|ACCESS_TOKEN|AUTH_TOKEN|OAUTH_TOKEN|MCP)/u.test(key)) delete env[key];
  return env;
}
const commandEnv = cleanEnvironment();
function bounded(text, bytes = 8192) {
  return Buffer.byteLength(text) <= bytes ? text : Buffer.from(text).subarray(0, bytes).toString('utf8') + '\n…<truncated>';
}
function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: options.cwd ?? root, env: { ...commandEnv, ...options.env }, encoding: 'utf8', timeout: options.timeout ?? 300_000, maxBuffer: maxCapturedBytes, windowsHide: true });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(command + ' ' + args.join(' ') + ' failed (' + result.status + ')\nstdout:\n' + bounded(result.stdout ?? '') + '\nstderr:\n' + bounded(result.stderr ?? ''));
  return (result.stdout ?? '').trim();
}
const stage = async (name, action) => {
  const started = Date.now();
  console.error('[stage:' + name + '] START');
  try {
    const value = await action();
    const durationMs = Date.now() - started;
    stages.push({ name, durationMs });
    console.error('[stage:' + name + '] PASS ' + durationMs + 'ms');
    return value;
  } catch (cause) {
    console.error('[stage:' + name + '] FAIL ' + (Date.now() - started) + 'ms: ' + bounded(cause instanceof Error ? cause.stack ?? cause.message : String(cause)));
    throw cause;
  }
}
const json = async (pathname) => JSON.parse(await readFile(pathname, 'utf8'));
const setJson = async (pathname, mutate) => {
  const value = await json(pathname);
  mutate(value);
  await writeFile(pathname, JSON.stringify(value, null, 2) + '\n');
}
function pack(directory, destination) {
  const output = JSON.parse(run('npm', ['pack', '--ignore-scripts', '--json', '--pack-destination', path.dirname(destination), directory]));
  assert.equal(output.length, 1);
  const generated = path.join(path.dirname(destination), output[0].filename);
  if (generated !== destination) renameSync(generated, destination);
}
const packArtifacts = async () => {
  assert.ok(existsSync(nativeSource), 'current native binary is missing: ' + nativeSource);
  assert.ok((await stat(nativeSource)).size > 0, 'current native binary is empty: ' + nativeSource);
  const native = path.join(staging, 'native');
  const launcher = path.join(staging, 'launcher');
  const extension = path.join(staging, 'extension');
  await cp(path.join(root, 'packages/pi/platforms', host.directory), native, { recursive: true });
  await cp(path.join(root, 'packages/pi/launcher'), launcher, { recursive: true });
  await cp(path.join(root, 'packages/pi/extension'), extension, { recursive: true });
  await cp(nativeSource, path.join(native, host.executable));
  if (process.platform !== 'win32') await chmod(path.join(native, host.executable), 0o755);
  const checksum = createHash('sha256').update(await readFile(path.join(native, host.executable))).digest('hex');
  await writeFile(path.join(native, 'checksum.sha256'), checksum + '  ' + host.executable + '\n');
  await setJson(path.join(native, 'package.json'), (manifest) => { delete manifest.scripts; });
  const nativeTar = path.join(artifacts, 'native.tgz');
  pack(native, nativeTar);
  const launcherTar = path.join(artifacts, 'launcher.tgz');
  await setJson(path.join(launcher, 'package.json'), (manifest) => { manifest.optionalDependencies = { [host.packageName]: 'file:' + nativeTar }; });
  pack(launcher, launcherTar);
  const typeboxRoot = path.join(root, 'node_modules', 'typebox');
  assert.ok(existsSync(path.join(typeboxRoot, 'package.json')), 'local typebox dependency is unavailable');
  const typeboxTar = path.join(artifacts, 'typebox.tgz');
  pack(typeboxRoot, typeboxTar);
  const extensionTar = path.join(artifacts, 'extension.tgz');
  await setJson(path.join(extension, 'package.json'), (manifest) => {
    manifest.dependencies['ast-sgrep'] = 'file:' + launcherTar;
    manifest.dependencies.typebox = 'file:' + typeboxTar;
    delete manifest.scripts;
  });
  pack(extension, extensionTar);
  return extensionTar;
}
function execAction(command, args, options) {
  return new Promise((resolve, reject) => {
    const childEnv = { ...options.env, PATH: emptyPath };
    for (const key of Object.keys(childEnv)) if (/(?:API_KEY|ACCESS_TOKEN|AUTH_TOKEN|OAUTH_TOKEN|MCP|ASGREP_BIN)/u.test(key)) delete childEnv[key];
    const child = spawn(command, [...args], { cwd: options.cwd, env: childEnv, signal: options.signal, windowsHide: true });
    children.add(child);
    let stdout = '';
    let stderr = '';
    let bytes = 0;
    let settled = false;
    const finish = (fn, value) => { if (!settled) { settled = true; children.delete(child); fn(value); } };
    const append = (which, chunk) => {
      bytes += chunk.length;
      if (bytes > maxCapturedBytes) {
        child.kill('SIGKILL');
        finish(reject, new Error('extension subprocess output exceeded ' + maxCapturedBytes + ' bytes'));
      } else if (which === 'stdout') stdout += chunk.toString('utf8');
      else stderr += chunk.toString('utf8');
    };
    child.stdout.on('data', (chunk) => append('stdout', chunk));
    child.stderr.on('data', (chunk) => append('stderr', chunk));
    child.once('error', (error) => finish(reject, error));
    child.once('close', (exitCode, signal) => finish(resolve, { stdout, stderr, exitCode, signal }));
  });
}
function envelope(result, command) {
  assert.equal(result.details.ok, true, JSON.stringify(result.details));
  const response = result.details.response;
  assert.equal(response.tool, 'asgrep');
  assert.equal(response.schema_version, machineSchema);
  assert.equal(response.ok, true);
  if (command) assert.equal(response.command, command);
  assert.ok(result.content[0].text.length <= 1200, 'tool summary exceeded 1200 characters');
  return response;
}
function assertHit(response, needle) {
  assert.ok(JSON.stringify(response).includes(needle), 'expected response to include ' + needle + ': ' + bounded(JSON.stringify(response), 4096));
}

let primaryFailure;
try {
  await Promise.all([mkdir(project, { recursive: true }), mkdir(home, { recursive: true }), mkdir(artifacts, { recursive: true }), mkdir(staging, { recursive: true }), mkdir(emptyPath, { recursive: true })]);
  await writeFile(path.join(project, 'app.ts'), 'export function initialNeedle(name: string) { return "hello " + name; }\nexport function initialCaller() { return initialNeedle("Pi"); }\n');
  await writeFile(path.join(project, 'worker.ts'), 'import { initialCaller } from "./app";\nexport const initialResult = initialCaller();\n');
  await writeFile(path.join(project, 'calls.rs'), 'pub fn rust_needle() -> i32 { 1 }\npub fn rust_caller() -> i32 { rust_needle() }\n');
  await writeFile(path.join(project, 'pattern.ts'), 'export function fetchNeedle(client: { fetch(url: string): Promise<string> }, url: string) { return await client.fetch(url); }\n');
  await stage('extension-build', async () => run('npm', ['run', 'build', '--workspace', 'pi-ast-sgrep']));
  const extensionTar = await stage('pack-local-artifacts', packArtifacts);
  const source = 'npm:pi-ast-sgrep@file:' + extensionTar;
  await stage('pi-install-packed-extension', async () => run(process.execPath, [piCli, 'install', source, '-l', '--approve'], { cwd: project }));
  const installRoot = path.join(project, '.pi', 'npm', 'node_modules');
  const extensionRoot = path.join(installRoot, 'pi-ast-sgrep');
  await stage('installed-version-alignment', async () => {
    assert.equal((await json(path.join(extensionRoot, 'package.json'))).version, version);
    assert.equal((await json(path.join(installRoot, 'ast-sgrep', 'package.json'))).version, version);
    assert.equal((await json(path.join(installRoot, host.packageName, 'package.json'))).version, version);
    assert.equal((await json(path.join(piRoot, 'package.json'))).version, piVersion);
    assert.match((await json(path.join(extensionRoot, 'package.json'))).peerDependencies['@earendil-works/pi-coding-agent'], /^>=0\.80\.6 <1$/u);
  });
  await stage('parent-environment-isolation', async () => {
    for (const key of Object.keys(process.env)) if (/(?:^ASGREP_|API_KEY|ACCESS_TOKEN|AUTH_TOKEN|OAUTH_TOKEN|MCP)/u.test(key)) delete process.env[key];
    process.env.ASGREP_REFRESH_INTERVAL_MS = '50';
    assert.ok(!Object.keys(process.env).some((key) => /(?:^ASGREP_(?!REFRESH_INTERVAL_MS$)|API_KEY|ACCESS_TOKEN|AUTH_TOKEN|OAUTH_TOKEN|MCP)/u.test(key)), 'sensitive or test-control parent environment reached the extension loader');
  });
  const pi = await import(pathToFileURL(path.join(piRoot, 'dist', 'index.js')).href);
  const loader = await import(pathToFileURL(path.join(piRoot, 'dist', 'core', 'extensions', 'loader.js')).href);
  process.env.ASGREP_REFRESH_INTERVAL_MS = '50';
  const runtime = pi.createExtensionRuntime();
  runtime.exec = execAction;
  const loaded = await stage('real-pi-loader-extension-api', async () => loader.loadExtensions([path.join(extensionRoot, 'dist', 'index.js')], project, pi.createEventBus(), runtime));
  assert.deepEqual(loaded.errors, []);
  assert.equal(loaded.extensions.length, 1);
  const runner = new pi.ExtensionRunner(loaded.extensions, runtime, project, {}, {});
  const toolNames = runner.getAllRegisteredTools().map(({ definition }) => definition.name).sort();
  const commandNames = runner.getRegisteredCommands().map(({ invocationName }) => invocationName).sort();
  assert.deepEqual(toolNames, ['asgrep_index', 'asgrep_search', 'asgrep_status']);
  assert.deepEqual(commandNames, ['asgrep-doctor', 'asgrep-index', 'asgrep-reindex', 'asgrep-status']);
  const context = runner.createContext();
  const searchTool = runner.getToolDefinition('asgrep_search');
  const indexTool = runner.getToolDefinition('asgrep_index');
  const statusTool = runner.getToolDefinition('asgrep_status');
  assert.ok(searchTool && indexTool && statusTool);
  const invokeSearch = (params, signal = undefined) => searchTool.execute('release-gate', params, signal, undefined, context);
  await stage('skill-discovery-workflow', async () => {
    const skills = pi.loadSkillsFromDir({ dir: path.join(extensionRoot, 'skills'), source: 'release-gate' });
    assert.deepEqual(skills.diagnostics, []);
    assert.deepEqual(skills.skills.map((skill) => skill.name), ['ast-sgrep']);
    assert.match(pi.formatSkillsForPrompt(skills.skills), /ast-sgrep/u);
    const text = await readFile(skills.skills[0].filePath, 'utf8');
    for (const instruction of ['exact-text search', 'natural', 'defs', 'callers', '/asgrep-doctor', '/asgrep-index']) assert.ok(text.includes(instruction), instruction);
  });
  const lazy = await stage('lazy-index-natural-search', async () => invokeSearch({ query: 'initialNeedle', mode: 'natural', limit: 8 }));
  assert.ok(existsSync(path.join(project, '.asgrep', 'index.db')), 'lazy search did not create the project index');
  assertHit(envelope(lazy), 'initialNeedle');
  await stage('pattern-defs-callers-semantic', async () => {
    assertHit(envelope(await invokeSearch({ query: '$CLIENT.fetch($$$ARGS)', mode: 'pattern', limit: 8 })), 'pattern.ts');
    assertHit(envelope(await invokeSearch({ query: 'initialNeedle', mode: 'defs', limit: 8 })), 'initialNeedle');
    assertHit(envelope(await invokeSearch({ query: 'rust_needle', mode: 'callers', limit: 8 })), 'rust_caller');
    assertHit(envelope(await invokeSearch({ query: 'function that greets a person', mode: 'semantic', limit: 8 })), 'app.ts');
  });
  await stage('create-modify-delete-freshness', async () => {
    const dynamic = path.join(project, 'dynamic.ts');
    await writeFile(dynamic, 'export function createdNeedle() { return 1; }\n');
    await runner.emitToolResult({ type: 'tool_result', toolCallId: 'write-1', toolName: 'write', input: { path: 'dynamic.ts', content: '' }, content: [], details: undefined, isError: false });
    assertHit(envelope(await invokeSearch({ query: 'createdNeedle', mode: 'defs', limit: 8 })), 'dynamic.ts');
    await writeFile(dynamic, 'export function modifiedNeedle() { return 2; }\n');
    await runner.emitToolResult({ type: 'tool_result', toolCallId: 'edit-1', toolName: 'edit', input: { path: 'dynamic.ts', oldText: '', newText: '' }, content: [], details: undefined, isError: false });
    assertHit(envelope(await invokeSearch({ query: 'modifiedNeedle', mode: 'defs', limit: 8 })), 'modifiedNeedle');
    await rm(dynamic);
    await new Promise((resolve) => setTimeout(resolve, 80));
    assert.ok(!JSON.stringify(envelope(await invokeSearch({ query: 'modifiedNeedle', mode: 'defs', limit: 8 }))).includes('dynamic.ts'), 'deleted file remained searchable');
  });
  await stage('tools-commands-doctor-status-index-reindex', async () => {
    envelope(await statusTool.execute('status', {}, undefined, undefined, context), 'status');
    envelope(await indexTool.execute('index', { force: false }, undefined, undefined, context), 'index');
    envelope(await indexTool.execute('reindex', { force: true }, undefined, undefined, context), 'reindex');
    const notices = [];
    const commandContext = runner.createCommandContext();
    commandContext.ui.notify = (message, type) => notices.push({ message, type });
    for (const name of ['asgrep-doctor', 'asgrep-status', 'asgrep-index', 'asgrep-reindex']) await runner.getCommand(name).handler('', commandContext);
    assert.equal(notices.length, 4);
    for (const notice of notices) {
      assert.equal(notice.type, 'info', notice.message);
      assert.ok(notice.message.length <= 1200);
      const parsed = JSON.parse(notice.message);
      assert.equal(parsed.ok, true);
      assert.equal(parsed.response.tool, 'asgrep');
    }
  });
  await stage('cancellation-and-index-recovery', async () => {
    const controller = new AbortController();
    controller.abort();
    const cancelled = await indexTool.execute('cancelled', { force: true }, controller.signal, undefined, context);
    assert.equal(cancelled.details.ok, false);
    assert.equal(cancelled.details.error.code, 'CANCELLED');
    const indexPath = path.join(project, '.asgrep', 'index.db');
    await writeFile(indexPath, 'incompatible-index');
    await new Promise((resolve) => setTimeout(resolve, 80));
    assertHit(envelope(await invokeSearch({ query: 'initialNeedle', mode: 'defs', limit: 8 })), 'initialNeedle');
    assert.ok((await stat(indexPath)).size > 'incompatible-index'.length);
    assert.ok(!(await readdir(path.dirname(indexPath))).some((name) => name.startsWith('.rebuild-') || name.includes('.backup-')));
  });
  await stage('two-version-lifecycle-reuse', async () => {
    const output = run(process.execPath, [path.join(root, 'packages/pi/scripts/two-version-e2e.mjs')], { cwd: root, timeout: 600_000, env: { ASGREP_CURRENT_ARTIFACT: extensionTar } });
    const value = JSON.parse(output.split(/\r?\n/u).at(-1));
    assert.equal(value.ok, true);
    assert.equal(value.currentArtifactLifecycle, true);
    assert.equal(value.projectIndexPreserved, true);
  });
  assert.equal(children.size, 0, 'extension subprocesses are still running');
  console.log(JSON.stringify({ ok: true, release: version, machineSchema, node: process.version, pi: piVersion, host: process.platform + '-' + process.arch, loader: 'Pi loadExtensions + ExtensionAPI + ExtensionRunner', packedArtifacts: ['native.tgz', 'launcher.tgz', 'typebox.tgz', 'extension.tgz'], tools: toolNames, commands: commandNames, stages, criteria: { packedArtifacts: true, parentEnvironmentIsolation: true, realPiLoader: true, toolsAndCommands: true, skillDiscovery: true, lazyIndex: true, naturalPatternDefsCallersSemantic: true, createModifyDeleteFreshness: true, cancellation: true, doctorStatusIndexReindex: true, versionAlignment: true, incompatibleIndexRecovery: true, updateRemovalViaTwoVersionHarness: true, projectIndexPreservedOnRemoval: true, boundedOutput: true, isolatedHomeProject: true, noCredentialsAdaptersPathOrMcp: true, cleanup: true } }));
} catch (cause) {
  primaryFailure = cause;
  throw cause;
} finally {
  for (const child of children) child.kill('SIGKILL');
  let restorationFailure;
  try {
    for (const key of Object.keys(process.env)) if (!(key in inheritedEnvironment)) delete process.env[key];
    Object.assign(process.env, inheritedEnvironment);
    assert.deepEqual({ ...process.env }, inheritedEnvironment, 'parent environment was not restored exactly');
  } catch (cause) {
    restorationFailure = cause;
    if (primaryFailure) console.error('[release-gate] environment restoration also failed: ' + String(cause));
  }
  await rm(temporary, { recursive: true, force: true });
  if (restorationFailure && !primaryFailure) throw restorationFailure;
}
