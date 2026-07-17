import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const workflowText = await readFile(path.join(root, '.github/workflows/pi-native-artifacts.yml'), 'utf8');
const officialText = await readFile(path.join(root, '.github/workflows/pi-npm-release.yml'), 'utf8');
const allowedSignersText = await readFile(path.join(root, 'packages/pi/release/allowed-signers'), 'utf8');
const expectedAllowedSigner = 'adityavgcode@gmail.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICeIowlFrWVQpSI2f/8qjz1KZY7Uif+cFR0u5Jwin8oH mac-m5-max\n';
const releaseHelper = await readFile(path.join(root, 'packages/pi/scripts/release-acceptance.mjs'), 'utf8');
const targets = JSON.parse(await readFile(path.join(root, 'packages/pi/release/targets.json'), 'utf8')).targets;
const helper = await readFile(path.join(root, 'packages/pi/scripts/ci-install-smoke.mjs'), 'utf8');
const ruby = 'document = YAML.safe_load(STDIN.read, aliases: true); puts JSON.generate(document)';
const parse = (text) => {
  const result = spawnSync('ruby', ['-rjson', '-ryaml', '-e', ruby], { input: text, encoding: 'utf8', windowsHide: true });
  if (result.status !== 0) throw new Error(result.stderr.trim() || 'Ruby YAML parser failed');
  const value = JSON.parse(result.stdout);
  if (value.on === undefined && value.true !== undefined) { value.on = value.true; delete value.true; }
  return value;
};
const activeRun = (step) => typeof step?.run === 'string' ? step.run.split('\n').map((line) => line.trim()).filter((line) => line && !line.startsWith('#')).join('\n') : '';
const validate = (text) => {
  const errors = [];
  let workflow;
  try { workflow = parse(text); } catch (error) { return ['YAML parse failed: ' + error.message]; }
  const report = (condition, message) => { if (!condition) errors.push(message); };
  const triggers = Object.keys(workflow.on ?? {});
  report(triggers.length === 1 && triggers[0] === 'workflow_dispatch', 'native artifact workflow must be manual-only');
  const load = workflow.jobs?.['target-matrix'];
  const native = workflow.jobs?.['native-artifact'];
  report(load?.outputs?.matrix === '${{ steps.targets.outputs.matrix }}', 'target matrix output expression is invalid');
  report(native?.strategy?.matrix === '${{ fromJSON(needs.target-matrix.outputs.matrix) }}', 'native matrix expression is invalid');
  report(native?.['runs-on'] === '${{ matrix.runner }}', 'native job must run on the matrix runner');
  const loadRuns = (load?.steps ?? []).map(activeRun);
  report(loadRuns.some((run) => run.includes('node packages/pi/scripts/release-artifact.mjs matrix')), 'authoritative matrix command is missing');
  report(loadRuns.some((run) => run.includes('node packages/pi/scripts/check-contract.mjs') && run.includes('node packages/pi/scripts/check-native-workflow.mjs')), 'contract checker commands are missing');
  const steps = new Map((native?.steps ?? []).filter((step) => step.name).map((step) => [step.name, activeRun(step)]));
  report(steps.get('Build target-local release executable')?.includes('cargo build --locked --release'), 'locked native build step is missing');
  const metadata = steps.get('Prepare and verify deterministic artifact metadata') ?? '';
  report(metadata.includes('release-artifact.mjs prepare') && metadata.includes('release-artifact.mjs verify'), 'metadata prepare/verify step is missing');
  const pack = steps.get('Pack native, launcher, and extension tarballs') ?? '';
  report(pack.includes('npm pack "$platform_dir"') && pack.includes('npm pack packages/pi/launcher') && pack.includes('npm pack packages/pi/extension'), 'all npm pack commands are required');
  report((steps.get('Clean-install local tarballs') ?? '').includes('npm install --no-audit --no-fund --prefix "$clean"'), 'clean local install is missing');
  report((steps.get('Exercise installed launcher and extension') ?? '').includes('node packages/pi/scripts/ci-install-smoke.mjs'), 'installed smoke command is missing');
  report((native?.steps ?? []).some((step) => step.name === 'Upload native artifact' && step.uses === 'actions/upload-artifact@v4'), 'artifact upload is missing');
  const acceptance = workflow.jobs?.['release-acceptance'];
  const acceptanceRuns = (acceptance?.steps ?? []).map(activeRun).join('\n');
  report(JSON.stringify(acceptance?.needs) === '["target-matrix","native-artifact"]', 'complete dry-run must wait for every native artifact');
  report(acceptanceRuns.includes('release-acceptance.mjs pack --native-root dist/native') && acceptanceRuns.includes('release-acceptance.mjs verify --artifacts npm-packs'), 'complete pack/inspection commands are missing');
  report(acceptanceRuns.includes('release-acceptance.mjs self-test') && acceptanceRuns.includes('npm run test:pi-e2e'), 'release gate and official Pi loader acceptance are missing');
  report(!/npm publish/u.test(workflowText), 'main/PR workflow must never contain npm publish');
  return errors;
};

