import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { mkdir, readFile, rm } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// npm run release:fleet -- plan                 resolve hosts, check tag/commit discipline, print the matrix
// npm run release:fleet -- build [--only ID]    build the 5-target native trio on the fleet (mac-local + spark + windows)
// npm run release:fleet -- pack [--skip-e2e]    stage + pack the seven-package family + manifest, verify, packaged e2e
// npm run release:fleet -- assets               create/upload the draft GitHub release assets (byte-compared)
// npm run release:fleet -- all [--skip-e2e]     build -> pack -> assets
//
// The fleet lane: same contracts as .github/workflows/pi-npm-release.yml
// (dormant spec), executed on owned hosts instead of metered runners —
// darwin targets on this Mac, linux targets on the Spark box (arm64 native +
// x86_64 cross via the x86_64-linux-gnu-gcc toolchain), win32 on the Windows
// box (MSVC, /Brepro for deterministic PDB timestamps). Every host builds the
// exact tag tree (git archive | ssh tar), so tag == checkout == build commit,
// the invariant release-artifact.mjs records in artifact-metadata.json.
//
// Hosts are private: configure with env vars or ~/.config/dsr/hosts.yaml
// (the DSR registry is the fallback source of truth; nothing is hardcoded
// in this public repo):
//   ASGREP_FLEET_SPARK_SSH / ASGREP_FLEET_SPARK_DIR
//   ASGREP_FLEET_WIN_SSH   / ASGREP_FLEET_WIN_DIR
//
// Publication is NOT here: npm publish keeps one non-OIDC path —
//   npm run release:fallback -- X.Y.Z --from-dir <pack-out>
// (token publish, one 2FA window; the GitHub lane remains the OIDC publisher
// when Actions is not starved). Post-publish: repin-lock.mjs --version X.Y.Z.

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const command = process.argv[2];
const only = process.argv.includes('--only') ? process.argv[process.argv.indexOf('--only') + 1] : null;
const allowUntagged = process.argv.includes('--allow-untagged');
const skipE2e = process.argv.includes('--skip-e2e');
const fail = (code, message) => { console.error(`[fleet] ${code}: ${message}`); process.exit(1); };
if (!['plan', 'build', 'pack', 'assets', 'all'].includes(command ?? '')) {
  fail('ASGREP_FLEET_USAGE', 'usage: npm run release:fleet -- plan|build|pack|assets|all [--only TARGET] [--skip-e2e] [--allow-untagged]');
}

const readJson = (file) => JSON.parse(readFileSync(path.join(root, file), 'utf8'));
const contract = readJson('packages/pi/release-contract.json');
const matrix = readJson('packages/pi/release/targets.json');
const version = contract.canonicalVersion.version;
const tag = contract.canonicalVersion.tag;

