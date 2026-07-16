import { spawn } from 'node:child_process';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const fail = (message) => { throw new Error(message); };
const option = (name) => {
  const index = process.argv.indexOf('--' + name);
  if (index < 0 || !process.argv[index + 1]) fail('missing --' + name);
  return path.resolve(process.argv[index + 1]);
};
const launcher = option('launcher');
const installRoot = option('install-root');
const fixture = option('fixture');
const launcherModule = pathToFileURL(path.join(installRoot, 'node_modules', 'ast-sgrep', 'src', 'index.js')).href;
const { resolveBinary } = await import(launcherModule);
const nativeBinary = resolveBinary();

const run = (label, args) => new Promise((resolve, reject) => {
  console.log('[pi-ci] run ' + label + ': asgrep ' + args.map((arg) => arg === fixture ? '<fixture>' : arg).join(' '));
  const child = spawn(process.execPath, [launcher, ...args], { cwd: installRoot, env: { ...process.env, NO_COLOR: '1' }, windowsHide: true });
  let stdout = '';
  let stderr = '';
  child.stdout.setEncoding('utf8');
  child.stderr.setEncoding('utf8');
  child.stdout.on('data', (chunk) => { stdout += chunk; });
  child.stderr.on('data', (chunk) => { stderr += chunk; });
  child.on('error', reject);
  child.on('close', (code, signal) => {
    console.log('[pi-ci] exit ' + label + ': code=' + String(code) + ' signal=' + String(signal));
    if (code !== 0) reject(new Error(label + ' failed\nstdout:\n' + stdout + '\nstderr:\n' + stderr));
    else resolve({ stdout, stderr });
  });
});
const runJson = async (label, args, validate) => {
  const { stdout } = await run(label, args);
  let value;
  try { value = JSON.parse(stdout); } catch { fail(label + ' did not emit one JSON document'); }
  if (!value || typeof value !== 'object' || Array.isArray(value)) fail(label + ' JSON must be an object');
  if (value.ok !== true || value.tool !== 'asgrep' || typeof value.schema_version !== 'string') fail(label + ' JSON envelope is invalid');
  validate(value);
};
const requireSearchHit = (value, predicate, label) => {
  if (value.command !== 'search' || !Array.isArray(value.hits) || !value.hits.some(predicate)) fail(label + ' did not return the expected fixture hit');
};

await rm(fixture, { recursive: true, force: true });
await mkdir(fixture, { recursive: true });
await writeFile(path.join(fixture, 'app.ts'), 'export function greet(name: string) { return "hello " + name; }\nexport function welcome() { return greet("Pi"); }\n');
await writeFile(path.join(fixture, 'worker.ts'), 'import { welcome } from "./app";\nexport const result = welcome();\n');
console.log('[pi-ci] fixture inventory: app.ts,worker.ts');

const version = await run('version', ['--version']);
if (!/\d+\.\d+\.\d+/.test(version.stdout)) fail('version output did not contain semver');
await runJson('fixture-index', ['--root', fixture, '--no-embed', '--json', 'index'], (value) => {
  if (value.command !== 'index' || value.files_failed !== 0 || value.walk_errors !== false || value.files_indexed < 2 || value.symbols_extracted < 2) fail('fixture-index invariants failed');
});
await runJson('status', ['--root', fixture, '--no-embed', '--json', 'status'], (value) => {
  if (value.command !== 'status' || value.file_count < 2 || value.symbol_count < 2 || value.caller_count < 1) fail('status invariants failed');
});
await runJson('semantic-natural-search', ['--root', fixture, '--json', 'find the function that greets a person'], (value) => {
  if (value.query !== 'find the function that greets a person') fail('semantic query was not preserved');
  requireSearchHit(value, (hit) => hit.file === 'app.ts' && /greet|welcome/u.test(hit.excerpt ?? ''), 'semantic-natural-search');
});
await runJson('defs', ['--root', fixture, '--no-embed', '--json', '--format', 'native', 'defs: greet'], (value) => {
  requireSearchHit(value, (hit) => hit.file === 'app.ts' && hit.kind === 'def' && hit.symbol === 'greet', 'defs');
});
await runJson('callers', ['--root', fixture, '--no-embed', '--json', '--format', 'native', 'callers: greet'], (value) => {
  requireSearchHit(value, (hit) => hit.file === 'app.ts' && hit.callee === 'greet' && hit.caller === 'welcome', 'callers');
});
await runJson('doctor', ['--root', fixture, '--no-embed', '--json', 'doctor'], (value) => {
  if (value.command !== 'doctor' || value.healthy !== true || !Array.isArray(value.issues) || value.issues.length !== 0 || value.status?.file_count < 2) fail('doctor invariants failed');
});

const cancellationRoot = path.join(fixture, 'cancellation');
await mkdir(cancellationRoot, { recursive: true });
await Promise.all(Array.from({ length: 256 }, (_, index) => writeFile(path.join(cancellationRoot, 'file-' + String(index).padStart(3, '0') + '.ts'), 'export function symbol' + index + '() { return ' + index + '; }\n')));
await new Promise((resolve, reject) => {
  const controller = new AbortController();
  console.log('[pi-ci] run timeout-cancellation: asgrep --root <fixture>/cancellation --no-embed --json index');
  const child = spawn(nativeBinary, ['--root', cancellationRoot, '--no-embed', '--json', 'index'], { cwd: installRoot, env: { ...process.env, NO_COLOR: '1' }, signal: controller.signal, windowsHide: true, stdio: 'ignore' });
  let aborted = false;
  child.once('spawn', () => { aborted = true; controller.abort(); });
  child.once('error', (error) => { if (!(aborted && error.name === 'AbortError')) reject(error); });
  child.once('close', (code, signal) => {
    console.log('[pi-ci] exit timeout-cancellation: aborted=' + aborted + ' code=' + String(code) + ' signal=' + String(signal));
    if (!aborted) reject(new Error('timeout/cancellation command exited before it was cancelled'));
    else resolve();
  });
});

const extensionUrl = pathToFileURL(path.join(installRoot, 'node_modules', 'pi-ast-sgrep', 'dist', 'index.js')).href;
await import(extensionUrl);
console.log('[pi-ci] extension-load: ok');
