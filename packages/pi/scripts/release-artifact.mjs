import { createHash } from 'node:crypto';
import { chmod, copyFile, mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const matrixPath = path.join(root, 'packages/pi/release/targets.json');
const contractPath = path.join(root, 'packages/pi/release-contract.json');
const cliBuildFlags = ['--locked', '--release', '-p', 'ast-sgrep-cli', '--bin', 'asgrep', '--no-default-features'];
const napiBuildFlags = ['--locked', '--release', '-p', 'ast-sgrep-codemode-napi'];
const fail = (message) => { throw new Error(message); };
const readJson = async (file) => JSON.parse(await readFile(file, 'utf8'));
const sha256 = async (file) => createHash('sha256').update(await readFile(file)).digest('hex');
const option = (name) => {
  const index = process.argv.indexOf('--' + name);
  if (index < 0 || !process.argv[index + 1]) fail('missing --' + name);
  return process.argv[index + 1];
};
const load = async () => {
  const [matrix, contract] = await Promise.all([readJson(matrixPath), readJson(contractPath)]);
  if (matrix.schemaVersion !== 1 || matrix.artifactSchemaVersion !== 2 || typeof matrix.napiAddon !== 'string' || !Array.isArray(matrix.targets)) {
    fail('unsupported target matrix schema');
  }
  return { matrix, contract };
};
const selectTarget = (matrix, id) => matrix.targets.find((target) => target.id === id) ?? fail('unsupported target: ' + id);
const canonicalMetadata = (value) => JSON.stringify(value, null, 2) + '\n';
const formatSha256Sums = (entries) => entries.map(([digest, name]) => digest + '  ' + name).join('\n') + '\n';
const parseSha256Sums = (text) => {
  const map = new Map();
  for (const line of text.replace(/\r\n/gu, '\n').split('\n')) {
    if (!line) continue;
    const match = line.match(/^([a-f0-9]{64})  (.+)$/u);
    if (!match) fail('SHA256SUMS line is invalid: ' + line);
    if (map.has(match[2])) fail('SHA256SUMS has a duplicate entry for ' + match[2]);
    map.set(match[2], match[1]);
  }
  return map;
};
const requireNonEmptyFile = async (file, label) => {
  const source = await stat(file).catch(() => fail(label + ' does not exist: ' + file));
  if (!source.isFile()) fail(label + ' is not a file: ' + file);
  if (source.size === 0) fail(label + ' is empty: ' + file);
  return source;
};

const command = process.argv[2];
const { matrix, contract } = await load();
if (command === 'matrix') {
  const include = matrix.targets.map(({ id, runner, rustTarget, package: packageName, executable, os, cpu }) => ({
    id, runner, rustTarget, package: packageName, executable, os, cpu, napiAddon: matrix.napiAddon
  }));
  process.stdout.write(JSON.stringify({ include }));
} else if (command === 'prepare') {
  const target = selectTarget(matrix, option('target'));
  const binary = path.resolve(option('binary'));
  const napi = path.resolve(option('napi'));
  const output = path.resolve(option('output'));
  const commit = option('commit').toLowerCase();
  if (!/^[0-9a-f]{40}$/.test(commit)) fail('commit must be a full 40-character hexadecimal SHA');
  await requireNonEmptyFile(binary, 'binary');
  await requireNonEmptyFile(napi, 'napi addon');
  await mkdir(output, { recursive: true });
  const executable = path.join(output, target.executable);
  const addon = path.join(output, matrix.napiAddon);
  await copyFile(binary, executable);
  await copyFile(napi, addon);
  if (target.os !== 'win32') await chmod(executable, 0o755);
  const executableChecksum = await sha256(executable);
  const napiChecksum = await sha256(addon);
  const metadata = {
    schemaVersion: matrix.artifactSchemaVersion,
    artifact: target.package + '-v' + contract.canonicalVersion.version,
    package: target.package,
    version: contract.canonicalVersion.version,
    commit,
    target: target.rustTarget,
    npm: { os: [target.os], cpu: [target.cpu], libc: target.libc ? [target.libc] : [] },
    executable: target.executable,
    napiAddon: matrix.napiAddon,
    build: { profile: 'release', cliFlags: cliBuildFlags, napiFlags: napiBuildFlags },
    checksums: {
      algorithm: 'sha256',
      files: {
        [target.executable]: executableChecksum,
        [matrix.napiAddon]: napiChecksum
      }
    }
  };
  await writeFile(path.join(output, 'artifact-metadata.json'), canonicalMetadata(metadata));
  await writeFile(path.join(output, 'SHA256SUMS'), formatSha256Sums([
    [executableChecksum, target.executable],
    [napiChecksum, matrix.napiAddon]
  ]));
  console.log(metadata.artifact);
} else if (command === 'verify') {
  const target = selectTarget(matrix, option('target'));
  const input = path.resolve(option('input'));
  const entries = (await readdir(input)).sort();
  const expectedEntries = ['SHA256SUMS', 'artifact-metadata.json', target.executable, matrix.napiAddon].sort();
  if (JSON.stringify(entries) !== JSON.stringify(expectedEntries)) {
    fail('artifact must contain CLI + NAPI addon plus artifact-metadata.json and SHA256SUMS; found: ' + entries.join(', '));
  }
  const executableFile = path.join(input, target.executable);
  const napiFile = path.join(input, matrix.napiAddon);
  await requireNonEmptyFile(executableFile, 'artifact executable');
  await requireNonEmptyFile(napiFile, 'artifact napi addon');
  const metadataFile = path.join(input, 'artifact-metadata.json');
  const metadataText = await readFile(metadataFile, 'utf8');
  const metadata = JSON.parse(metadataText);
  if (metadataText !== canonicalMetadata(metadata)) fail('artifact metadata is not deterministically formatted');
  const platform = contract.packages.platforms.find((item) => item.target === target.rustTarget);
  if (!platform) fail('target is absent from release contract: ' + target.rustTarget);
  const expected = {
    schemaVersion: matrix.artifactSchemaVersion,
    artifact: target.package + '-v' + contract.canonicalVersion.version,
    package: target.package,
    version: contract.canonicalVersion.version,
    commit: metadata.commit,
    target: target.rustTarget,
    npm: { os: [target.os], cpu: [target.cpu], libc: target.libc ? [target.libc] : [] },
    executable: target.executable,
    napiAddon: matrix.napiAddon,
    build: { profile: 'release', cliFlags: cliBuildFlags, napiFlags: napiBuildFlags },
    checksums: metadata.checksums
  };
  if (!/^[0-9a-f]{40}$/.test(metadata.commit ?? '')) fail('metadata commit is not a full hexadecimal SHA');
  if (JSON.stringify(metadata) !== JSON.stringify(expected)) fail('metadata does not match the authoritative target/version/build contract');
  if (platform.name !== target.package || platform.executable !== target.executable || platform.optionalDependencyVersion !== metadata.version) {
    fail('release contract package metadata does not match target matrix');
  }
  const executableChecksum = await sha256(executableFile);
  const napiChecksum = await sha256(napiFile);
  if (metadata.checksums?.algorithm !== 'sha256') fail('checksum algorithm must be sha256');
  if (metadata.checksums?.files?.[target.executable] !== executableChecksum) fail('executable checksum does not match metadata');
  if (metadata.checksums?.files?.[matrix.napiAddon] !== napiChecksum) fail('napi addon checksum does not match metadata');
  const checksumText = await readFile(path.join(input, 'SHA256SUMS'), 'utf8');
  const expectedSums = formatSha256Sums([
    [executableChecksum, target.executable],
    [napiChecksum, matrix.napiAddon]
  ]);
  if (checksumText !== expectedSums) fail('SHA256SUMS does not match staged artifacts');
  const parsed = parseSha256Sums(checksumText);
  if (parsed.get(target.executable) !== executableChecksum || parsed.get(matrix.napiAddon) !== napiChecksum || parsed.size !== 2) {
    fail('SHA256SUMS parse round-trip failed');
  }
  console.log('Verified ' + metadata.artifact + ' (' + metadata.target + ')');
} else {
  fail('usage: release-artifact.mjs matrix | prepare --target ID --binary PATH --napi PATH --output DIR --commit SHA | verify --target ID --input DIR');
}
