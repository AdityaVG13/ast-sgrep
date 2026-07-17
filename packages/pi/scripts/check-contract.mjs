import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const errors = [];
const report = (condition, message) => { if (!condition) errors.push(message); };
const equal = (actual, expected) => JSON.stringify(actual) === JSON.stringify(expected);
const readJson = async (relativePath) => {
  const text = await readFile(path.join(root, relativePath), 'utf8');
  const value = JSON.parse(text);
  report(text === JSON.stringify(value, null, 2) + '\n', relativePath + ' must use deterministic two-space JSON formatting with a trailing newline');
  return value;
};
const required = (object, fields, label) => {
  for (const field of fields) report(Object.hasOwn(object ?? {}, field), label + ' is missing ' + field);
};

const contract = await readJson('packages/pi/release-contract.json');
const targetMatrix = await readJson('packages/pi/release/targets.json');
const workspace = await readJson('package.json');
const cargo = await readFile(path.join(root, 'Cargo.toml'), 'utf8');
const extensionManifest = JSON.parse(await readFile(path.join(root, 'packages/pi/extension/package.json'), 'utf8'));
const launcherManifest = JSON.parse(await readFile(path.join(root, 'packages/pi/launcher/package.json'), 'utf8'));
const runtimeSource = await readFile(path.join(root, 'packages/pi/extension/src/runtime.ts'), 'utf8');
const launcherSource = await readFile(path.join(root, 'packages/pi/launcher/src/index.js'), 'utf8');
const workspaceSection = cargo.match(/^\[workspace\.package\]\s*$([\s\S]*?)(?=^\[|(?![\s\S]))/m)?.[1] ?? "";
const cargoVersion = workspaceSection.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];

