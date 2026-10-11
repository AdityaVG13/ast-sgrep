import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// npm run release -- X.Y.Z [--dry-run] — the release moment.
//
// Assumes `release:prepare` plus the human remainder (CHANGELOG prose,
// README, docs pointers) are already committed and pushed on main. Verifies
// the tree is exactly that prepared release, runs the preflight battery,
// signs and pushes the official tag, dispatches the family lane (which
// publishes all seven packages in lockstep, then re-pins the lockfile), and
// prints the run page for the single npm-production approval.
//
// Retry-safe: when vX.Y.Z already exists at HEAD (a previous dispatch died
// before publishing), the tag is reused and only the dispatch repeats. A tag
// pointing anywhere else is never moved implicitly.

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const fail = (code, message) => { throw new Error(`${code}: ${message}`); };
const runCapture = (command, args) => {
  const result = spawnSync(command, args, { cwd: root, encoding: 'utf8', windowsHide: true });
  if (result.status !== 0) fail('ASGREP_RELEASE_COMMAND', `${command} ${args.join(' ')} failed (${result.status}): ${String(result.stderr ?? result.stdout ?? '').trim()}`);
  return (result.stdout ?? '').trim();
};
const runInherit = (command, args) => {
  const result = spawnSync(command, args, { cwd: root, encoding: 'utf8', stdio: 'inherit', windowsHide: true });
  if (result.status !== 0) fail('ASGREP_RELEASE_COMMAND', `${command} ${args.join(' ')} failed (${result.status})`);
};
const query = (command, args) => {
  const result = spawnSync(command, args, { cwd: root, encoding: 'utf8', windowsHide: true });
  return result.status === 0 ? (result.stdout ?? '').trim() : null;
};

const args = process.argv.slice(2);
const version = args.find((arg) => !arg.startsWith('--'));
const dryRun = args.includes('--dry-run');
if (!/^\d+\.\d+\.\d+$/.test(version ?? '')) fail('ASGREP_RELEASE_USAGE', 'usage: npm run release -- X.Y.Z [--dry-run] (explicit version only, no keywords)');
for (const arg of args) if (arg.startsWith('--') && arg !== '--dry-run') fail('ASGREP_RELEASE_USAGE', `unknown flag ${arg}`);
const tag = `v${version}`;

const contract = JSON.parse(readFileSync(path.join(root, 'packages/pi/release-contract.json'), 'utf8'));
const canonical = contract.canonicalVersion.version;
const extensionContract = contract.packages?.extension?.version;
const extensionManifest = JSON.parse(readFileSync(path.join(root, 'packages/pi/extension/package.json'), 'utf8')).version;
if (canonical !== version || extensionContract !== version || extensionManifest !== version) {
  fail('ASGREP_RELEASE_NOT_PREPARED', `tree is canonical=${canonical} extension(contract)=${extensionContract} extension(manifest)=${extensionManifest}; run release:prepare for ${version} first`);
}

// Versionless lockfile stubs (br-zvh): `npm install --package-lock-only`
// while the new version is unpublished records {"optional": true}
// placeholders that crash every later `npm ci` with `Invalid Version:` once
// the version goes live. Refuse to tag over them.
const lock = JSON.parse(readFileSync(path.join(root, 'package-lock.json'), 'utf8'));
const stubs = contract.packages.platforms.filter(({ name }) => {
  const entry = lock.packages?.['node_modules/' + name];
  return !entry || entry.link || typeof entry.version !== 'string' || !/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/u.test(entry.version) ||
    typeof entry.resolved !== 'string' || !/^https?:\/\//u.test(entry.resolved) || !/^sha512-[A-Za-z0-9+/]+={0,2}$/u.test(entry.integrity ?? '');
}).map(({ name }) => name);
if (stubs.length) fail('ASGREP_RELEASE_LOCK_STUBS', `lockfile has unresolvable optionals (${stubs.join(', ')}); restore the last published lock (git checkout origin/main -- package-lock.json) — do NOT regenerate until after publish`);

