import { createHash } from 'node:crypto';
import { chmod, copyFile, mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const matrixPath = path.join(root, 'packages/pi/release/targets.json');
const contractPath = path.join(root, 'packages/pi/release-contract.json');
const buildFlags = ['--locked', '--release', '-p', 'ast-sgrep-cli', '--bin', 'asgrep', '--no-default-features'];
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
  if (matrix.schemaVersion !== 1 || matrix.artifactSchemaVersion !== 1 || !Array.isArray(matrix.targets)) fail('unsupported target matrix schema');
  return { matrix, contract };
};
const selectTarget = (matrix, id) => matrix.targets.find((target) => target.id === id) ?? fail('unsupported target: ' + id);
const canonicalMetadata = (value) => JSON.stringify(value, null, 2) + '\n';

const command = process.argv[2];
const { matrix, contract } = await load();
if (command === 'matrix') {
  const include = matrix.targets.map(({ id, runner, rustTarget, package: packageName, executable, os, cpu }) => ({ id, runner, rustTarget, package: packageName, executable, os, cpu }));
  process.stdout.write(JSON.stringify({ include }));
} else if (command === 'prepare') {
  const target = selectTarget(matrix, option('target'));
  const binary = path.resolve(option('binary'));
  const output = path.resolve(option('output'));
  const commit = option('commit').toLowerCase();
  if (!/^[0-9a-f]{40}$/.test(commit)) fail('commit must be a full 40-character hexadecimal SHA');
  const source = await stat(binary).catch(() => fail('binary does not exist: ' + binary));
  if (!source.isFile()) fail('binary is not a file: ' + binary);
  if (source.size === 0) fail('binary is empty: ' + binary);
  await mkdir(output, { recursive: true });
  const executable = path.join(output, target.executable);
  await copyFile(binary, executable);
  if (target.os !== 'win32') await chmod(executable, 0o755);
  const checksum = await sha256(executable);
  const metadata = {
    schemaVersion: matrix.artifactSchemaVersion,
    artifact: target.package + '-v' + contract.canonicalVersion.version,
    package: target.package,
    version: contract.canonicalVersion.version,
    commit,
    target: target.rustTarget,
    npm: { os: [target.os], cpu: [target.cpu], libc: target.libc ? [target.libc] : [] },
    executable: target.executable,
    build: { profile: 'release', flags: buildFlags },
    checksum: { algorithm: 'sha256', value: checksum }
  };
  await writeFile(path.join(output, 'artifact-metadata.json'), canonicalMetadata(metadata));
  await writeFile(path.join(output, 'SHA256SUMS'), checksum + '  ' + target.executable + '\n');
  console.log(metadata.artifact);
} else if (command === 'verify') {
  const target = selectTarget(matrix, option('target'));
  const input = path.resolve(option('input'));
  const entries = (await readdir(input)).sort();
  const expectedEntries = ['SHA256SUMS', 'artifact-metadata.json', target.executable].sort();
  if (JSON.stringify(entries) !== JSON.stringify(expectedEntries)) fail('artifact must contain exactly one executable plus artifact-metadata.json and SHA256SUMS; found: ' + entries.join(', '));
  const executableFile = path.join(input, target.executable);
  const executableStat = await stat(executableFile);
  if (!executableStat.isFile() || executableStat.size === 0) fail('artifact executable must be a non-empty file: ' + target.executable);
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
    build: { profile: 'release', flags: buildFlags },
    checksum: metadata.checksum
  };
  if (!/^[0-9a-f]{40}$/.test(metadata.commit ?? '')) fail('metadata commit is not a full hexadecimal SHA');
  if (JSON.stringify(metadata) !== JSON.stringify(expected)) fail('metadata does not match the authoritative target/version/build contract');
  if (platform.name !== target.package || platform.executable !== target.executable || platform.optionalDependencyVersion !== metadata.version) fail('release contract package metadata does not match target matrix');
  const checksum = await sha256(executableFile);
  if (metadata.checksum?.algorithm !== 'sha256' || metadata.checksum.value !== checksum) fail('executable checksum does not match metadata');
  const checksumText = await readFile(path.join(input, 'SHA256SUMS'), 'utf8');
  if (checksumText !== checksum + '  ' + target.executable + '\n') fail('SHA256SUMS does not match executable');
  console.log('Verified ' + metadata.artifact + ' (' + metadata.target + ')');
} else {
  fail('usage: release-artifact.mjs matrix | prepare --target ID --binary PATH --output DIR --commit SHA | verify --target ID --input DIR');
}