const validateOfficial = (text, signersText = allowedSignersText) => {
  const errors = [];
  let workflow;
  try { workflow = parse(text); } catch (error) { return ['official YAML parse failed: ' + error.message]; }
  const report = (condition, message) => { if (!condition) errors.push(message); };
  const triggers = Object.keys(workflow.on ?? {});
  const inputs = workflow.on?.workflow_dispatch?.inputs;
  report(triggers.length === 1 && triggers[0] === 'workflow_dispatch', 'official publication workflow must be manual-only');
  report(inputs?.release_tag?.required === true && inputs.release_tag.type === 'string', 'official publication requires an explicit release_tag string input');
  report(inputs?.publish?.required === true && inputs.publish.type === 'boolean' && inputs.publish.default === false, 'official publication requires an explicit publish intent defaulting to false');
  report(inputs?.bootstrap_token?.required === true && inputs.bootstrap_token.type === 'boolean' && inputs.bootstrap_token.default === false, 'official publication requires an explicit bootstrap_token intent defaulting to false');
  const gate = workflow.jobs?.['release-gate'];
  report(gate?.if === "${{ inputs.publish == true && github.ref_type == 'tag' && github.ref_name == inputs.release_tag }}", 'official release gate must require publish intent and an exact tag ref match');
  const build = workflow.jobs?.['build-native'];
  const verify = workflow.jobs?.['verify-release'];
  const publish = workflow.jobs?.publish;
  const bootstrapToken = "${{ inputs.bootstrap_token && secrets.NPM_TOKEN || '' }}";
  report(build?.strategy?.matrix === '${{ fromJSON(needs.release-gate.outputs.matrix) }}' && build?.['runs-on'] === '${{ matrix.runner }}', 'official native build must use the authoritative native matrix once');
  const gateRuns = (gate?.steps ?? []).map(activeRun);
  const sshSetup = 'git config --local gpg.format ssh\ngit config --local gpg.ssh.allowedSignersFile \"$GITHUB_WORKSPACE/packages/pi/release/allowed-signers\"';
  const sshSetupIndex = gateRuns.indexOf(sshSetup);
  const officialGateIndex = gateRuns.findIndex((run) => run.includes('release-acceptance.mjs gate'));
  report(sshSetupIndex !== -1 && sshSetupIndex < officialGateIndex, 'official release gate must configure repository-local SSH verification with the tracked allowed-signers file before verifying the tag');
  report(signersText === expectedAllowedSigner, 'tracked SSH allowed signer must contain the exact release principal and public key');
  const gateRunText = gateRuns.join('\n');
  report(gateRunText.includes('release-acceptance.mjs gate') && gateRunText.includes('npm run check:pi-contract') && gateRunText.includes('npm run check:pi-release'), 'official tag/version/duplicate gate is missing');
  const verifyRuns = (verify?.steps ?? []).map(activeRun).join('\n');
  report(verifyRuns.includes('release-acceptance.mjs pack --native-root dist/native') && verifyRuns.includes('release-acceptance.mjs verify --artifacts npm-packs') && verifyRuns.includes('npm run test:pi-e2e'), 'official release must pack and verify the complete family exactly once');
  report(verify?.permissions?.['id-token'] === 'write' && verify?.permissions?.attestations === 'write' && (verify?.steps ?? []).some((step) => step.uses === 'actions/attest-build-provenance@v2'), 'artifact provenance attestation is missing');
  report(publish?.environment === 'npm-production' && publish?.env?.ASGREP_NPM_PROTECTED_ENVIRONMENT === 'npm-production' && publish?.permissions?.['id-token'] === 'write' && publish?.needs === 'verify-release', 'npm publication must use protected npm-production OIDC after verification');
  report(publish?.env?.NODE_AUTH_TOKEN === bootstrapToken, 'npm bootstrap token must use the exact default-off opt-in expression');
  report(text.split('\n').filter((line) => line.trim() === 'NODE_AUTH_TOKEN: ' + bootstrapToken).length === 1 && (text.match(/secrets\.NPM_TOKEN/gu) ?? []).length === 1, 'npm bootstrap token wiring must appear exactly once in the publish job');
  const publishSteps = (publish?.steps ?? []).filter((step) => step.name).map((step) => [step.name, activeRun(step)]);
  const layers = publishSteps.filter(([, run]) => run.includes('release-acceptance.mjs publish')).map(([, run]) => run.match(/--layer (native|launcher|extension)/u)?.[1]);
  report(JSON.stringify(layers) === '["native","launcher","extension"]', 'publication order must be native -> launcher -> extension');

  return errors;
};

