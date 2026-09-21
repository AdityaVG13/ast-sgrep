import { createHash } from 'node:crypto';
import { chmod, copyFile, mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { assertLauncherRangeResolves, fetchPublishedVersions } from './check-launcher-resolves.mjs';

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
  if (matrix.napiAddon !== 'ast-sgrep-codemode.node') fail('ASGREP_RELEASE_TARGETS', 'napiAddon must be ast-sgrep-codemode.node');
  if (new Set(packageOrder(state)).size !== 7) fail('ASGREP_RELEASE_PACKAGE_DUPLICATE', 'release package names must be unique');
  if (contract.canonicalVersion.tag !== `v${version}`) fail('ASGREP_RELEASE_TAG_VERSION', 'canonical tag does not match canonical version');
  if (launcher.version !== version) fail('ASGREP_RELEASE_VERSION_SKEW', 'launcher version must exactly match the canonical version');
  validateExtensionAlignment(state);
  if (contract.compatibility?.layers?.machineSchema?.version !== '1.0.0') fail('ASGREP_RELEASE_PROTOCOL', 'machine protocol version changed without a release-contract update');
  for (let index = 0; index < matrix.targets.length; index += 1) {
    const target = matrix.targets[index];
    const manifest = platforms[index];
    const dependencyVersion = launcher.optionalDependencies?.[target.package];
    const contractPlatform = contract.packages.platforms[index];
    if (manifest.name !== target.package || manifest.version !== version || dependencyVersion !== version || contractPlatform?.optionalDependencyVersion !== version) fail('ASGREP_RELEASE_VERSION_SKEW', `${target.package} is not exactly aligned to ${version}`);
    if (JSON.stringify(manifest.os) !== JSON.stringify([target.os]) || JSON.stringify(manifest.cpu) !== JSON.stringify([target.cpu]) || JSON.stringify(manifest.libc ?? []) !== JSON.stringify(target.libc ? [target.libc] : [])) fail('ASGREP_RELEASE_PLATFORM_SKEW', `${target.package} platform selectors do not match the target matrix`);
    const expectedFiles = [target.executable, matrix.napiAddon, 'checksum.sha256', 'LICENSE'].sort();
    if (JSON.stringify([...(manifest.files ?? [])].sort()) !== JSON.stringify(expectedFiles)) fail('ASGREP_RELEASE_PLATFORM_SKEW', `${target.package} files inventory must include CLI and NAPI addon`);
  }
};
const classify = (state, name) => state.matrix.targets.some((target) => target.package === name) ? 'native' : name === state.launcher.name ? 'launcher' : name === state.extension.name ? 'extension' : fail('ASGREP_RELEASE_UNKNOWN_PACKAGE', name);
// Extension lane: the pi-extension is severed from the canonical family and
// publishes independently under signed pi-v<version> tags. --lane extension
// switches pack/verify/gate to a single-artifact manifest at
// packages.extension.version; inside a family release the extension artifact
// still rides its own version line (idempotent skip when already live).
const lane = () => option('lane', 'family');
const extensionSpec = (state) => state.contract.packages?.extension ?? {};
const extensionTag = (state) => `${extensionSpec(state).tagPrefix}${state.extension.version}`;
const expectedArtifactVersion = (state, name) => name === state.extension.name ? extensionSpec(state).version : state.version;
const validateExtensionAlignment = (state) => {
  const spec = extensionSpec(state);
  if (spec.independent !== true || spec.tagPrefix !== 'pi-v') fail('ASGREP_RELEASE_EXTENSION_POLICY', 'extension lane requires independent versioning with the pi-v tag prefix in the contract');
  if (state.extension.name !== spec.name || spec.directory !== 'packages/pi/extension') fail('ASGREP_RELEASE_EXTENSION_POLICY', 'extension package identity drifts from the contract');
  if (state.extension.version !== spec.version) fail('ASGREP_RELEASE_VERSION_SKEW', `extension manifest version ${state.extension.version} does not match contract packages.extension.version ${spec.version}`);
  if (state.extension.dependencies?.[state.launcher.name] !== spec.launcherRange) fail('ASGREP_RELEASE_VERSION_SKEW', `extension must depend on ast-sgrep "${spec.launcherRange}"`);
  if (!/^>=\d+\.\d+\.\d+ <\d+$/u.test(spec.launcherRange ?? '')) fail('ASGREP_RELEASE_EXTENSION_POLICY', 'launcherRange must be a bounded >=<floor> <<major> range');
};
const parseSha256Sums = (text) => {
  const map = new Map();
  for (const line of text.replace(/\r\n/gu, '\n').split('\n')) {
    if (!line) continue;
    const match = line.match(/^([a-f0-9]{64})  (.+)$/u);
    if (!match || map.has(match[2])) return null;
    map.set(match[2], match[1]);
  }
  return map;
};
const validateFiles = (state, artifact) => {
  const files = artifact.files.map((file) => file.path).sort();
  const nativeTarget = state.matrix.targets.find((target) => target.package === artifact.name);
  const required = artifact.layer === 'native'
    ? ['LICENSE', 'checksum.sha256', nativeTarget.executable, state.matrix.napiAddon, 'package.json']
    : artifact.layer === 'launcher'
      ? ['LICENSE', 'README.md', 'bin/asgrep.js', 'package.json', 'src/index.d.ts', 'src/index.js']
      : ['LICENSE', 'README.md', 'assets/preview.png', 'dist/codemode/index.d.ts', 'dist/codemode/index.js', 'dist/codemode/native.d.ts', 'dist/codemode/native.js', 'dist/codemode/guest-worker.mjs', 'dist/index.d.ts', 'dist/index.js', 'dist/ui/present.d.ts', 'dist/ui/present.js', 'dist/runtime/runtime.d.ts', 'dist/runtime/runtime.js', 'dist/runtime/types.d.ts', 'dist/runtime/types.js', 'dist/runtime/config.d.ts', 'dist/runtime/config.js', 'dist/runtime/freshness.d.ts', 'dist/runtime/freshness.js', 'dist/runtime/index-health.d.ts', 'dist/runtime/index-health.js', 'dist/host/results.d.ts', 'dist/host/results.js', 'dist/host/tools.d.ts', 'dist/host/tools.js', 'dist/host/commands.d.ts', 'dist/host/commands.js', 'native/README.md', 'package.json'];
  for (const entry of required) if (!files.includes(entry)) fail('ASGREP_RELEASE_CONTENT_MISSING', `${artifact.name} is missing ${entry}`);
  for (const entry of files) if (/(^|\/)(test|node_modules)(\/|$)/u.test(entry) || /\.(rs|toml)$/u.test(entry)) fail('ASGREP_RELEASE_CONTENT_FORBIDDEN', `${artifact.name} unexpectedly contains ${entry}`);
};
const inspectPackResult = (state, result) => {
  if (!Array.isArray(result) || result.length !== 1) fail('ASGREP_RELEASE_PACK_RESULT', 'npm pack must produce exactly one artifact');
  const item = result[0];
  if (!packageOrder(state).includes(item.name) || item.version !== expectedArtifactVersion(state, item.name) || !item.filename || !Array.isArray(item.files)) fail('ASGREP_RELEASE_PACK_METADATA', 'npm pack metadata is incomplete or skewed');
  const artifact = { name: item.name, version: item.version, layer: classify(state, item.name), filename: item.filename, integrity: item.integrity, shasum: item.shasum, files: item.files.map(({ path: filePath, size, mode }) => ({ path: filePath, size, mode })) };
  validateFiles(state, artifact);
  return artifact;
};
const validateChecksumRecord = (target, napiAddon, checksumText, digests) => {
  if (checksumText === null) fail('ASGREP_RELEASE_CHECKSUM_MISSING', target.package);
  const parsed = parseSha256Sums(checksumText);
  if (!parsed || parsed.size !== 2 || parsed.get(target.executable) !== digests.executable || parsed.get(napiAddon) !== digests.napi) {
    fail('ASGREP_RELEASE_CHECKSUM_MISMATCH', target.package);
  }
};
const verifyNativeSource = async (target, napiAddon) => {
  const directory = path.join(root, 'packages/pi/platforms', target.id);
  const executable = path.join(directory, target.executable);
  const addon = path.join(directory, napiAddon);
  const checksumFile = path.join(directory, 'checksum.sha256');
  const executableStat = await stat(executable).catch(() => fail('ASGREP_RELEASE_EXECUTABLE_MISSING', target.package));
  if (!executableStat.isFile() || executableStat.size === 0) fail('ASGREP_RELEASE_EXECUTABLE_MISSING', target.package);
  const addonStat = await stat(addon).catch(() => fail('ASGREP_RELEASE_EXECUTABLE_MISSING', `${target.package} napi`));
  if (!addonStat.isFile() || addonStat.size === 0) fail('ASGREP_RELEASE_EXECUTABLE_MISSING', `${target.package} napi`);
  const checksumText = await readFile(checksumFile, 'utf8').catch(() => null);
  validateChecksumRecord(target, napiAddon, checksumText, {
    executable: await sha256(executable),
    napi: await sha256(addon)
  });
};
const stageNative = async (state, nativeRoot, commit) => {
  if (!nativeRoot) {
    for (const target of state.matrix.targets) await verifyNativeSource(target, state.matrix.napiAddon);
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
    await copyFile(path.join(source, state.matrix.napiAddon), path.join(destination, state.matrix.napiAddon));
    await copyFile(path.join(source, 'SHA256SUMS'), path.join(destination, 'checksum.sha256'));
  }
  return {
    directories: state.matrix.targets.map((target) => path.join(temporary, 'platforms', target.id)),
    cleanup: () => rm(temporary, { recursive: true, force: true })
  };
};
const packExtension = async (state) => {
  validateExtensionAlignment(state);
  run(process.execPath, ['packages/pi/scripts/check-contract.mjs']);
  const output = path.resolve(option('output', path.join(root, 'dist/pi-extension-release')));
  const existing = await readdir(output).catch(() => []);
  if (existing.length) fail('ASGREP_RELEASE_OUTPUT_NOT_EMPTY', `${output} must be empty; refusing to overwrite release evidence`);
  await mkdir(output, { recursive: true });
  const artifacts = [inspectPackResult(state, JSON.parse(run('npm', ['pack', path.join(root, 'packages/pi/extension'), '--pack-destination', output, '--json'])))];
  for (const artifact of artifacts) artifact.sha256 = await sha256(path.join(output, artifact.filename));
  const manifest = { schemaVersion: 1, lane: 'extension', version: state.extension.version, tag: extensionTag(state), commit: option('commit', null), packageOrder: [state.extension.name], artifacts };
  await writeFile(path.join(output, 'release-manifest.json'), canonical(manifest));
  console.log(`[pi-release] packed extension artifact ${state.extension.name}@${state.extension.version} for ${manifest.tag}`);
  console.log('[pi-release] publication: disabled (npm pack only)');
};
const pack = async () => {
  const state = await load();
  if (lane() === 'extension') return packExtension(state);
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
  if (text !== canonical(manifest) || manifest.schemaVersion !== 1) fail('ASGREP_RELEASE_MANIFEST', 'release manifest is non-canonical');
  const extensionLane = manifest.lane === 'extension';
  if (extensionLane) {
    if (manifest.version !== state.extension.version || manifest.tag !== extensionTag(state)) fail('ASGREP_RELEASE_MANIFEST', 'extension manifest is version-skewed');
    if (JSON.stringify(manifest.packageOrder) !== JSON.stringify([state.extension.name]) || manifest.artifacts.length !== 1) fail('ASGREP_RELEASE_COMPLETENESS', 'extension manifest must contain exactly the pi-ast-sgrep artifact');
  } else {
    if (manifest.version !== state.version || manifest.tag !== state.contract.canonicalVersion.tag) fail('ASGREP_RELEASE_MANIFEST', 'release manifest is version-skewed');
    if (JSON.stringify(manifest.packageOrder) !== JSON.stringify(packageOrder(state)) || manifest.artifacts.length !== 7) fail('ASGREP_RELEASE_COMPLETENESS', 'release manifest must contain the exact seven-package family in canonical order');
  }
  const entries = (await readdir(directory)).filter((entry) => entry !== 'publish-receipt.json').sort();
  const expected = ['release-manifest.json', ...manifest.artifacts.map((artifact) => artifact.filename)].sort();
  if (JSON.stringify(entries) !== JSON.stringify(expected)) fail('ASGREP_RELEASE_COMPLETENESS', `artifact directory differs from manifest: ${entries.join(', ')}`);
  for (let index = 0; index < manifest.artifacts.length; index += 1) {
    const artifact = manifest.artifacts[index];
    if (artifact.name !== manifest.packageOrder[index] || artifact.version !== expectedArtifactVersion(state, artifact.name) || artifact.layer !== classify(state, artifact.name)) fail('ASGREP_RELEASE_ORDER', `${artifact.name} is out of order or version-skewed`);
    validateFiles(state, artifact);
    if (!/^[a-f0-9]{64}$/u.test(artifact.sha256 ?? '') || artifact.sha256 !== await sha256(path.join(directory, artifact.filename))) fail('ASGREP_RELEASE_CHECKSUM_MISMATCH', artifact.filename);
    console.log(`[pi-release] artifact ${index + 1}/${manifest.artifacts.length}: ${artifact.name}@${artifact.version} ${artifact.filename} sha256=${artifact.sha256} files=${artifact.files.length}`);
  }
  console.log(`[pi-release] verified ${manifest.artifacts.length} immutable artifacts at ${manifest.version}`);
  return { state, directory, manifest };
};
const registryVersions = async (state, snapshotPath, specs) => {
  if (snapshotPath) return await readJson(path.resolve(snapshotPath));
  const observed = {};
  const names = specs ?? packageOrder(state).map((name) => `${name}@${expectedArtifactVersion(state, name)}`);
  for (const spec of names) {
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
  const names = packageOrder(state);
  const live = names.filter((name) => observed[`${name}@${expectedArtifactVersion(state, name)}`] !== null);
  const pending = names.filter((name) => !live.includes(name));
  if (live.length === names.length) fail('ASGREP_RELEASE_DUPLICATE_VERSION', `all ${names.length} packages already exist at ${state.version}; bump the canonical version for a new release`);
  return { live, pending };
};
const gateExtensionState = (state, input, observed) => {
  if (!input.clean) fail('ASGREP_RELEASE_DIRTY', 'release checkout must be clean');
  if (input.refType !== 'tag' || input.tag !== extensionTag(state)) fail('ASGREP_RELEASE_TAG_VERSION', `expected extension tag ${extensionTag(state)}`);
  if (!/^[a-f0-9]{40}$/u.test(input.commit) || input.tagCommit !== input.commit) fail('ASGREP_RELEASE_TAG_COMMIT', 'tag, checkout, and workflow commit must be identical');
  const spec = `${state.extension.name}@${state.extension.version}`;
  if (observed[spec] != null) fail('ASGREP_RELEASE_DUPLICATE_VERSION', `${spec} already exists; bump packages.extension.version for a new release`);
};
const gateExtension = async () => {
  const state = await load();
  validateExtensionAlignment(state);
  const tag = process.env.GITHUB_REF_NAME ?? option('tag');
  const commit = (process.env.GITHUB_SHA ?? option('commit', '')).toLowerCase();
  const refType = process.env.GITHUB_REF_TYPE ?? option('ref-type', '');
  const clean = run('git', ['status', '--porcelain']).trim() === '';
  const tagType = run('git', ['cat-file', '-t', `refs/tags/${tag}`]).trim();
  if (tagType !== 'tag') fail('ASGREP_RELEASE_UNSIGNED_TAG', 'extension release requires an annotated signed tag');
  run('git', ['verify-tag', tag]);
  const tagCommit = run('git', ['rev-list', '-n', '1', tag]).trim().toLowerCase();
  const spec = `${state.extension.name}@${state.extension.version}`;
  const observed = await registryVersions(state, option('registry-snapshot'), [spec]);
  gateExtensionState(state, { clean, refType, tag, commit, tagCommit }, observed);
  // The launcherRange must resolve to a published launcher: without this, a
  // missing canonical family ships an extension nobody can install (2026-09-19:
  // pi-ast-sgrep@2.2.0 required ast-sgrep@>=2.1.0 <3 with only 2.0.0 live).
  const snapshotPath = option('registry-snapshot');
  let published;
  if (snapshotPath) {
    published = (await readJson(path.resolve(snapshotPath))).launcherVersions;
    if (!Array.isArray(published)) fail('ASGREP_RELEASE_REGISTRY', 'extension registry snapshot must include launcherVersions: [...]');
  } else {
    published = fetchPublishedVersions(state.launcher.name);
  }
  const hits = assertLauncherRangeResolves({ launcher: state.launcher.name, range: extensionSpec(state).launcherRange, extension: spec, canonical: state.version, versions: published });
  console.log(`[pi-release] launcherRange ${extensionSpec(state).launcherRange} resolves: ${hits.map((version) => `${state.launcher.name}@${version}`).join(', ')}`);
  console.log(`[pi-release] gate accepted signed ${tag} at ${commit}; extension ${spec} pending publication`);
};
const gate = async () => {
  const state = await load();
  if (lane() === 'extension') return gateExtension();
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
  const { live, pending } = gateState(state, { clean, refType, tag, commit, tagCommit }, observed);
  console.log(`[pi-release] gate accepted signed ${tag} at ${commit}; publish plan at ${state.version}: ${pending.length} to publish, ${live.length} already live${live.length ? ` (idempotent skip: ${live.join(', ')})` : ''}`);
};
const validatePublishContext = (state, manifest, environment = process.env) => {
  if (environment.GITHUB_ACTIONS !== 'true' || !environment.ACTIONS_ID_TOKEN_REQUEST_URL) fail('ASGREP_RELEASE_OIDC_REQUIRED', 'publication is only allowed from GitHub Actions OIDC');
  if (environment.ASGREP_NPM_PROTECTED_ENVIRONMENT !== 'npm-production') fail('ASGREP_RELEASE_PROTECTED_ENVIRONMENT', 'npm-production approval marker is required');
  const expectedTag = manifest.lane === 'extension' ? manifest.tag : state.contract.canonicalVersion.tag;
  if (environment.GITHUB_REF_TYPE !== 'tag' || environment.GITHUB_REF_NAME !== expectedTag) fail('ASGREP_RELEASE_TAG_VERSION', 'publication context is not the expected official tag');
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
  const observed = await registryVersions(state, undefined, manifest.packageOrder.map((name) => `${name}@${expectedArtifactVersion(state, name)}`));
  const publishDelayMs = Math.max(0, Number(process.env.ASGREP_PUBLISH_DELAY_MS ?? '0'));
  const published = [];
  for (const artifact of selected) {
    if (observed[`${artifact.name}@${expectedArtifactVersion(state, artifact.name)}`] !== null) {
      console.log(`[pi-release] skip ${artifact.name}@${expectedArtifactVersion(state, artifact.name)}: already live (idempotent re-run)`);
    } else {
      if (publishDelayMs > 0) await delay(publishDelayMs);
      // Bootstrap lane (classic token present) publishes WITHOUT --provenance:
      // the registry checks the OIDC/provenance identity even on token-authed
      // PUTs, and the trusted-publisher claims are unresolved (br-ijy), so an
      // attested bootstrap PUT 403s identically to a pure-OIDC PUT. The OIDC
      // lane (no token) always attests. Provenance returns to every lane once
      // br-ijy closes; all pre-2.5.0 releases shipped unattested.
      const args = ['publish', path.join(directory, artifact.filename), '--access', 'public'];
      if (!process.env.NODE_AUTH_TOKEN) args.push('--provenance');
      run('npm', args, { stdio: 'inherit' });
      published.push(artifact.name);
    }
    receipt.published.push(artifact.name);
    await writeFile(receiptPath, canonical(receipt));
  }
  console.log(`[pi-release] ${layer}: newly published [${published.join(', ') || 'none'}]; complete layer is [${selected.map((item) => item.name).join(', ')}]`);
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
    const napi = path.join(output, `${target.id}.napi.fixture`);
    await writeFile(binary, `contract-only target-shaped fixture for ${target.package}@${state.version}\n`);
    await writeFile(napi, `contract-only napi fixture for ${target.package}@${state.version}\n`);
    if (target.os !== 'win32') await chmod(binary, 0o755);
    run(process.execPath, ['packages/pi/scripts/release-artifact.mjs', 'prepare', '--target', target.id, '--binary', binary, '--napi', napi, '--output', path.join(output, target.id), '--commit', commit]);
    run(process.execPath, ['packages/pi/scripts/release-artifact.mjs', 'verify', '--target', target.id, '--input', path.join(output, target.id)]);
  }
  console.log(`[pi-release] created and verified ${state.matrix.targets.length} disposable target-shaped fixtures at commit ${commit}`);
  console.log('[pi-release] fixture mode is structural pack evidence only; fixtures are never publishable native binaries');
};
const selfTest = async () => {
  const state = await load();
  validateAlignment(state);
  const commit = 'a'.repeat(40);
  const specOf = (name) => `${name}@${expectedArtifactVersion(state, name)}`;
  const empty = Object.fromEntries(packageOrder(state).map((name) => [specOf(name), null]));
  const canonicalInput = { clean: true, refType: 'tag', tag: state.contract.canonicalVersion.tag, commit, tagCommit: commit };
  const fresh = gateState(state, canonicalInput, empty);
  if (fresh.pending.length !== 7 || fresh.live.length !== 0) fail('ASGREP_RELEASE_SELF_TEST', 'a fresh version must plan to publish all seven packages');
  const partialObserved = { ...empty, [specOf(state.launcher.name)]: state.launcher.version, [specOf(state.matrix.targets[0].package)]: state.version };
  const partial = gateState(state, canonicalInput, partialObserved);
  if (partial.live.length !== 2 || partial.pending.length !== 5 || partial.pending.includes(state.launcher.name)) fail('ASGREP_RELEASE_SELF_TEST', 'a partial version must skip live packages and re-publish only the remainder');
  const rejected = [];
  const expect = (label, callback) => { try { callback(); } catch (error) { rejected.push(`${label}=${error.message.split(':')[0]}`); return; } fail('ASGREP_RELEASE_SELF_TEST', `${label} was accepted`); };
  expect('dirty', () => gateState(state, { ...canonicalInput, clean: false }, empty));
  expect('wrong-tag', () => gateState(state, { ...canonicalInput, tag: 'v0.0.0' }, empty));
  expect('wrong-commit', () => gateState(state, { ...canonicalInput, tagCommit: 'b'.repeat(40) }, empty));
  expect('fully-published', () => gateState(state, canonicalInput, Object.fromEntries(packageOrder(state).map((name) => [specOf(name), expectedArtifactVersion(state, name)]))));
  expect('version-skew', () => validateAlignment({ ...state, launcher: { ...state.launcher, version: '0.0.0' } }));
  expect('missing-checksum', () => validateChecksumRecord(state.matrix.targets[0], state.matrix.napiAddon, null, { executable: '0'.repeat(64), napi: '0'.repeat(64) }));
  expect('checksum-mismatch', () => validateChecksumRecord(state.matrix.targets[0], state.matrix.napiAddon, `${'1'.repeat(64)}  asgrep\n${'2'.repeat(64)}  ${state.matrix.napiAddon}\n`, { executable: '0'.repeat(64), napi: '0'.repeat(64) }));
  expect('local-publish', () => validatePublishContext(state, { commit }, {}));
  // Extension lane: accepts a fresh signed pi-v<version> tag, rejects skews and duplicates.
  const extInput = { clean: true, refType: 'tag', tag: extensionTag(state), commit, tagCommit: commit };
  const extSpec = `${state.extension.name}@${state.extension.version}`;
  gateExtensionState(state, extInput, { [extSpec]: null });
  expect('extension-version-skew', () => validateExtensionAlignment({ ...state, extension: { ...state.extension, version: '9.9.9' } }));
  expect('extension-dep-skew', () => validateExtensionAlignment({ ...state, extension: { ...state.extension, dependencies: { [state.launcher.name]: '2.0.0' } } }));
  expect('extension-wrong-tag', () => gateExtensionState(state, { ...extInput, tag: state.contract.canonicalVersion.tag }, { [extSpec]: null }));
  expect('extension-duplicate', () => gateExtensionState(state, extInput, { [extSpec]: state.extension.version }));
  // 2026-09-19 incident: the extension floor sat above every published launcher.
  expect('extension-launcher-unresolved', () => assertLauncherRangeResolves({ launcher: state.launcher.name, range: extensionSpec(state).launcherRange, extension: extSpec, canonical: state.version, versions: [] }));
  if (assertLauncherRangeResolves({ launcher: state.launcher.name, range: extensionSpec(state).launcherRange, extension: extSpec, canonical: state.version, versions: [state.version] }).join() !== state.version) fail('ASGREP_RELEASE_SELF_TEST', 'a published canonical launcher must satisfy the extension launcherRange');
  console.log(`[pi-release] gate self-test accepted canonical + extension lane input and rejected ${rejected.join(', ')}`);
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