const sh = (cmd, args, options = {}) => spawnSync(cmd, args, { cwd: root, encoding: 'utf8', windowsHide: true, ...options });
const capture = (cmd, args) => {
  const r = sh(cmd, args);
  return { status: r.status, stdout: String(r.stdout ?? '').trim(), stderr: String(r.stderr ?? '').trim() };
};
const step = (name, ok, detail = '') => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? ' — ' + detail : ''}`); if (!ok) fail('ASGREP_FLEET_STEP', name); return ok; };
const run = (cmd, args, options = {}) => {
  const r = sh(cmd, args, { stdio: 'inherit', ...options });
  if (r.status !== 0) fail('ASGREP_FLEET_COMMAND', `${cmd} ${args.join(' ')} failed (${r.status})`);
};

// --- release discipline: clean tree, tag == checkout -------------------------
const dirty = capture('git', ['status', '--porcelain']);
if (dirty.status !== 0) fail('ASGREP_FLEET_STEP', 'git status failed');
if (dirty.stdout === '') console.log('PASS  clean tree');
else if (allowUntagged) console.log(`[fleet] UNTAGGED smoke with a dirty tree (${dirty.stdout.split('\n').length} changed paths) — artifacts record HEAD only`);
else step('clean tree', false, 'uncommitted changes present');
const head = capture('git', ['rev-parse', 'HEAD']);
step('resolve HEAD', head.status === 0 && /^[0-9a-f]{40}$/u.test(head.stdout), head.stdout.slice(0, 8));
const tagCommit = capture('git', ['rev-list', '-n', '1', `refs/tags/${tag}`]);
const tagged = tagCommit.status === 0 && tagCommit.stdout === head.stdout;
if (!tagged) {
  if (allowUntagged) console.log(`[fleet] UNTAGGED smoke mode: HEAD ${head.stdout.slice(0, 8)} != ${tag}; artifacts record HEAD, assets/publish stay closed`);
  else fail('ASGREP_FLEET_TAG', `HEAD is not ${tag} (tag at ${tagCommit.stdout.slice(0, 8) || 'missing'}). Commit and tag the release first, or pass --allow-untagged for a build smoke`);
}
const commit = head.stdout;

// --- hosts: env override, then the DSR registry ------------------------------
const dsrHost = (name) => {
  if (existsSync(path.join(homedir(), '.config/dsr/hosts.yaml'))) {
    const text = readFileSync(path.join(homedir(), '.config/dsr/hosts.yaml'), 'utf8');
    const block = new RegExp(`^  ${name}:\\n((?:    .*\\n?)+)`, 'mu').exec(text);
    const sshHost = block ? /^    ssh_host:\s*(\S+)/mu.exec(block[1]) : null;
    if (sshHost) return sshHost[1];
  }
  return null;
};
const sparkSsh = process.env.ASGREP_FLEET_SPARK_SSH ?? dsrHost('spark');
const sparkDir = process.env.ASGREP_FLEET_SPARK_DIR ?? '/home/aditya/dsr/ast-sgrep-fleet';
const winSsh = process.env.ASGREP_FLEET_WIN_SSH ?? dsrHost('windows-native');
const winDir = process.env.ASGREP_FLEET_WIN_DIR ?? 'C:/Users/aditya/dsr/ast-sgrep-fleet';

// --- build plan ---------------------------------------------------------------
const cliArgs = ['build', '--locked', '--release', '-p', 'ast-sgrep-cli', '--bin', 'asgrep', '--no-default-features'];
const watchArgs = ['build', '--locked', '--release', '-p', 'ast-sgrep-watch', '--bin', 'asgrep-watch'];
const napiArgs = ['build', '--locked', '--release', '-p', 'ast-sgrep-codemode-napi'];
const napiFile = (os) => os === 'darwin' ? 'libast_sgrep_codemode_napi.dylib' : os === 'linux' ? 'libast_sgrep_codemode_napi.so' : 'ast_sgrep_codemode_napi.dll';
const legs = [
  { id: 'darwin-arm64', host: 'mac', target: 'aarch64-apple-darwin' },
  { id: 'darwin-x64', host: 'mac', target: 'x86_64-apple-darwin' },
  { id: 'linux-arm64-gnu', host: 'spark', target: 'aarch64-unknown-linux-gnu' },
  { id: 'linux-x64-gnu', host: 'spark', target: 'x86_64-unknown-linux-gnu', cross: true },
  { id: 'win32-x64-msvc', host: 'windows', target: 'x86_64-pc-windows-msvc' }
];
const targetOf = (id) => matrix.targets.find((t) => t.id === id) ?? fail('ASGREP_FLEET_TARGET', 'unknown target: ' + id);

if (command === 'plan') {
  console.log(`[fleet] ${tag} @ ${commit.slice(0, 8)}${tagged ? '' : ' (UNTAGGED smoke)'}`);
  step('spark host', Boolean(sparkSsh), sparkSsh ?? 'set ASGREP_FLEET_SPARK_SSH (or add spark to ~/.config/dsr/hosts.yaml)');
  step('windows host', Boolean(winSsh), winSsh ?? 'set ASGREP_FLEET_WIN_SSH (or add windows-native to ~/.config/dsr/hosts.yaml)');
  for (const leg of legs) console.log(`  ${leg.id.padEnd(16)} ${leg.host.padEnd(8)} ${leg.target}`);
  process.exit(0);
}

// --- build --------------------------------------------------------------------
const rawDir = path.join(root, 'dist/fleet/raw');
const nativeRoot = path.join(root, 'dist/native');
const packOut = path.join(root, 'dist/fleet/npm-packs');

const localBuild = (leg) => {
  console.log(`[fleet] building ${leg.id} locally (${leg.target})`);
  const env = { ...process.env, RUSTC_WRAPPER: '' };
  for (const args of [cliArgs, watchArgs, napiArgs]) run('cargo', [...args, '--target', leg.target], { env });
  const base = path.join(root, 'target', leg.target, 'release');
  return { binary: path.join(base, 'asgrep'), watch: path.join(base, 'asgrep-watch'), napi: path.join(base, napiFile('darwin')) };
};

const sshSpark = (script, options = {}) => sh('ssh', ['-o', 'ConnectTimeout=15', sparkSsh, script], options);
const sshWin = (script, options = {}) => sh('ssh', ['-o', 'ConnectTimeout=15', winSsh, script], options);

const syncTreeTo = (sshHost, remote) => {
  // Byte-exact tag tree: git archive pipes only tracked files at the commit.
  const remoteCmd = sshHost === winSsh
    ? `New-Item -ItemType Directory -Force ${remote} | Out-Null; tar -xf - -C ${remote}`
    : `mkdir -p ${remote} && tar -x -C ${remote}`;
  const r = sh('bash', ['-c', `git -C ${root} archive ${commit} | ssh -o ConnectTimeout=15 ${sshHost} '${remoteCmd.replace(/'/gu, "'\\''")}'`]);
  step(`sync tag tree -> ${sshHost}`, r.status === 0, r.status === 0 ? '' : String(r.stderr ?? '').trim().split('\n').slice(-1)[0]);
};