required(contract, ['schemaVersion', 'canonicalVersion', 'packages', 'compatibility', 'surface', 'config', 'offlineSemantics', 'dataLifecycle', 'updates', 'registries', 'firstPublication', 'releaseAutomation', 'nonGoals'], 'contract');
required(contract.canonicalVersion, ['version', 'source', 'npmWorkspaceSource', 'tag', 'nativeCliVersion', 'nativeCliSource', 'coupledComponents', 'driftPolicy'], 'canonicalVersion');
required(contract.packages, ['extension', 'launcher', 'platformDependencyPolicy', 'platforms', 'unsupportedTargets', 'installPolicy'], 'packages');
required(contract.packages?.extension, ['name', 'directory'], 'packages.extension');
required(contract.packages?.launcher, ['name', 'directory', 'commands'], 'packages.launcher');
required(contract.compatibility, ['node', 'pi'], 'compatibility');
required(contract.compatibility?.node, ['range', 'minimum', 'policy'], 'compatibility.node');
required(contract.compatibility?.pi, ['package', 'range', 'minimum', 'api', 'policy'], 'compatibility.pi');
required(contract.compatibility?.layers, ['extension', 'launcher', 'binary', 'machineSchema', 'configSchema', 'indexFormat'], 'compatibility.layers');
for (const component of ['extension', 'launcher', 'binary']) required(contract.compatibility?.layers?.[component], ['version', 'compatibility'], 'compatibility.layers.' + component);
required(contract.compatibility?.layers?.machineSchema, ['version', 'readable', 'policy'], 'compatibility.layers.machineSchema');
required(contract.compatibility?.layers?.configSchema, ['current', 'readable', 'rollback', 'policy'], 'compatibility.layers.configSchema');
required(contract.compatibility?.layers?.indexFormat, ['current', 'reusable', 'rebuild', 'newer', 'policy'], 'compatibility.layers.indexFormat');
required(contract.surface, ['tools', 'commands', 'cliCommands', 'defaultSearchFormat', 'separation'], 'surface');
required(contract.config, ['precedenceHighToLow', 'developerBinaryOverrides', 'defaultRoot', 'rootPolicy'], 'config');
required(contract.offlineSemantics, ['defaultBackend', 'localSemanticSearchAlwaysAvailable', 'firstUseModelDownload', 'credentialsRequired', 'lazyIndexOnFirstSearch', 'optionalBackends', 'policy'], 'offlineSemantics');
required(contract.dataLifecycle, ['path', 'created', 'contents', 'gitignoreMutation', 'compatibleUpdate', 'incompatibleUpdate', 'failedUpdate', 'uninstall', 'deletion'], 'dataLifecycle');
required(contract.updates, ['mainBranch', 'officialTag', 'selection', 'runtimeDownloads', 'partialPublishRecovery'], 'updates');
required(contract.registries, ['sharedAnchor', 'npm', 'cratesIo'], 'registries');
required(contract.registries?.npm, ['requiresCratesIoPublication', 'availabilityObserved', 'policy'], 'registries.npm');
required(contract.registries?.cratesIo, ['requiresNpmPublication', 'policy'], 'registries.cratesIo');
required(contract.firstPublication, ['humanAuthorizationRequired', 'automatedFirstPublishForbidden', 'protectedEnvironmentApprovalRequired', 'packageNameAndOwnershipVerificationRequired', 'trustedPublishingRequired', 'provenanceRequired', 'authorizationRecordRequired', 'gate'], 'firstPublication');
required(contract.releaseAutomation, ['dryRunWorkflow', 'officialWorkflow', 'officialTrigger', 'protectedEnvironment', 'trustedPublishing', 'provenance', 'packageOrder', 'idempotence'], 'releaseAutomation');
report(contract.schemaVersion === 2, 'unsupported contract schemaVersion');
const version = contract.canonicalVersion?.version;
const nativeVersion = contract.canonicalVersion?.nativeCliVersion;
report(typeof version === 'string' && /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version), 'canonical version must be explicit semver');
report(contract.canonicalVersion?.source === 'package.json#version' && contract.canonicalVersion?.npmWorkspaceSource === 'package.json#version', 'canonical npm version source changed');
report(contract.canonicalVersion?.tag === 'v' + version, 'official tag must be v<canonical version>');
report(contract.canonicalVersion?.nativeCliSource === 'Cargo.toml#workspace.package.version' && cargoVersion === nativeVersion, 'native CLI version drifts from the Rust workspace');
report(workspace.version === version, 'npm workspace version drifts from the Pi contract');
report(workspace.name === 'ast-sgrep-workspace' && workspace.private === true, 'root npm workspace name/private policy changed');
report(equal(workspace.workspaces, ['packages/pi/extension', 'packages/pi/launcher']), 'root npm workspace paths changed or native platform templates became direct workspaces');
report(workspace.scripts?.['check:pi-contract'] === 'node packages/pi/scripts/check-contract.mjs', 'root contract-check script changed');
report(workspace.scripts?.['check:pi-release'] === 'node packages/pi/scripts/check-native-workflow.mjs' && workspace.scripts?.['pack:pi-release'] === 'node packages/pi/scripts/release-acceptance.mjs pack' && workspace.scripts?.['test:pi-release-gate'] === 'node packages/pi/scripts/release-acceptance.mjs self-test', 'root release acceptance scripts changed');
report(Object.keys(workspace).every((key) => ['name', 'version', 'private', 'workspaces', 'scripts'].includes(key)), 'root package.json must remain the minimal private workspace');

