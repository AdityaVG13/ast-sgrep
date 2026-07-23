import { createHash } from 'node:crypto';
import { chmod, copyFile, mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const fail = (code, message) => { throw new Error(`${code}: ${message}`); };
const readJson = async (file) => JSON.parse(await readFile(file, 'utf8'));
const canonical = (value) => JSON.stringify(value, null, 2) + '\n';
const sha256 = async (file) => createHash('sha256').update(await readFile(file)).digest('hex');
const delay = (ms) => new Promise((resolve) => { setTimeout(resolve, ms); });
const option = (name, fallback) => {
  const index = process.argv.indexOf(`--${name}`);
  return index < 0 ? fallback : process.argv[index + 1] ?? fail('ASGREP_RELEASE_OPTION', `missing value for --${name}`);
};
const run = (command, args, options = {}) => {
  const result = spawnSync(command, args, { cwd: root, encoding: 'utf8', windowsHide: true, ...options });
  if (result.status !== 0) fail('ASGREP_RELEASE_COMMAND', `${command} ${args.join(' ')} failed (${result.status}): ${String(result.stderr ?? result.stdout ?? '').trim()}`);
  return result.stdout ?? '';
};
const load = async () => {
  const contract = await readJson(path.join(root, 'packages/pi/release-contract.json'));
  const matrix = await readJson(path.join(root, 'packages/pi/release/targets.json'));
  const launcher = await readJson(path.join(root, 'packages/pi/launcher/package.json'));
  const extension = await readJson(path.join(root, 'packages/pi/extension/package.json'));
  const platforms = await Promise.all(matrix.targets.map((target) => readJson(path.join(root, 'packages/pi/platforms', target.id, 'package.json'))));
  return { contract, matrix, launcher, extension, platforms, version: contract.canonicalVersion.version };
};
const packageOrder = (state) => [...state.matrix.targets.map((target) => target.package), state.launcher.name, state.extension.name];
const validateAlignment = (state) => {
  const { contract, matrix, launcher, extension, platforms, version } = state;
  if (matrix.targets.length !== 5) fail('ASGREP_RELEASE_TARGETS', 'exactly five native targets are required');
  if (new Set(packageOrder(state)).size !== 7) fail('ASGREP_RELEASE_PACKAGE_DUPLICATE', 'release package names must be unique');
  if (contract.canonicalVersion.tag !== `v${version}`) fail('ASGREP_RELEASE_TAG_VERSION', 'canonical tag does not match canonical version');
  if (launcher.version !== version || extension.version !== version || extension.dependencies?.[launcher.name] !== version) fail('ASGREP_RELEASE_VERSION_SKEW', 'launcher/extension versions and dependency must exactly match the canonical version');
  if (contract.compatibility?.layers?.machineSchema?.version !== '1.0.0') fail('ASGREP_RELEASE_PROTOCOL', 'machine protocol version changed without a release-contract update');
  for (let index = 0; index < matrix.targets.length; index += 1) {
    const target = matrix.targets[index];
    const manifest = platforms[index];
    const dependencyVersion = launcher.optionalDependencies?.[target.package];
    const contractPlatform = contract.packages.platforms[index];
    if (manifest.name !== target.package || manifest.version !== version || dependencyVersion !== version || contractPlatform?.optionalDependencyVersion !== version) fail('ASGREP_RELEASE_VERSION_SKEW', `${target.package} is not exactly aligned to ${version}`);
    if (JSON.stringify(manifest.os) !== JSON.stringify([target.os]) || JSON.stringify(manifest.cpu) !== JSON.stringify([target.cpu]) || JSON.stringify(manifest.libc ?? []) !== JSON.stringify(target.libc ? [target.libc] : [])) fail('ASGREP_RELEASE_PLATFORM_SKEW', `${target.package} platform selectors do not match the target matrix`);
  }
};
const classify = (state, name) => state.matrix.targets.some((target) => target.package === name) ? 'native' : name === state.launcher.name ? 'launcher' : name === state.extension.name ? 'extension' : fail('ASGREP_RELEASE_UNKNOWN_PACKAGE', name);
const validateFiles = (state, artifact) => {
  const files = artifact.files.map((file) => file.path).sort();
  const required = artifact.layer === 'native'
    ? ['LICENSE', 'checksum.sha256', state.matrix.targets.find((target) => target.package === artifact.name).executable, 'package.json']
    : artifact.layer === 'launcher'
      ? ['LICENSE', 'README.md', 'bin/asgrep.js', 'package.json', 'src/index.d.ts', 'src/index.js']
      : ['LICENSE', 'README.md', 'assets/preview.png', 'dist/index.d.ts', 'dist/index.js', 'dist/runtime.d.ts', 'dist/runtime.js', 'package.json', 'skills/ast-sgrep/SKILL.md', 'skills/ast-sgrep/references/query-guide.md'];
  for (const entry of required) if (!files.includes(entry)) fail('ASGREP_RELEASE_CONTENT_MISSING', `${artifact.name} is missing ${entry}`);
  for (const entry of files) if (/(^|\/)(test|node_modules)(\/|$)/u.test(entry) || /\.(rs|toml)$/u.test(entry)) fail('ASGREP_RELEASE_CONTENT_FORBIDDEN', `${artifact.name} unexpectedly contains ${entry}`);
};
const inspectPackResult = (state, result) => {
  if (!Array.isArray(result) || result.length !== 1) fail('ASGREP_RELEASE_PACK_RESULT', 'npm pack must produce exactly one artifact');
  const item = result[0];
  if (!packageOrder(state).includes(item.name) || item.version !== state.version || !item.filename || !Array.isArray(item.files)) fail('ASGREP_RELEASE_PACK_METADATA', 'npm pack metadata is incomplete or skewed');
  const artifact = { name: item.name, version: item.version, layer: classify(state, item.name), filename: item.filename, integrity: item.integrity, shasum: item.shasum, files: item.files.map(({ path: filePath, size, mode }) => ({ path: filePath, size, mode })) };
  validateFiles(state, artifact);
  return artifact;
};
const validateChecksumRecord = (target, checksumText, actual) => {
  if (checksumText === null) fail('ASGREP_RELEASE_CHECKSUM_MISSING', target.package);
  const checksum = checksumText.trim().split(/\s+/u);
  if (checksum.length !== 2 || checksum[1] !== target.executable || checksum[0] !== actual) fail('ASGREP_RELEASE_CHECKSUM_MISMATCH', target.package);
};
const verifyNativeSource = async (target) => {
  const directory = path.join(root, 'packages/pi/platforms', target.id);
  const executable = path.join(directory, target.executable);
  const checksumFile = path.join(directory, 'checksum.sha256');
  const executableStat = await stat(executable).catch(() => fail('ASGREP_RELEASE_EXECUTABLE_MISSING', target.package));
  if (!executableStat.isFile() || executableStat.size === 0) fail('ASGREP_RELEASE_EXECUTABLE_MISSING', target.package);
  const checksumText = await readFile(checksumFile, 'utf8').catch(() => null);
  validateChecksumRecord(target, checksumText, await sha256(executable));
};
const stageNative = async (state, nativeRoot, commit) => {
  if (!nativeRoot) {
    for (const target of state.matrix.targets) await verifyNativeSource(target);
    return { directories: state.matrix.targets.map((target) => path.join(root, 'packages/pi/platforms', target.id)), cleanup: async () => {} };
  }
  const temporary = await mkdtemp(path.join(tmpdir(), 'ast-sgrep-pi-pack-'));
  await mkdir(path.join(temporary, 'platforms'), { recursive: true });
  await mkdir(path.join(temporary, 'release'), { recursive: true });
  await copyFile(path.join(root, 'packages/pi/platforms/prepack-verify.mjs'), path.join(temporary, 'platforms/prepack-verify.mjs'));
  await copyFile(path.join(root, 'packages/pi/release-contract.json'), path.join(temporary, 'release-contract.json'));
  await copyFile(path.join(root, 'packages/pi/release/targets.json'), path.join(temporary, 'release/targets.json'));
  for (const target of state.matrix.targets) {
    const source = path.resolve(nativeRoot, target.id);
    run(process.execPath, ['packages/pi/scripts/release-artifact.mjs', 'verify', '--target', target.id, '--input', source]);
    const metadata = await readJson(path.join(source, 'artifact-metadata.json'));
    if (commit && metadata.commit !== commit.toLowerCase()) fail('ASGREP_RELEASE_COMMIT_SKEW', `${target.package} was built from ${metadata.commit}, expected ${commit}`);
    const destination = path.join(temporary, 'platforms', target.id);
    await mkdir(destination, { recursive: true });
    for (const file of ['package.json', 'LICENSE']) await copyFile(path.join(root, 'packages/pi/platforms', target.id, file), path.join(destination, file));
    await copyFile(path.join(source, target.executable), path.join(destination, target.executable));
    await copyFile(path.join(source, 'SHA256SUMS'), path.join(destination, 'checksum.sha256'));
  }
  return {
    directories: state.matrix.targets.map((target) => path.join(temporary, 'platforms', target.id)),
    cleanup: () => rm(temporary, { recursive: true, force: true })
  };
};
const pack = async () => {
  const state = await load();
  validateAlignment(state);
  run(process.execPath, ['packages/pi/scripts/check-contract.mjs']);
  run(process.execPath, ['packages/pi/scripts/check-native-workflow.mjs']);
  const output = path.resolve(option('output', path.join(root, 'dist/pi-release')));
  const existing = await readdir(output).catch(() => []);
  if (existing.length) fail('ASGREP_RELEASE_OUTPUT_NOT_EMPTY', `${output} must be empty; refusing to overwrite release evidence`);
  await mkdir(output, { recursive: true });
  const staged = await stageNative(state, option('native-root'), option('commit'));
  const directories = [...staged.directories, path.join(root, 'packages/pi/launcher'), path.join(root, 'packages/pi/extension')];
  const artifacts = [];
  try {
    for (const directory of directories) artifacts.push(inspectPackResult(state, JSON.parse(run('npm', ['pack', directory, '--pack-destination', output, '--json']))));
  } finally {
    await staged.cleanup();
  }
  if (artifacts.map((artifact) => artifact.name).join(',') !== packageOrder(state).join(',')) fail('ASGREP_RELEASE_ORDER', 'package order changed');
  for (const artifact of artifacts) artifact.sha256 = await sha256(path.join(output, artifact.filename));
  const manifest = { schemaVersion: 1, version: state.version, tag: state.contract.canonicalVersion.tag, commit: option('commit', null), packageOrder: packageOrder(state), artifacts };
  await writeFile(path.join(output, 'release-manifest.json'), canonical(manifest));
  console.log(`[pi-release] packed ${artifacts.length} artifacts in order: ${manifest.packageOrder.join(' -> ')}`);
  console.log('[pi-release] publication: disabled (npm pack only)');
};
const verify = async (directoryOption) => {
  const state = await load();
  validateAlignment(state);
  const directory = path.resolve(directoryOption ?? option('artifacts', path.join(root, 'dist/pi-release')));
  const manifestPath = path.join(directory, 'release-manifest.json');
  const text = await readFile(manifestPath, 'utf8');
  const manifest = JSON.parse(text);
  if (text !== canonical(manifest) || manifest.schemaVersion !== 1 || manifest.version !== state.version || manifest.tag !== state.contract.canonicalVersion.tag) fail('ASGREP_RELEASE_MANIFEST', 'release manifest is non-canonical or version-skewed');
  if (JSON.stringify(manifest.packageOrder) !== JSON.stringify(packageOrder(state)) || manifest.artifacts.length !== 7) fail('ASGREP_RELEASE_COMPLETENESS', 'release manifest must contain the exact seven-package family in canonical order');
  const entries = (await readdir(directory)).filter((entry) => entry !== 'publish-receipt.json').sort();
  const expected = ['release-manifest.json', ...manifest.artifacts.map((artifact) => artifact.filename)].sort();
  if (JSON.stringify(entries) !== JSON.stringify(expected)) fail('ASGREP_RELEASE_COMPLETENESS', `artifact directory differs from manifest: ${entries.join(', ')}`);
  for (let index = 0; index < manifest.artifacts.length; index += 1) {
    const artifact = manifest.artifacts[index];
    if (artifact.name !== manifest.packageOrder[index] || artifact.version !== state.version || artifact.layer !== classify(state, artifact.name)) fail('ASGREP_RELEASE_ORDER', `${artifact.name} is out of order or version-skewed`);
    validateFiles(state, artifact);
    if (!/^[a-f0-9]{64}$/u.test(artifact.sha256 ?? '') || artifact.sha256 !== await sha256(path.join(directory, artifact.filename))) fail('ASGREP_RELEASE_CHECKSUM_MISMATCH', artifact.filename);
    console.log(`[pi-release] artifact ${index + 1}/${manifest.artifacts.length}: ${artifact.name}@${artifact.version} ${artifact.filename} sha256=${artifact.sha256} files=${artifact.files.length}`);
  }
  console.log(`[pi-release] verified ${manifest.artifacts.length} immutable artifacts at ${manifest.version}`);
  return { state, directory, manifest };
};
const registryVersions = async (state, snapshotPath) => {
  if (snapshotPath) return await readJson(path.resolve(snapshotPath));
  const observed = {};
  for (const name of packageOrder(state)) {
    const spec = `${name}@${state.version}`;
    const result = spawnSync('npm', ['view', spec, 'version', '--json'], { cwd: root, encoding: 'utf8', windowsHide: true });
    if (result.status === 0) observed[spec] = JSON.parse(result.stdout || 'null');
    else if (/E404|404 Not Found|is not in this registry/u.test(result.stderr + result.stdout)) observed[spec] = null;
    else fail('ASGREP_RELEASE_REGISTRY', `could not establish immutability for ${spec}: ${(result.stderr || result.stdout).trim()}`);
  }
  return observed;
};
const gateState = (state, input, observed) => {
  if (!input.clean) fail('ASGREP_RELEASE_DIRTY', 'release checkout must be clean');
  if (input.refType !== 'tag' || input.tag !== state.contract.canonicalVersion.tag) fail('ASGREP_RELEASE_TAG_VERSION', `expected official tag ${state.contract.canonicalVersion.tag}`);
  if (!/^[a-f0-9]{40}$/u.test(input.commit) || input.tagCommit !== input.commit) fail('ASGREP_RELEASE_TAG_COMMIT', 'tag, checkout, and workflow commit must be identical');
  for (const name of packageOrder(state)) {
    const spec = `${name}@${state.version}`;
    if (observed[spec] !== null) fail('ASGREP_RELEASE_DUPLICATE_VERSION', `${spec} already exists; immutable versions are never overwritten`);
  }
};
const gate = async () => {
  const state = await load();
  validateAlignment(state);
  const tag = process.env.GITHUB_REF_NAME ?? option('tag');
  const commit = (process.env.GITHUB_SHA ?? option('commit', '')).toLowerCase();
  const refType = process.env.GITHUB_REF_TYPE ?? option('ref-type', '');
  const clean = run('git', ['status', '--porcelain']).trim() === '';
  const tagType = run('git', ['cat-file', '-t', `refs/tags/${tag}`]).trim();
  if (tagType !== 'tag') fail('ASGREP_RELEASE_UNSIGNED_TAG', 'official release requires an annotated signed tag');
  run('git', ['verify-tag', tag]);
  const tagCommit = run('git', ['rev-list', '-n', '1', tag]).trim().toLowerCase();
  const observed = await registryVersions(state, option('registry-snapshot'));
  gateState(state, { clean, refType, tag, commit, tagCommit }, observed);
  console.log(`[pi-release] gate accepted signed ${tag} at ${commit}; all ${packageOrder(state).length} versions are unpublished`);
};
const validatePublishContext = (state, manifest, environment = process.env) => {
  if (environment.GITHUB_ACTIONS !== 'true' || !environment.ACTIONS_ID_TOKEN_REQUEST_URL) fail('ASGREP_RELEASE_OIDC_REQUIRED', 'publication is only allowed from GitHub Actions OIDC');
  if (environment.ASGREP_NPM_PROTECTED_ENVIRONMENT !== 'npm-production') fail('ASGREP_RELEASE_PROTECTED_ENVIRONMENT', 'npm-production approval marker is required');
  if (environment.GITHUB_REF_TYPE !== 'tag' || environment.GITHUB_REF_NAME !== state.contract.canonicalVersion.tag) fail('ASGREP_RELEASE_TAG_VERSION', 'publication context is not the canonical official tag');
  if (environment.GITHUB_SHA?.toLowerCase() !== manifest.commit?.toLowerCase()) fail('ASGREP_RELEASE_TAG_COMMIT', 'preserved artifacts do not match the workflow commit');
};
const publish = async () => {
  const { state, directory, manifest } = await verify();
  validatePublishContext(state, manifest);
  const layer = option('layer');
  if (!['native', 'launcher', 'extension'].includes(layer)) fail('ASGREP_RELEASE_LAYER', 'layer must be native, launcher, or extension');
  const receiptPath = path.join(directory, 'publish-receipt.json');
  const receipt = await readJson(receiptPath).catch(() => ({ schemaVersion: 1, version: manifest.version, published: [] }));
  const expectedPrior = layer === 'native' ? [] : layer === 'launcher' ? manifest.artifacts.filter((item) => item.layer === 'native').map((item) => item.name) : manifest.artifacts.filter((item) => item.layer !== 'extension').map((item) => item.name);
  if (JSON.stringify(receipt.published) !== JSON.stringify(expectedPrior)) fail('ASGREP_RELEASE_PUBLISH_ORDER', `${layer} cannot publish after [${receipt.published.join(', ')}]`);
  const selected = manifest.artifacts.filter((artifact) => artifact.layer === layer);
  const publishDelayMs = Math.max(0, Number(process.env.ASGREP_PUBLISH_DELAY_MS ?? '30000'));
  for (const artifact of selected) {
    if (publishDelayMs > 0) await delay(publishDelayMs);
    run('npm', ['publish', path.join(directory, artifact.filename), '--access', 'public', '--provenance'], { stdio: 'inherit' });
    receipt.published.push(artifact.name);
    await writeFile(receiptPath, canonical(receipt));
  }
  console.log(`[pi-release] published ${layer}: ${selected.map((item) => item.name).join(' -> ')}`);
};
const fixtureNative = async () => {
  const state = await load();
  validateAlignment(state);
  const output = path.resolve(option('output'));
  if ((await readdir(output).catch(() => [])).length) fail('ASGREP_RELEASE_OUTPUT_NOT_EMPTY', `${output} must be empty`);
  await mkdir(output, { recursive: true });
  const commit = 'f'.repeat(40);
  for (const target of state.matrix.targets) {
    const binary = path.join(output, `${target.id}.fixture`);
    await writeFile(binary, `contract-only target-shaped fixture for ${target.package}@${state.version}\n`);
    if (target.os !== 'win32') await chmod(binary, 0o755);
    run(process.execPath, ['packages/pi/scripts/release-artifact.mjs', 'prepare', '--target', target.id, '--binary', binary, '--output', path.join(output, target.id), '--commit', commit]);
    run(process.execPath, ['packages/pi/scripts/release-artifact.mjs', 'verify', '--target', target.id, '--input', path.join(output, target.id)]);
  }
  console.log(`[pi-release] created and verified ${state.matrix.targets.length} disposable target-shaped fixtures at commit ${commit}`);
  console.log('[pi-release] fixture mode is structural pack evidence only; fixtures are never publishable native binaries');
};
const selfTest = async () => {
  const state = await load();
  validateAlignment(state);
  const commit = 'a'.repeat(40);
  const empty = Object.fromEntries(packageOrder(state).map((name) => [`${name}@${state.version}`, null]));
  gateState(state, { clean: true, refType: 'tag', tag: state.contract.canonicalVersion.tag, commit, tagCommit: commit }, empty);
  const rejected = [];
  const expect = (label, callback) => { try { callback(); } catch (error) { rejected.push(`${label}=${error.message.split(':')[0]}`); return; } fail('ASGREP_RELEASE_SELF_TEST', `${label} was accepted`); };
  expect('dirty', () => gateState(state, { clean: false, refType: 'tag', tag: state.contract.canonicalVersion.tag, commit, tagCommit: commit }, empty));
  expect('wrong-tag', () => gateState(state, { clean: true, refType: 'tag', tag: 'v0.0.0', commit, tagCommit: commit }, empty));
  expect('wrong-commit', () => gateState(state, { clean: true, refType: 'tag', tag: state.contract.canonicalVersion.tag, commit, tagCommit: 'b'.repeat(40) }, empty));
  expect('duplicate', () => gateState(state, { clean: true, refType: 'tag', tag: state.contract.canonicalVersion.tag, commit, tagCommit: commit }, { ...empty, [`${state.launcher.name}@${state.version}`]: state.version }));
  expect('version-skew', () => validateAlignment({ ...state, launcher: { ...state.launcher, version: '0.0.0' } }));
  expect('missing-checksum', () => validateChecksumRecord(state.matrix.targets[0], null, '0'.repeat(64)));
  expect('checksum-mismatch', () => validateChecksumRecord(state.matrix.targets[0], `${'1'.repeat(64)}  asgrep`, '0'.repeat(64)));
  expect('local-publish', () => validatePublishContext(state, { commit }, {}));
  console.log(`[pi-release] gate self-test accepted canonical input and rejected ${rejected.join(', ')}`);
  console.log(`[pi-release] publish order: ${packageOrder(state).join(' -> ')}`);
  console.log('[pi-release] publication: disabled (self-test only)');
};

const command = process.argv[2];
if (command === 'pack') await pack();
else if (command === 'verify') await verify();
else if (command === 'gate') await gate();
else if (command === 'publish') await publish();
else if (command === 'fixture-native') await fixtureNative();
else if (command === 'self-test') await selfTest();
else fail('ASGREP_RELEASE_USAGE', 'pack | verify | gate | publish | fixture-native | self-test');