const errors = [...validate(workflowText), ...validateOfficial(officialText)];
for (const target of targets) {
  if (!target.runner) errors.push(target.id + ' has no native runner');
  if (target.id === 'linux-arm64-gnu' && !/^ubuntu-.*-arm$/u.test(target.runner)) errors.push('linux arm64 must use an explicit native ARM runner');
}
for (const label of ['version', 'fixture-index', 'status', 'semantic-natural-search', 'defs', 'callers', 'timeout-cancellation', 'doctor', 'extension-load']) if (!helper.includes(label)) errors.push('smoke helper is missing ' + label);
const mutations = [
  workflowText.replace('matrix: ${{ fromJSON(needs.target-matrix.outputs.matrix) }}', 'matrix: {}'),
  workflowText.replace('  workflow_dispatch:', '  push:'),
  workflowText.replace('          node packages/pi/scripts/ci-install-smoke.mjs', '          # node packages/pi/scripts/ci-install-smoke.mjs'),
  workflowText.replace('          node packages/pi/scripts/release-acceptance.mjs self-test', '          # node packages/pi/scripts/release-acceptance.mjs self-test')
];
for (const [index, mutation] of mutations.entries()) if (validate(mutation).length === 0) errors.push('negative dry-run mutation ' + (index + 1) + ' was not rejected');
const officialMutations = [
  officialText.replace('    environment: npm-production', '    # environment removed'),
  officialText.replace('--layer native', '--layer launcher'),
  officialText.replace('      id-token: write', '      id-token: read'),
  officialText.replace('      release_tag:', '      release_tag_removed:'),
  officialText.replace('        default: false', '        default: true'),
  officialText.replace("github.ref_name == inputs.release_tag", "github.ref_name != inputs.release_tag"),
  officialText.replace("      bootstrap_token:\n        description: One-time first-publication npm token bootstrap; leave off for all subsequent publications\n        required: true\n        type: boolean\n        default: false", "      bootstrap_token:\n        description: One-time first-publication npm token bootstrap; leave off for all subsequent publications\n        required: true\n        type: boolean\n        default: true"),
  officialText.replace("      bootstrap_token:", "      bootstrap_token_removed:"),
  officialText.replace("      NODE_AUTH_TOKEN: ${{ inputs.bootstrap_token && secrets.NPM_TOKEN || '' }}", "      NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}"),
  officialText.replace("      NODE_AUTH_TOKEN: ${{ inputs.bootstrap_token && secrets.NPM_TOKEN || '' }}", "      # NODE_AUTH_TOKEN omitted"),
  officialText.replace("      NODE_AUTH_TOKEN: ${{ inputs.bootstrap_token && secrets.NPM_TOKEN || '' }}", "      NODE_AUTH_TOKEN: ${{ inputs.bootstrap_token && secrets.NPM_TOKEN }}"),
  officialText.replace("      - name: Configure SSH tag verification\n        run: |\n          git config --local gpg.format ssh\n          git config --local gpg.ssh.allowedSignersFile \"$GITHUB_WORKSPACE/packages/pi/release/allowed-signers\"\n", ''),
  officialText.replace('packages/pi/release/allowed-signers', 'packages/pi/release/wrong-signers')
];
for (const [index, mutation] of officialMutations.entries()) if (validateOfficial(mutation).length === 0) errors.push('negative official mutation ' + (index + 1) + ' was not rejected');
const signerMutations = [
  allowedSignersText.replace('adityavgcode@gmail.com', 'attacker@example.com'),
  allowedSignersText.replace('AAAAC3NzaC1lZDI1NTE5AAAAICeIowlFrWVQpSI2f/8qjz1KZY7Uif+cFR0u5Jwin8oH', 'AAAAC3NzaC1lZDI1NTE5AAAAICorrupted')
];
for (const [index, mutation] of signerMutations.entries()) if (validateOfficial(officialText, mutation).length === 0) errors.push('negative allowed-signers mutation ' + (index + 1) + ' was not rejected');
for (const token of ['ASGREP_RELEASE_DIRTY', 'ASGREP_RELEASE_TAG_VERSION', 'ASGREP_RELEASE_TAG_COMMIT', 'ASGREP_RELEASE_CHECKSUM_MISSING', 'ASGREP_RELEASE_CHECKSUM_MISMATCH', 'ASGREP_RELEASE_VERSION_SKEW', 'ASGREP_RELEASE_DUPLICATE_VERSION', 'ASGREP_RELEASE_OIDC_REQUIRED', 'ASGREP_RELEASE_PROTECTED_ENVIRONMENT', "['publish'", "'--provenance'"]) if (!releaseHelper.includes(token)) errors.push('release acceptance helper is missing ' + token);
if (errors.length) {
  for (const error of errors) console.error('Pi native workflow: ' + error);
  process.exitCode = 1;
} else console.log('Pi npm workflows are structurally consistent across ' + targets.length + ' targets and 7 packages; rejected ' + (mutations.length + officialMutations.length + signerMutations.length) + ' negative mutations');
