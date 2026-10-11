import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmod, copyFile, cp, mkdir, mkdtemp, readFile, realpath, rename, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const matrix = JSON.parse(await readFile(path.join(repo, 'packages/pi/release/targets.json'), 'utf8'));
const target = matrix.targets.find(entry => entry.os === process.platform && entry.cpu === process.arch);
const run = (command, args, cwd, env = {}) => {
  // Windows cannot spawn npm.cmd directly without a shell. Invoke the bundled
  // npm CLI through Node instead, keeping arguments out of shell interpolation.
  if (command === 'npm' && process.platform === 'win32') {
    args = [process.env.npm_execpath ?? path.join(path.dirname(process.execPath), 'node_modules/npm/bin/npm-cli.js'), ...args];
    command = process.execPath;
  }
  const result = spawnSync(command, args, { cwd, env: { ...process.env, ...env }, encoding: 'utf8', timeout: 30_000, windowsHide: true });
  assert.equal(result.status, 0, `${command} ${args.join(' ')}: ${result.error ?? result.stderr}`);
  return result.stdout;
};

// Requires real, target-local CLI/watch/NAPI artifacts; never substitute scripts
// for native watch behavior. Native artifact workflows supply this directory.
test('a clean packed launcher watches and indexes a new file without PATH or auto-index fallback', { timeout: 60_000 }, async t => {
  assert.ok(target, 'this host is not in the supported native target matrix');
  assert.ok(process.env.ASGREP_TEST_NATIVE_DIR, 'set ASGREP_TEST_NATIVE_DIR to a directory containing the real CLI, watch helper, and NAPI addon');
  const source = path.resolve(process.env.ASGREP_TEST_NATIVE_DIR);
  const root = await realpath(await mkdtemp(path.join(tmpdir(), 'asgrep-npm-watch-')));
  const native = path.join(root, 'packages/pi/platforms', target.id);
  const launcher = path.join(root, 'packages/pi/launcher');
  const packs = path.join(root, 'packs');
  const install = path.join(root, 'install');
  const fixture = path.join(root, 'fixture');
  for (const directory of [native, packs, install, fixture, path.join(root, 'packages/pi/release')]) await mkdir(directory, { recursive: true });
  // A real checkout boundary prevents an enclosing developer index from owning
  // this cold fixture; watch must still create and update its default index.
  run('git', ['init', '--quiet', fixture], root);
  for (const file of ['packages/pi/release/targets.json', 'packages/pi/release-contract.json', 'packages/pi/platforms/prepack-verify.mjs']) await copyFile(path.join(repo, file), path.join(root, file));
  for (const file of ['package.json', 'LICENSE']) await copyFile(path.join(repo, 'packages/pi/platforms', target.id, file), path.join(native, file));
  const checksums = [];
  for (const name of [target.executable, target.watchHelper, matrix.napiAddon]) {
    const bytes = await readFile(path.join(source, name));
    assert.ok(bytes.length > 0, name + ' is empty');
    await writeFile(path.join(native, name), bytes);
    if (target.os !== 'win32' && name !== matrix.napiAddon) await chmod(path.join(native, name), 0o755);
    checksums.push(createHash('sha256').update(bytes).digest('hex') + '  ' + name);
  }
  await writeFile(path.join(native, 'checksum.sha256'), checksums.sort().join('\n') + '\n');
  const npmEnv = { npm_config_offline: 'true', npm_config_cache: path.join(root, 'cache'), npm_config_user_agent: 'OpenAI File Downloader, XaiImageApiFetch/1.0' };
  const pack = directory => {
    const packed = JSON.parse(run('npm', ['pack', directory, '--json', '--pack-destination', packs], root, npmEnv));
    const entries = Array.isArray(packed) ? packed : Object.values(packed);
    assert.equal(entries.length, 1, 'npm must return exactly one packed artifact');
    return entries[0];
  };
  const nativePack = pack(native);
  assert.ok(nativePack.files.some(file => file.path === target.watchHelper), 'the packed native tarball must contain the helper');
  await cp(path.join(repo, 'packages/pi/launcher'), launcher, { recursive: true });
  const manifest = JSON.parse(await readFile(path.join(launcher, 'package.json'), 'utf8'));
  manifest.optionalDependencies = { [target.package]: 'file:' + path.join(packs, nativePack.filename) };
  await writeFile(path.join(launcher, 'package.json'), JSON.stringify(manifest));
  const launcherPack = pack(launcher);
  await writeFile(path.join(install, 'package.json'), JSON.stringify({ name: 'watch-packed-install', private: true }));
  run('npm', ['install', '--no-audit', '--no-fund', path.join(packs, launcherPack.filename)], install, npmEnv);
  const { resolveBinary } = await import(pathToFileURL(path.join(install, 'node_modules/ast-sgrep/src/index.js')).href);
  const binary = resolveBinary({ env: {} });
  assert.ok(binary.startsWith(path.join(install, 'node_modules') + path.sep), 'must resolve the installed native package, not a developer override');
  const helper = path.join(path.dirname(binary), target.watchHelper);
  const env = { ...process.env, PATH: '', NO_COLOR: '1', ASGREP_DISABLE_AUTO_INDEX: '1' };
  delete env.ASGREP_WATCH_BIN;
  delete env.AST_SGREP_BINARY;
  const query = () => JSON.parse(run(binary, ['--root', fixture, '--no-embed', '--no-auto-index', '--json', '--format', 'native', 'defs: npmPackedWatchMarker'], install, env));
  await writeFile(path.join(fixture, 'base.ts'), 'export function baseline() { return 1; }\n');
  await rename(helper, helper + '.held');
  const missing = spawnSync(binary, ['--root', fixture, '--no-embed', 'watch'], { cwd: install, env, encoding: 'utf8', timeout: 5000 });
  await rename(helper + '.held', helper);
  assert.notEqual(missing.status, 0, 'watch must fail when neither its sibling nor a PATH fallback exists');
  assert.match(missing.stderr ?? '', /asgrep-watch helper not found/u);
  let diagnostics = '', failure, evidence;
  const transcript = path.join(root, 'watch-evidence.json');
  const child = spawn(binary, ['--root', fixture, '--no-embed', 'watch', '--debounce-ms', '50'], { cwd: install, env, windowsHide: true });
  const exited = new Promise(resolve => {
    child.once('error', error => { failure = error; resolve(); });
    child.once('close', (code, signal) => { failure ??= new Error(`watch exited (${code}, ${signal}): ${diagnostics}`); resolve(); });
  });
  child.stdout.on('data', chunk => { diagnostics += chunk; });
  child.stderr.on('data', chunk => { diagnostics += chunk; });
  t.after(async () => {
    if (child.pid && child.exitCode === null && child.signalCode === null) {
      // Windows CLI waits for its helper rather than exec'ing it. Terminate
      // only this test-owned process tree so the helper cannot outlive the test.
      if (process.platform === 'win32') run('taskkill', ['/PID', String(child.pid), '/T', '/F'], install);
      else child.kill();
    }
    await Promise.race([exited, delay(3000, undefined, { ref: false })]);
    if (child.exitCode === null && child.signalCode === null) { child.kill('SIGKILL'); await exited; }
    if (evidence) {
      await writeFile(transcript, JSON.stringify({ ...evidence, watcherStopped: true }, null, 2) + '\n');
      console.log('retained packed watch evidence: ' + transcript);
    }
  });
  const until = async predicate => {
    const deadline = performance.now() + 15_000;
    while (performance.now() < deadline) {
      if (failure) throw failure;
      if (await predicate()) return;
      await delay(100);
    }
    assert.fail('watch did not become observable within 15s: ' + diagnostics);
  };
  await until(() => {
    const result = spawnSync(binary, ['--root', fixture, '--no-embed', '--json', 'status'], { cwd: install, env, encoding: 'utf8', timeout: 5000 });
    if (result.error) throw result.error;
    const status = JSON.parse(result.stdout);
    if (result.status !== 0) {
      // A read-only status before the first watch commit correctly reports an
      // empty index. Retry readiness, never issue an index/search refresh.
      assert.equal(result.status, 2, result.stdout + result.stderr);
      assert.equal(status.ok, false);
      assert.equal(status.error?.kind, 'operational');
      diagnostics += '\nreadiness: ' + status.error.message;
      return false;
    }
    return status.ok === true && status.file_count === 1 && status.symbol_count >= 1;
  });
  assert.equal(query().hits?.length, 0, 'marker must not already be indexed');
  await writeFile(path.join(fixture, 'changed.ts'), 'export function npmPackedWatchMarker() { return 42; }\n');
  let observed;
  await until(() => {
    observed = query();
    return observed.ok === true && observed.hits?.some(hit => hit.file === 'changed.ts' && hit.symbol === 'npmPackedWatchMarker');
  });
  evidence = { target: target.id, nativeTarball: path.join(packs, nativePack.filename), launcherTarball: path.join(packs, launcherPack.filename), missingHelperRejected: true, pathFallbackDisabled: true, autoIndexDisabled: true, watcherPid: child.pid, result: observed };
});