const sparkBuild = (leg) => {
  console.log(`[fleet] building ${leg.id} on spark (${leg.target}${leg.cross ? ' cross' : ''})`);
  const cross = leg.cross
    ? `export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc AR_x86_64_unknown_linux_gnu=x86_64-linux-gnu-ar; `
    : '';
  const builds = [cliArgs, watchArgs, napiArgs]
    .map((args) => `cargo ${args.join(' ')} --target ${leg.target}`)
    .join(' && ');
  const r = sshSpark(`cd ${sparkDir} && ${cross}${builds} && echo FLEET_BUILD_OK`, { stdio: 'inherit' });
  step(`spark build ${leg.id}`, r.status === 0);
  const base = `${sparkDir}/target/${leg.target}/release`;
  return { binary: `${base}/asgrep`, watch: `${base}/asgrep-watch`, napi: `${base}/${napiFile('linux')}` };
};

const windowsBuild = (leg) => {
  console.log(`[fleet] building ${leg.id} on windows (${leg.target}, MSVC /Brepro)`);
  // cmd.exe chaining (proven shape on this host): env var persists across the
  // three builds; && propagates the first failure.
  const builds = [cliArgs, watchArgs, napiArgs]
    .map((args) => `cargo ${args.join(' ')} --target ${leg.target}`)
    .join(' && ');
  const script = `cd /d ${winDir.replace(/\//gu, '\\')} && set RUSTFLAGS=-C link-arg=/Brepro&& ${builds} && echo FLEET_BUILD_OK`;
  const r = sshWin(`cmd /d /s /c "${script.replace(/"/gu, "'")}"`, { stdio: 'inherit' });
  step(`windows build ${leg.id}`, r.status === 0);
  const base = `${winDir}/target/${leg.target}/release`;
  return { binary: `${base}/asgrep.exe`, watch: `${base}/asgrep-watch.exe`, napi: `${base}/${napiFile('win32')}` };
};

const fetch = async (leg, artifacts, sshHost) => {
  const local = path.join(rawDir, leg.id);
  await mkdir(local, { recursive: true });
  for (const [kind, remotePath] of Object.entries(artifacts)) {
    const name = kind === 'binary' ? targetOf(leg.id).executable : kind === 'watch' ? targetOf(leg.id).watchHelper : 'napi-raw';
    const r = sh('scp', ['-o', 'ConnectTimeout=15', `${sshHost}:${remotePath}`, path.join(local, name)]);
    step(`fetch ${leg.id}/${name}`, r.status === 0, r.status === 0 ? '' : String(r.stderr ?? '').trim().split('\n').slice(-1)[0]);
  }
  return {
    binary: path.join(local, targetOf(leg.id).executable),
    watch: path.join(local, targetOf(leg.id).watchHelper),
    napi: path.join(local, 'napi-raw')
  };
};

const buildLeg = async (leg) => {
  let artifacts;
  if (leg.host === 'mac') artifacts = localBuild(leg);
  else {
    const sshHost = leg.host === 'spark' ? sparkSsh : winSsh;
    syncTreeTo(sshHost, leg.host === 'spark' ? sparkDir : winDir);
    artifacts = leg.host === 'spark' ? sparkBuild(leg) : windowsBuild(leg);
    artifacts = await fetch(leg, artifacts, sshHost);
  }
  await mkdir(nativeRoot, { recursive: true });
  const out = path.join(nativeRoot, leg.id);
  await rm(out, { recursive: true, force: true });
  run(process.execPath, ['packages/pi/scripts/release-artifact.mjs', 'prepare',
    '--target', leg.id, '--binary', artifacts.binary, '--watch', artifacts.watch, '--napi', artifacts.napi,
    '--output', out, '--commit', commit]);
  run(process.execPath, ['packages/pi/scripts/release-artifact.mjs', 'verify', '--target', leg.id, '--input', out]);
  console.log(`[fleet] ${leg.id}: staged + verified`);
};