required(targetMatrix, ['schemaVersion', 'artifactSchemaVersion', 'targets'], 'target matrix');
report(targetMatrix.schemaVersion === 1 && targetMatrix.artifactSchemaVersion === 1, 'unsupported target matrix schema');
report(Array.isArray(targetMatrix.targets) && targetMatrix.targets.length === 5, 'target matrix must contain exactly five supported targets');
const ids = targetMatrix.targets?.map((target) => target.id) ?? [];
report(new Set(ids).size === ids.length, 'target matrix IDs must be unique');
report(equal(ids, ['darwin-arm64', 'darwin-x64', 'linux-arm64-gnu', 'linux-x64-gnu', 'win32-x64-msvc']), 'target matrix set or deterministic order changed');
for (const target of targetMatrix.targets ?? []) {
  required(target, ['id', 'package', 'rustTarget', 'runner', 'os', 'cpu', 'libc', 'executable'], 'target matrix ' + target.id);
  report(target.package === 'ast-sgrep-' + target.id, target.id + ' package name must match its npm platform/CPU ID');
  report(target.executable === (target.os === 'win32' ? 'asgrep.exe' : 'asgrep'), target.id + ' executable name is invalid');
  report(!/musl/.test(target.id + target.rustTarget + (target.libc ?? '')), 'musl targets are unsupported');
  report(!(target.os === 'win32' && target.cpu === 'arm64'), 'Windows arm64 is unsupported');
  report((target.os === 'darwin' && /^macos-/.test(target.runner)) || (target.os === 'linux' && /^ubuntu-/.test(target.runner)) || (target.os === 'win32' && /^windows-/.test(target.runner)), target.id + ' must use a native GitHub runner');
}
const expectedPlatforms = (targetMatrix.targets ?? []).map((target) => [target.package, 'packages/pi/platforms/' + target.id, target.rustTarget, [target.os], [target.cpu], target.libc ? [target.libc] : [], target.executable]);
report(contract.packages?.extension?.name === 'pi-ast-sgrep', 'unsupported Pi extension package name');
report(contract.packages?.launcher?.name === 'ast-sgrep', 'unsupported launcher package name');
report(equal(contract.packages?.launcher?.commands, ['asgrep', 'ast-sgrep']), 'unsupported launcher command names');
report(contract.packages?.platformDependencyPolicy === 'exact', 'platform dependencies must use exact versions');
report(Array.isArray(contract.packages?.platforms) && contract.packages.platforms.length === expectedPlatforms.length, 'platform target count changed');
for (let index = 0; index < expectedPlatforms.length; index += 1) {
  const platform = contract.packages?.platforms?.[index] ?? {};
  const [name, directory, target, os, cpu, libc, executable] = expectedPlatforms[index];
  required(platform, ['name', 'directory', 'target', 'os', 'cpu', 'libc', 'executable', 'optionalDependencyVersion'], 'packages.platforms[' + index + ']');
  report(equal([platform.name, platform.directory, platform.target, platform.os, platform.cpu, platform.libc, platform.executable], [name, directory, target, os, cpu, libc, executable]), 'invalid target/package mapping at platform index ' + index);
  report(platform.optionalDependencyVersion === version, name + ' optional dependency is not pinned to the exact canonical version');
}
report(equal(contract.packages?.unsupportedTargets, ['linux-musl', 'win32-arm64']), 'unsupported target policy changed');
report(equal(contract.surface?.tools, ['asgrep_search', 'asgrep_index', 'asgrep_status']), 'unsupported Pi tool names');
report(equal(contract.surface?.commands, ['/asgrep-doctor', '/asgrep-status', '/asgrep-index', '/asgrep-reindex']), 'unsupported Pi command names');
report(equal(contract.surface?.cliCommands, ['asgrep', 'ast-sgrep']) && contract.surface?.defaultSearchFormat === 'agent-capsule', 'unsupported CLI surface or default search format');
report(contract.compatibility?.node?.range === '>=22.19.0' && contract.compatibility.node.minimum === '22.19.0', 'Node compatibility floor changed');
report(contract.compatibility?.pi?.package === '@earendil-works/pi-coding-agent' && contract.compatibility.pi.range === '>=0.80.6 <1' && contract.compatibility.pi.minimum === '0.80.6' && contract.compatibility.pi.api === 'ExtensionAPI', 'Pi compatibility policy changed');
const layers = contract.compatibility?.layers ?? {};
report(layers.extension?.version === version && layers.launcher?.version === version && layers.binary?.version === nativeVersion, 'npm layers or embedded native CLI drift from their canonical versions');
report(layers.extension?.compatibility === 'exact' && layers.launcher?.compatibility === 'exact' && layers.binary?.compatibility === 'exact-native-cli', 'release layer compatibility must reject package or native CLI version skew');
report(extensionManifest.version === version && launcherManifest.version === version && extensionManifest.dependencies?.['ast-sgrep'] === version, 'extension and launcher manifests drift from the compatibility matrix');
report(runtimeSource.includes(`export const RUNTIME_VERSION = "${nativeVersion}";`) && launcherSource.includes(`const VERSION = "${version}";`), 'runtime native CLI expectation or launcher package version drifts from the compatibility matrix');
report(layers.machineSchema?.version === '1.0.0' && equal(layers.machineSchema.readable, ['1.0.0']) && runtimeSource.includes('export const MACHINE_SCHEMA_VERSION = "1.0.0";'), 'machine schema compatibility matrix is inconsistent');
report(layers.configSchema?.current === 1 && equal(layers.configSchema.readable, [0, 1]) && equal(layers.configSchema.rollback, [0]) && runtimeSource.includes('export const CONFIG_SCHEMA_VERSION = 1 as const;'), 'config schema migration/rollback matrix is inconsistent');
report(layers.indexFormat?.current === 5 && equal(layers.indexFormat.reusable, [5]) && equal(layers.indexFormat.rebuild, [0, 1, 2, 3, 4]) && layers.indexFormat.newer === 'reject-and-preserve' && runtimeSource.includes('export const INDEX_FORMAT_VERSION = 5 as const;'), 'index format reuse/rebuild matrix is inconsistent');
report(equal(contract.config?.precedenceHighToLow, ['explicit-project-config', 'project-settings', 'global-settings', 'environment', 'defaults']), 'config precedence changed');
report(contract.offlineSemantics?.defaultBackend === 'local' && contract.offlineSemantics.localSemanticSearchAlwaysAvailable === true && contract.offlineSemantics.firstUseModelDownload === false && contract.offlineSemantics.credentialsRequired === false && contract.offlineSemantics.lazyIndexOnFirstSearch === true, 'offline local semantic contract changed');
report(contract.dataLifecycle?.path === '<project-root>/.asgrep' && contract.dataLifecycle.gitignoreMutation === false && /preserves/.test(contract.dataLifecycle.uninstall ?? '') && /explicit user request/.test(contract.dataLifecycle.deletion ?? ''), '.asgrep lifecycle is incomplete');
report(contract.updates?.runtimeDownloads === false && /never publish/.test(contract.updates.mainBranch ?? '') && /human-approved official version tag/.test(contract.updates.officialTag ?? ''), 'update/publication policy changed');
report(contract.registries?.npm?.requiresCratesIoPublication === false && contract.registries?.cratesIo?.requiresNpmPublication === false, 'npm and crates.io must remain independently publishable');
const availability = contract.registries?.npm?.availabilityObserved ?? {};
required(availability, ['date', 'version', 'registryResult', 'names', 'reservationGuaranteed', 'policy'], 'registries.npm.availabilityObserved');
report(availability.date === '2026-07-16' && availability.version === version && availability.registryResult === '404/not-found', 'npm availability observation changed');
report(equal(availability.names, ['pi-ast-sgrep', 'ast-sgrep', ...expectedPlatforms.map(([name]) => name)]), 'npm availability observation must cover every public package name');
report(availability.reservationGuaranteed === false && /re-verify/.test(availability.policy ?? ''), 'npm availability must not be represented as a reservation guarantee');
const gate = contract.firstPublication ?? {};
for (const field of ['humanAuthorizationRequired', 'automatedFirstPublishForbidden', 'protectedEnvironmentApprovalRequired', 'packageNameAndOwnershipVerificationRequired', 'trustedPublishingRequired', 'provenanceRequired', 'authorizationRecordRequired']) report(gate[field] === true, 'first-publication gate requires ' + field);
report(/Before any external registry side effect/.test(gate.gate ?? '') && /human MUST/.test(gate.gate ?? ''), 'first publication lacks an explicit human gate');
const automation = contract.releaseAutomation ?? {};
report(automation.dryRunWorkflow === '.github/workflows/pi-native-artifacts.yml' && automation.officialWorkflow === '.github/workflows/pi-npm-release.yml', 'release workflow paths changed');
report(automation.officialTrigger === 'signed canonical version tag only' && automation.protectedEnvironment === 'npm-production' && automation.trustedPublishing === 'npm OIDC' && automation.provenance === true, 'official release protection/OIDC/provenance contract changed');
report(equal(automation.packageOrder, [...expectedPlatforms.map(([name]) => name), 'ast-sgrep', 'pi-ast-sgrep']), 'release package order must be native -> launcher -> extension');
report(/dirty/.test(automation.idempotence ?? '') && /wrong-tag/.test(automation.idempotence ?? '') && /already-published/.test(automation.idempotence ?? ''), 'release idempotence refusal contract is incomplete');

if (errors.length) {
  for (const error of errors) console.error('Pi contract: ' + error);
  process.exitCode = 1;
} else {
  console.log('Pi release contract is consistent at ' + version);
}
