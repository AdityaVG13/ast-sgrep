import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// npm run release:fallback -- X.Y.Z      publish + verify (interactive: one npm 2FA approval)
// npm run release:verify -- X.Y.Z        verify only (no publishes; safe anytime)
//
// The starved-Actions fallback (2026-10-19: the OIDC publication job queued
// 4+ h on exhausted runner minutes while all build+verify stages were green
// and the tarballs sat on the GitHub release). This script publishes the
// preserved release tarballs from your machine with your npm login:
//
//   1. tag + release assets exist and match the release manifest (integrity,
//      not just names); manifest commit == tag's peeled commit
//   2. `npm publish` each still-missing package in canonical order
//      (native -> launcher -> extension), skipping live ones — idempotent
//   3. set `latest` on all seven in the SAME 2FA window (a 2026-10-19
//      publish batch moved versions without moving dist-tags; unexplained,
//      so tags are always set explicitly right after publishing)
//   4. flip the GitHub release out of draft and mark it Latest (the lane's
//      publication job normally does this; when it starves, the release
//      object stays a draft and the sidebar crowns the previous version)
//   5. verify: all seven live at X.Y.Z, every dist-tag latest, the GitHub
//      latest release is this tag, and clean installs of the launcher and
//      the extension actually run
//
// Provenance caveat: publishes via your token carry no OIDC attestation.
// Acceptable as the documented fallback; note it in the release prose when
// it happens. The lane stays the canonical publisher.

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const args = process.argv.slice(2);
const verifyOnly = args.includes('--verify-only');
const version = args.find((arg) => !arg.startsWith('--'));
const fail = (code, message) => { console.error(`[fallback] ${code}: ${message}`); process.exit(1); };
if (!/^\d+\.\d+\.\d+$/.test(version ?? '')) fail('ASGREP_FALLBACK_USAGE', 'usage: npm run release:fallback -- X.Y.Z   (or release:verify -- X.Y.Z)');
const tag = `v${version}`;

const sh = (command, argv, options = {}) => spawnSync(command, argv, { cwd: root, encoding: 'utf8', windowsHide: true, ...options });
const capture = (command, argv) => {
  const r = sh(command, argv);
  return { status: r.status, stdout: String(r.stdout ?? '').trim(), stderr: String(r.stderr ?? '').trim() };
};

// --- 1. tag + release assets -------------------------------------------------
const localTag = capture('git', ['rev-list', '-n', '1', `refs/tags/${tag}`]);
if (localTag.status !== 0) fail('ASGREP_FALLBACK_TAG', `${tag} missing locally`);
const remote = capture('git', ['ls-remote', 'origin', `refs/tags/${tag}`, `refs/tags/${tag}^{}`]);
const peeled = remote.stdout.split(/\r?\n/u).find((line) => line.endsWith(`refs/tags/${tag}^{}`))?.split('\t')[0];
const remoteTag = peeled ?? remote.stdout.split(/\r?\n/u).find((line) => line.endsWith(`refs/tags/${tag}`))?.split('\t')[0];
if (!remoteTag) fail('ASGREP_FALLBACK_TAG', `${tag} is not pushed to origin`);
if ((peeled ?? remoteTag) !== localTag.stdout) fail('ASGREP_FALLBACK_TAG', `${tag} on origin points elsewhere`);

const releaseJson = capture('gh', ['release', 'view', tag, '--json', 'isDraft,isPrerelease,assets']);
if (releaseJson.status !== 0) fail('ASGREP_FALLBACK_RELEASE', `no GitHub release for ${tag}: ${releaseJson.stderr.split('\n').slice(-1)[0]}`);
const release = JSON.parse(releaseJson.stdout);

