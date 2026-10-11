import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFile, mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const script = path.join(repo, 'packages/pi/scripts/repin-lock.mjs');
const matrix = JSON.parse(await readFile(path.join(repo, 'packages/pi/release/targets.json'), 'utf8'));
const version = JSON.parse(await readFile(path.join(repo, 'package.json'), 'utf8')).version;
const userAgent = 'OpenAI File Downloader, XaiImageApiFetch/1.0';
function run(command, args, cwd, env = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd, env: { ...process.env, ...env }, windowsHide: true });
    let stdout = '', stderr = '';
    child.stdout.on('data', chunk => { stdout += chunk; });
    child.stderr.on('data', chunk => { stderr += chunk; });
    child.once('error', reject);
    child.once('close', code => resolve({ code, stdout, stderr }));
  });
}

// A local npm registry with real tarballs exercises npm's optional dependency
// handling and cache, rather than substituting a successful npm process.
async function fixture(t, { missing = 0, corrupt = false, hang = false, lockMismatch = false } = {}) {
  const root = await mkdtemp(path.join(tmpdir(), 'asgrep-repin-test-'));
  const packs = new Map();
  for (const target of matrix.targets) {
    const directory = path.join(root, 'tarballs', target.id, 'package');
    await mkdir(directory, { recursive: true });
    await writeFile(path.join(directory, 'package.json'), JSON.stringify({ name: target.package, version, os: [target.os], cpu: [target.cpu] }));
    const tarball = path.join(root, 'tarballs', target.id + '.tgz');
    const packed = await run('tar', ['-czf', tarball, '-C', path.dirname(directory), 'package'], root);
    assert.equal(packed.code, 0, packed.stderr);
    const bytes = await readFile(tarball);
    packs.set(target.package, { target, bytes, integrity: 'sha512-' + createHash('sha512').update(bytes).digest('base64') });
  }
  const requests = [];
  const server = createServer((request, response) => {
    const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    requests.push({ path: pathname, userAgent: request.headers['user-agent'], command: request.headers['npm-command'], accept: request.headers.accept });
    response.setHeader('Cache-Control', 'no-store');
    const pack = [...packs.values()].find(({ target }) => pathname === '/' + target.package || pathname === '/tarballs/' + target.id + '.tgz');
    if (!pack) { response.writeHead(404).end('{}'); return; }
    if (pathname.startsWith('/tarballs/')) { response.end(pack.bytes); return; }
    if (hang) return;
    if (pack.target.id === matrix.targets[0].id && missing-- > 0) { response.writeHead(404).end('{"error":"propagating"}'); return; }
    const dist = { tarball: `http://127.0.0.1:${server.address().port}/tarballs/${pack.target.id}.tgz`, integrity: pack.integrity };
    if (corrupt && pack.target.id === matrix.targets.at(-1).id) delete dist.integrity;
    if (lockMismatch && request.headers['npm-command'] === 'install' && pack.target.id === matrix.targets.at(-1).id) dist.integrity = 'sha512-' + Buffer.alloc(64).toString('base64');
    response.setHeader('Content-Type', 'application/json');
    response.end(JSON.stringify({ name: pack.target.package, 'dist-tags': { latest: version }, versions: { [version]: { name: pack.target.package, version, os: [pack.target.os], cpu: [pack.target.cpu], dist } } }));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => { server.closeAllConnections(); server.close(); });
  const workspaces = ['packages/pi/extension', 'packages/pi/launcher'];
  for (const workspace of workspaces) await mkdir(path.join(root, workspace), { recursive: true });
  await mkdir(path.join(root, 'packages/pi/release'), { recursive: true });
  for (const file of ['packages/pi/release-contract.json', 'packages/pi/release/targets.json']) await copyFile(path.join(repo, file), path.join(root, file));
  const optionalDependencies = Object.fromEntries(matrix.targets.map(target => [target.package, version]));
  await writeFile(path.join(root, 'package.json'), JSON.stringify({ name: 'repin-fixture', version, private: true, workspaces }));
  await writeFile(path.join(root, workspaces[0], 'package.json'), JSON.stringify({ name: 'pi-ast-sgrep', version, private: true, dependencies: { 'ast-sgrep': version } }));
  await writeFile(path.join(root, workspaces[1], 'package.json'), JSON.stringify({ name: 'ast-sgrep', version, optionalDependencies }));
  const before = JSON.stringify({ name: 'repin-fixture', version, lockfileVersion: 3, requires: true, packages: {
    '': { name: 'repin-fixture', version, workspaces },
    [workspaces[0]]: { name: 'pi-ast-sgrep', version, dependencies: { 'ast-sgrep': version } },
    [workspaces[1]]: { name: 'ast-sgrep', version, optionalDependencies },
    'node_modules/pi-ast-sgrep': { resolved: workspaces[0], link: true },
    'node_modules/ast-sgrep': { resolved: workspaces[1], link: true },
    ...Object.fromEntries(matrix.targets.map(target => ['node_modules/' + target.package, { optional: true }]))
  } }, null, 2) + '\n';
  await writeFile(path.join(root, 'package-lock.json'), before);
  const registry = `http://127.0.0.1:${server.address().port}/`;
  await writeFile(path.join(root, '.npmrc'), `registry=${registry}\nuser-agent=${userAgent}\n`);
  const env = { npm_config_registry: registry, npm_config_user_agent: userAgent, npm_config_cache: path.join(root, 'cache'), npm_config_userconfig: path.join(root, '.npmrc') };
  console.log('retained npm re-pin fixture: ' + root);
  return { root, before, packs, requests, env };
}

test('re-pin retries delayed metadata, repairs every native stub, and supports a real npm ci', { timeout: 40_000 }, async t => {
  const f = await fixture(t, { missing: 2 });
  const result = await run(process.execPath, [script, '--version', version, '--timeout-ms', '20000', '--poll-ms', '50'], f.root, f.env);
  assert.equal(result.code, 0, result.stderr);
  const lock = JSON.parse(await readFile(path.join(f.root, 'package-lock.json'), 'utf8'));
  for (const [name, pack] of f.packs) {
    assert.equal(lock.packages['node_modules/' + name]?.version, version, name);
    assert.equal(lock.packages['node_modules/' + name]?.integrity, pack.integrity, name);
    assert.match(lock.packages['node_modules/' + name]?.resolved ?? '', /\/tarballs\//u);
  }
  assert.ok(f.requests.filter(request => request.path === '/' + matrix.targets[0].package).length >= 3, 'must actually retry after two unavailable responses');
  const ci = await run('npm', ['ci', '--ignore-scripts', '--no-audit', '--no-fund'], f.root, f.env);
  assert.equal(ci.code, 0, ci.stderr);
  assert.ok(f.requests.every(request => request.userAgent === userAgent));
});

test('unavailable, malformed, and hanging registry metadata fail within budget without changing the lock', { timeout: 30_000 }, async t => {
  for (const options of [{ missing: Infinity }, { corrupt: true }, { hang: true }, { lockMismatch: true }]) {
    const f = await fixture(t, options);
    const result = await run(process.execPath, [script, '--version', version, '--timeout-ms', '1800', '--poll-ms', '50'], f.root, f.env);
    assert.notEqual(result.code, 0, JSON.stringify({ options, requests: f.requests, result }));
    assert.match(result.stderr, /ASGREP_RELEASE_REPIN_TIMEOUT/u);
    assert.equal(await readFile(path.join(f.root, 'package-lock.json'), 'utf8'), f.before);
    assert.ok(f.requests.length > 0, 'must reach the registry rather than fail during setup');
  }
});

test('a different main version is not re-pinned or queried', async t => {
  const f = await fixture(t);
  const result = await run(process.execPath, [script, '--version', '0.0.0'], f.root, f.env);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /skipping/u);
  assert.equal(await readFile(path.join(f.root, 'package-lock.json'), 'utf8'), f.before);
  assert.equal(f.requests.length, 0);
});

test('the pre-tag release guard rejects partially populated native entries before any Git or registry action', async t => {
  const f = await fixture(t);
  const directory = path.join(f.root, 'packages/pi/scripts');
  await mkdir(directory, { recursive: true });
  await copyFile(path.join(repo, 'packages/pi/scripts/release.mjs'), path.join(directory, 'release.mjs'));
  for (const field of ['version', 'resolved', 'integrity']) {
    const lock = JSON.parse(f.before);
    for (const [name, pack] of f.packs) lock.packages['node_modules/' + name] = { version, resolved: `http://localhost/tarballs/${pack.target.id}.tgz`, integrity: pack.integrity, optional: true };
    delete lock.packages['node_modules/' + matrix.targets[0].package][field];
    await writeFile(path.join(f.root, 'package-lock.json'), JSON.stringify(lock));
    const result = await run(process.execPath, [path.join(directory, 'release.mjs'), version, '--dry-run'], f.root, f.env);
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /ASGREP_RELEASE_LOCK_STUBS/u, 'missing ' + field);
  }
  assert.equal(f.requests.length, 0);
});