if (runCapture('git', ['rev-parse', '--abbrev-ref', 'HEAD']) !== 'main') fail('ASGREP_RELEASE_BRANCH', 'releases cut from main only');
if (runCapture('git', ['status', '--porcelain']) !== '') fail('ASGREP_RELEASE_DIRTY', 'working tree must be clean');
runCapture('gh', ['auth', 'status']);
runCapture('git', ['fetch', 'origin', 'main']);
const head = runCapture('git', ['rev-parse', 'HEAD']);
if (head !== runCapture('git', ['rev-parse', 'origin/main'])) fail('ASGREP_RELEASE_SYNC', 'main is not in sync with origin/main (push or pull first)');
const tagRef = `refs/tags/${tag}`;
// show-ref has a documented missing-ref status; repository/transport failures
// must never become authorization to create or push a replacement tag.
const localProbe = spawnSync('git', ['show-ref', '--verify', '--quiet', tagRef], { cwd: root, encoding: 'utf8', windowsHide: true });
if (localProbe.status !== 0 && !(localProbe.status === 1 && !localProbe.stderr)) {
  fail('ASGREP_RELEASE_COMMAND', `local tag lookup failed (${localProbe.status}): ${String(localProbe.stderr ?? '').trim()}`);
}
const localTag = localProbe.status === 0 ? runCapture('git', ['rev-list', '-n', '1', tagRef]) : null;
const remoteLines = runCapture('git', ['ls-remote', '--tags', 'origin', tagRef, `${tagRef}^{}`]);
const remoteRefs = new Map();
for (const line of remoteLines ? remoteLines.split(/\r?\n/u) : []) {
  const fields = line.split('\t');
  const [oid, ref] = fields;
  if (fields.length !== 2 || !/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/u.test(oid) ||
      (ref !== tagRef && ref !== `${tagRef}^{}`) || remoteRefs.has(ref)) {
    fail('ASGREP_RELEASE_TAG_LOOKUP', 'origin returned malformed or duplicate tag references');
  }
  remoteRefs.set(ref, oid);
}
if (remoteRefs.has(`${tagRef}^{}`) && !remoteRefs.has(tagRef)) {
  fail('ASGREP_RELEASE_TAG_LOOKUP', 'origin returned a peeled tag without its direct reference');
}
// Annotated (including signed) tags advertise their tag-object OID first.
// Compare the peeled commit; a lightweight tag already advertises the commit.
const remoteTag = remoteRefs.get(`${tagRef}^{}`) ?? remoteRefs.get(tagRef) ?? null;
if (localTag !== null && localTag !== head) fail('ASGREP_RELEASE_TAG_EXISTS', `${tag} exists locally at another commit; move it only manually while nothing is published`);
if (remoteTag !== null && remoteTag !== head) fail('ASGREP_RELEASE_TAG_EXISTS', `${tag} exists on origin at another commit; move it only manually while nothing is published`);

if (dryRun) {
  console.log(`[release] dry run for ${version} at ${head.slice(0, 8)} (preflight + tag + dispatch skipped):`);
  if (localTag === null && remoteTag === null) console.log(`[release]   sign tag ${tag}`);
  if (remoteTag === null) console.log(`[release]   push ${tag} to origin`);
  console.log(`[release]   dispatch pi-npm-release.yml family lane from ${tag}`);
  console.log('[release]   approve the npm-production gate on the run page');
  process.exit(0);
}

console.log(`[release] preflight ${version}`);
runInherit('npm', ['run', 'release:preflight', '--silent']);
if (localTag === null && remoteTag === null) {
  runInherit('git', ['tag', '-s', tag, '-m', `ast-sgrep ${version}: official npm release`]);
  runInherit('git', ['verify-tag', tag]);
} else {
  console.log(`[release] reusing ${tag} at HEAD`);
}
if (remoteTag === null) runInherit('git', ['push', 'origin', `refs/tags/${tag}`]);
const dispatch = runCapture('gh', ['workflow', 'run', 'pi-npm-release.yml', '--ref', tag, '-f', `release_tag=${tag}`, '-f', 'publish=true', '-f', 'layer=family', '-f', 'mode=full']);
const runUrl = dispatch.match(/https:\/\/\S+/u)?.[0] ?? dispatch;
console.log(`[release] dispatched: ${runUrl}`);
console.log('[release] approve the npm-production deployment on the run page (one click) — builds take ~30 min, then all seven packages publish');
if (process.platform === 'darwin' && runUrl.startsWith('https://')) query('open', [runUrl]);
