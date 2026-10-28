import { spawnSync } from 'node:child_process';
import { copyFile, mkdir, mkdtemp, readFile, rename, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

// Post-publication only. npm can succeed while recording unresolvable optional
// stubs, so neither exit zero nor registry visibility alone is sufficient.
// The checkout's lock is replaced only after an isolated regeneration passes
// exact version/tarball/integrity checks against all five live native packages.
const fail = (code, message) => { throw new Error(`${code}: ${message}`); };
const root = process.cwd();
const option = (name, fallback) => {
  const index = process.argv.indexOf('--' + name);
  return index < 0 ? fallback : process.argv[index + 1];
};
// Shared predicate for the live retry loop and the self-test regression
// (br-zvh): a regenerated lock is committable only when EVERY expected
// native entry carries the exact canonical version, is not a link, and
// resolves to the observed tarball + integrity. This is the rejection that
// the historical 2.5.4 single-shot repin lacked.
const validateNativeLock = (candidate, targets, observed, version) => {
  const violations = [];
  for (const target of targets) {
    const entry = candidate.packages?.['node_modules/' + target.package];
    const dist = observed.get(target.package);
    if (entry?.version !== version || entry.link || entry.resolved !== dist.tarball || entry.integrity !== dist.integrity) {
      violations.push(`${target.package}@${version}: regenerated lock has missing or mismatched native metadata`);
    }
  }
  return violations;
};
// Regression for the 2.5.4 incomplete-lock class: every stub variant the
// live loop must reject, plus the complete candidate that must pass.
const selfTest = () => {
  const targets = [
    { package: '@ast-sgrep/native-a' },
    { package: '@ast-sgrep/native-b' },
  ];
  const version = '2.7.0';
  const observed = new Map(targets.map(target => [target.package, {
    tarball: `https://registry.npmjs.org/${target.package}/-/${target.package.split('/').pop()}-${version}.tgz`,
    integrity: `sha512-${'A'.repeat(86)}=`,
  }]));
  const entry = (pkg, overrides = {}) => ({
    version,
    resolved: observed.get(pkg).tarball,
    integrity: observed.get(pkg).integrity,
    ...overrides,
  });
  const complete = {
    packages: Object.fromEntries(targets.map(target => ['node_modules/' + target.package, entry(target.package)])),
  };
  const stubs = {
    'versionless optional stub': { packages: { ...complete.packages, 'node_modules/@ast-sgrep/native-a': entry(targets[0].package, { version: '' }) } },
    'link stub': { packages: { ...complete.packages, 'node_modules/@ast-sgrep/native-a': entry(targets[0].package, { link: true }) } },
    'mismatched integrity': { packages: { ...complete.packages, 'node_modules/@ast-sgrep/native-a': entry(targets[0].package, { integrity: `sha512-${'B'.repeat(86)}=` }) } },
    'stale resolved url': { packages: { ...complete.packages, 'node_modules/@ast-sgrep/native-a': entry(targets[0].package, { resolved: 'https://registry.npmjs.org/stale.tgz' }) } },
    'missing native entry': { packages: { 'node_modules/@ast-sgrep/native-b': entry(targets[1].package) } },
  };
  const rejected = [];
  for (const [name, candidate] of Object.entries(stubs)) {
    const violations = validateNativeLock(candidate, targets, observed, version);
    if (violations.length === 0) throw new Error(`[pi-release] repin self-test FAILED: ${name} lock was accepted`);
    rejected.push(name);
  }
  if (validateNativeLock(complete, targets, observed, version).length) throw new Error('[pi-release] repin self-test FAILED: complete candidate lock was rejected');
  console.log(`[pi-release] repin self-test accepted a complete lock and rejected ${rejected.join(', ')}`);
};
if (process.argv[2] === '--self-test') {
  selfTest();
  process.exit(0);
}
const version = option('version');
const timeoutMs = Number(option('timeout-ms', '120000'));
const pollMs = Number(option('poll-ms', '2000'));
if (!/^\d+\.\d+\.\d+$/u.test(version ?? '') || !Number.isSafeInteger(timeoutMs) || timeoutMs <= 0 || !Number.isSafeInteger(pollMs) || pollMs <= 0) {
  fail('ASGREP_RELEASE_REPIN_USAGE', 'usage: repin-lock.mjs --version X.Y.Z [--timeout-ms 120000] [--poll-ms 2000]');
}
const readJson = async file => JSON.parse(await readFile(file, 'utf8'));
try {
  const manifest = await readJson(path.join(root, 'package.json'));
  if (manifest.version !== version) {
    console.log(`[pi-release] main is at ${manifest.version}, release was ${version}; skipping lock re-pin`);
  } else {
    const contract = await readJson(path.join(root, 'packages/pi/release-contract.json'));
    const matrix = await readJson(path.join(root, 'packages/pi/release/targets.json'));
    const launcher = await readJson(path.join(root, 'packages/pi/launcher/package.json'));
    if (contract.canonicalVersion.version !== version || launcher.version !== version || matrix.targets.length !== 5 || matrix.targets.some(target => launcher.optionalDependencies?.[target.package] !== version)) {
      fail('ASGREP_RELEASE_VERSION_SKEW', 'canonical version, launcher, and every native dependency must match the release');
    }
    const lockPath = path.join(root, 'package-lock.json');
    const original = await readFile(lockPath);
    const lock = JSON.parse(original);
    const staging = await mkdtemp(path.join(tmpdir(), 'asgrep-repin-'));
    // Retain candidate files on failure for diagnosis; never erase the original
    // lock or operate on the checkout's installed dependencies.
    console.log(`[pi-release] isolated re-pin candidate: ${staging}`);
    for (const directory of ['', ...manifest.workspaces]) {
      await mkdir(path.join(staging, directory), { recursive: true });
      await copyFile(path.join(root, directory, 'package.json'), path.join(staging, directory, 'package.json'));
    }
    const nativePaths = Object.keys(lock.packages ?? {}).filter(key => matrix.targets.some(target => key.endsWith('node_modules/' + target.package)));
    for (const key of nativePaths) delete lock.packages[key];
    const remaining = () => Math.max(0, timeoutMs - (performance.now() - started));
    const started = performance.now();
    const npm = args => {
      const budget = Math.ceil(remaining());
      if (!budget) throw new Error('registry deadline exhausted');
      const result = spawnSync('npm', [...args, '--fetch-retries=0', '--fetch-timeout=' + budget, '--prefer-online'], {
        cwd: staging, encoding: 'utf8', timeout: budget, killSignal: 'SIGKILL', windowsHide: true,
        env: { ...process.env, npm_config_user_agent: 'OpenAI File Downloader, XaiImageApiFetch/1.0', npm_config_cache: path.join(staging, 'cache') }
      });
      if (result.error || result.status !== 0) throw new Error(`npm ${args[0]} failed: ${(result.error?.message ?? result.stderr ?? '').trim().slice(-800)}`);
      return result.stdout;
    };
    // Use the checkout's ordinary registry configuration without exposing it in
    // diagnostics. CI is public/OIDC-only; local registries are useful for E2E.
    await copyFile(path.join(root, '.npmrc'), path.join(staging, '.npmrc')).catch(error => { if (error.code !== 'ENOENT') throw error; });
    let lastError = 'native registry metadata is not visible';
    let candidate;
    while (remaining() > 0) {
      try {
        const observed = new Map();
        for (const target of matrix.targets) {
          const live = JSON.parse(npm(['view', `${target.package}@${version}`, '--json']));
          if (live.version !== version || typeof live.dist?.tarball !== 'string' || !/^https?:\/\//u.test(live.dist.tarball) || !/^sha512-[A-Za-z0-9+/]+={0,2}$/u.test(live.dist.integrity ?? '')) {
            throw new Error(`${target.package}@${version}: version/tarball/integrity is not yet complete`);
          }
          observed.set(target.package, live.dist);
        }
        // Discard only the staged native entries on every retry, including stubs
        // npm may have cached in its successful-but-incomplete previous result.
        await writeFile(path.join(staging, 'package-lock.json'), JSON.stringify(lock, null, 2) + '\n');
        npm(['install', '--package-lock-only', '--ignore-scripts', '--include=optional', '--no-audit', '--no-fund']);
        candidate = await readJson(path.join(staging, 'package-lock.json'));
        const violations = validateNativeLock(candidate, matrix.targets, observed, version);
        if (violations.length) throw new Error(violations[0]);
        if (remaining() <= 0) throw new Error('registry deadline exhausted before validation');
        break;
      } catch (error) {
        candidate = undefined;
        lastError = error.message;
        console.error(`[pi-release] waiting for complete registry/lock metadata: ${lastError}`);
        await delay(Math.min(pollMs, remaining()));
      }
    }
    if (!candidate) fail('ASGREP_RELEASE_REPIN_TIMEOUT', `no valid native lock within ${timeoutMs}ms; checkout unchanged. Last failure: ${lastError}`);
    if (!(await readFile(lockPath)).equals(original)) fail('ASGREP_RELEASE_REPIN_CHANGED', 'checkout lock changed during re-pin; refusing to replace concurrent edits');
    await rename(path.join(staging, 'package-lock.json'), lockPath);
    console.log(`[pi-release] validated lock re-pin: all ${matrix.targets.length} native packages resolve at ${version}`);
  }
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