const temporary = await mkdtemp(path.join(tmpdir(), `asgrep-fallback-${version}-`));
try {
  const download = sh('gh', ['release', 'download', tag, '--repo', 'AdityaVG13/ast-sgrep', '--dir', temporary, '--pattern', '*.tgz', '--pattern', 'release-manifest.json']);
  if (download.status !== 0) fail('ASGREP_FALLBACK_ASSETS', `asset download failed: ${String(download.stderr ?? '').split('\n').slice(-1)[0]}`);
  const manifest = JSON.parse(await readFile(path.join(temporary, 'release-manifest.json'), 'utf8'));
  if (manifest.version !== version) fail('ASGREP_FALLBACK_MANIFEST', `manifest version ${manifest.version} != ${version}`);
  if (manifest.commit !== localTag.stdout) fail('ASGREP_FALLBACK_MANIFEST', `manifest commit ${manifest.commit.slice(0, 8)} != ${tag} commit ${localTag.stdout.slice(0, 8)}`);
  const artifacts = manifest.artifacts ?? [];
  if (artifacts.length !== 7) fail('ASGREP_FALLBACK_MANIFEST', `manifest lists ${artifacts.length} artifacts, expected 7`);

  for (const artifact of artifacts) {
    const bytes = await readFile(path.join(temporary, artifact.filename));
    const sha1 = createHash('sha1').update(bytes).digest('hex');
    const ok = artifact.shasum ? artifact.shasum === sha1
      : artifact.integrity ? artifact.integrity === `sha512-${createHash('sha512').update(bytes).digest('base64')}`
      : null;
    if (ok === null) fail('ASGREP_FALLBACK_MANIFEST', `artifact ${artifact.name} carries neither shasum nor integrity`);
    if (!ok) fail('ASGREP_FALLBACK_INTEGRITY', `${artifact.filename} does not match the manifest`);
  }
  console.log(`[fallback] ${tag}: manifest + 7 tarballs verified against ${localTag.stdout.slice(0, 8)}`);

  // --- 2/3. publish missing + set tags (one 2FA window) ---------------------
  const live = [];
  const toPublish = [];
  for (const artifact of artifacts) {
    const view = capture('npm', ['view', `${artifact.name}@${version}`, 'version']);
    if (view.status === 0) live.push(artifact.name); else toPublish.push(artifact);
  }
  console.log(`[fallback] registry: ${live.length} live, ${toPublish.length} to publish`);

  if (!verifyOnly && toPublish.length) {
    console.log('[fallback] npm 2FA: the FIRST publish below prints an npm URL — open it, approve, and the rest of the window finishes unattended.\n');
    for (const artifact of toPublish) {
      console.log(`[fallback] npm publish ${artifact.filename}`);
      const r = sh('npm', ['publish', path.join(temporary, artifact.filename), '--access', 'public'], { stdio: 'inherit' });
      if (r.status !== 0) fail('ASGREP_FALLBACK_PUBLISH', `${artifact.name} publish failed (fix and rerun; completed ones are skipped)`);
    }
    // Tags in the same window, always explicit — never trust publish to have moved latest.
    for (const artifact of artifacts) {
      const r = sh('npm', ['dist-tag', 'add', `${artifact.name}@${version}`, 'latest']);
      if (r.status !== 0) fail('ASGREP_FALLBACK_TAG_ADD', `npm dist-tag add failed for ${artifact.name} (rerun release:verify once the 2FA window is fresh)`);
    }
    console.log('[fallback] dist-tags set explicitly on all seven');
  }

  // --- 4. release object out of draft ---------------------------------------
  if (!verifyOnly && (release.isDraft || release.isPrerelease)) {
    const r = sh('gh', ['release', 'edit', tag, '--draft=false', '--prerelease=false', '--latest']);
    if (r.status !== 0) fail('ASGREP_FALLBACK_RELEASE_EDIT', `gh release edit failed: ${String(r.stderr ?? '').split('\n').slice(-1)[0]}`);
    console.log('[fallback] release object published and marked Latest');
  }

  // --- 5. verification tail ---------------------------------------------------
  const problems = [];
  for (const artifact of artifacts) {
    const view = capture('npm', ['view', `${artifact.name}@${version}`, 'version']);
    if (view.status !== 0 || view.stdout !== version) problems.push(`${artifact.name}@${version} not live`);
    const distTag = capture('npm', ['view', artifact.name, 'dist-tags.latest']);
    if (distTag.stdout !== version) problems.push(`${artifact.name} latest=${distTag.stdout || '?'}`);
  }
  const ghLatest = capture('gh', ['api', 'repos/AdityaVG13/ast-sgrep/releases/latest', '--jq', '.tag_name']);
  if (ghLatest.stdout !== tag) problems.push(`GitHub latest release = ${ghLatest.stdout || '?'} (want ${tag})`);

  const smokeDir = path.join(temporary, 'smoke');
  const smoke = (project, check) => {
    const install = sh('npm', ['install', '--prefix', smokeDir, `${project}@${version}`, '--no-audit', '--no-fund']);
    if (install.status !== 0) { problems.push(`install ${project}@${version} failed`); return; }
    const r = check(smokeDir);
    if (!r.ok) problems.push(`${project} smoke: ${r.why}`);
  };
  smoke('ast-sgrep', (dir) => {
    const run = capture(path.join(dir, 'node_modules', '.bin', 'asgrep'), ['--version']);
    return run.status === 0 && run.stdout.includes(version) ? { ok: true } : { ok: false, why: `asgrep --version -> "${run.stdout.slice(0, 40)}"` };
  });
  smoke('pi-ast-sgrep', (dir) => {
    try {
      const pkg = JSON.parse(readFileSync(path.join(dir, 'node_modules', 'pi-ast-sgrep', 'package.json'), 'utf8'));
      if (pkg.version !== version) return { ok: false, why: `manifest version ${pkg.version}` };
      if (!existsSync(path.join(dir, 'node_modules', 'pi-ast-sgrep', 'dist', 'index.js'))) return { ok: false, why: 'dist/index.js missing' };
      const launcher = JSON.parse(readFileSync(path.join(dir, 'node_modules', 'ast-sgrep', 'package.json'), 'utf8'));
      return launcher.version === version ? { ok: true } : { ok: false, why: `launcher resolved ${launcher.version}` };
    } catch (error) { return { ok: false, why: String(error).slice(0, 80) }; }
  });

  if (problems.length) { console.error('[fallback] VERIFY FAILED:\n  ' + problems.join('\n  ')); process.exit(1); }
  console.log(verifyOnly
    ? `[fallback] verify-only OK: 7/7 live at ${version}, tags + GitHub latest + install smoke green`
    : `[fallback] DONE: ${version} fully published and verified (token publish; no OIDC provenance — note it in release prose)`);
} finally {
  await rm(temporary, { recursive: true, force: true });
}