const doBuild = async () => {
  step('spark host', Boolean(sparkSsh), sparkSsh ?? 'missing');
  step('windows host', Boolean(winSsh), winSsh ?? 'missing');
  const selected = legs.filter((leg) => !only || leg.id === only);
  if (only && !selected.length) fail('ASGREP_FLEET_TARGET', `--only ${only} matches no target`);
  await mkdir(rawDir, { recursive: true });
  for (const leg of selected) await buildLeg(leg);
};

// --- pack + packaged e2e --------------------------------------------------------
const doPack = async () => {
  for (const leg of legs) {
    if (!existsSync(path.join(nativeRoot, leg.id, 'artifact-metadata.json'))) {
      fail('ASGREP_FLEET_NATIVE_MISSING', `${nativeRoot}/${leg.id} is not staged; run release:fleet build first`);
    }
  }
  await rm(packOut, { recursive: true, force: true });
  run(process.execPath, ['packages/pi/scripts/release-acceptance.mjs', 'pack', '--native-root', nativeRoot, '--output', packOut, '--commit', commit]);
  run(process.execPath, ['packages/pi/scripts/release-acceptance.mjs', 'verify', '--artifacts', packOut]);
  if (skipE2e) { console.log('[fleet] packaged e2e skipped (--skip-e2e)'); return; }
  console.log('[fleet] building the debug trio the packaged e2e drives');
  for (const args of [['build', '--locked', '-p', 'ast-sgrep-cli', '--bin', 'asgrep'], ['build', '--locked', '-p', 'ast-sgrep-watch', '--bin', 'asgrep-watch'], ['build', '--locked', '-p', 'ast-sgrep-codemode-napi']]) {
    run('cargo', args, { env: { ...process.env, RUSTC_WRAPPER: '' } });
  }
  const cdylib = existsSync(path.join(root, 'target/debug/libast_sgrep_codemode_napi.dylib'))
    ? 'target/debug/libast_sgrep_codemode_napi.dylib' : 'target/debug/libast_sgrep_codemode_napi.so';
  run('cp', [cdylib, 'target/debug/ast-sgrep-codemode.node']);
  run('npm', ['run', 'test:pi-e2e']);
  console.log(`[fleet] pack complete: ${packOut}`);
};

// --- draft release assets --------------------------------------------------------
const doAssets = () => {
  if (!tagged) fail('ASGREP_FLEET_TAG', 'assets require the checkout to be exactly ' + tag);
  const view = capture('gh', ['release', 'view', tag, '--json', 'isDraft,isPrerelease,assets']);
  let release;
  if (view.status === 0) release = JSON.parse(view.stdout);
  else {
    console.log(`[fleet] creating draft release ${tag}`);
    run('gh', ['release', 'create', tag, '--verify-tag', '--draft',
      '--title', `ast-sgrep ${tag}`,
      '--notes', `Fleet-built, immutable npm tarballs and manifest for ${tag} (built on owned hosts; publish with release:fallback --from-dir).`]);
    release = { isDraft: true, isPrerelease: false, assets: [] };
  }
  const remoteNames = new Set((release.assets ?? []).map((asset) => asset.name));
  const scratch = path.join(root, 'dist/fleet/remote-assets');
  const upload = [];
  const names = readdirSync(packOut).filter((name) => name.endsWith('.tgz') || name === 'release-manifest.json');
  for (const name of names) {
    const file = path.join(packOut, name);
    if (remoteNames.has(name)) {
      run('mkdir', ['-p', scratch]); // byte-compare against the preserved asset, like the lane
      run('gh', ['release', 'download', tag, '--pattern', name, '--dir', scratch]);
      const same = readFileSync(file).equals(readFileSync(path.join(scratch, name)));
      step(`asset unchanged ${name}`, same, same ? '' : 'differs from the preserved release asset');
    } else upload.push(file);
  }
  if (upload.length) run('gh', ['release', 'upload', tag, ...upload]);
  console.log(`[fleet] release ${tag} assets current (${upload.length} uploaded, ${remoteNames.size} preserved)`);
};

if (command === 'build') await doBuild();
else if (command === 'pack') await doPack();
else if (command === 'assets') doAssets();
else { await doBuild(); await doPack(); doAssets(); }
