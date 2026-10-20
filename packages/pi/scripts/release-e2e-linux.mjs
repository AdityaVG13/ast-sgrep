import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// npm run release:e2e-linux — run the packaged release-gate e2e on a real
// Linux host before dispatching the CI lane.
//
// Why: the 2.6.0 "database disk image is malformed" bug was Linux-only and
// only surfaced in the lane's packaged e2e, after ~30 min of CI minutes per
// attempt. The same e2e on a Linux box takes ~2 min once the natives are
// built there, and the bug is platform-conditional, not CI-conditional.
//
// The host is NOT hardcoded: this repository is public and the hosts are
// private infrastructure. Configure with env vars:
//   ASGREP_E2E_SSH   user@host           (required; ssh must be key-based)
//   ASGREP_E2E_DIR   remote directory    (required; created if missing)
// The remote needs bash, node >= 22.19, npm, cargo, rsync. node_modules is
// synced incrementally by rsync, so the first run pays the install and later
// runs pay only deltas.

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const sshTarget = process.env.ASGREP_E2E_SSH;
const remoteDir = process.env.ASGREP_E2E_DIR;
const fail = (code, message) => { console.error(`[e2e-linux] ${code}: ${message}`); process.exit(1); };
if (!sshTarget || !remoteDir) {
  fail('ASGREP_E2E_USAGE', 'set ASGREP_E2E_SSH=user@host and ASGREP_E2E_DIR=/remote/path (private build host; nothing is hardcoded in this public repo)');
}

const sh = (command, args, options = {}) => spawnSync(command, args, { cwd: root, encoding: 'utf8', windowsHide: true, ...options });
const ssh = (script, options = {}) => sh('ssh', ['-o', 'ConnectTimeout=15', sshTarget, script], options);
const step = (name, ok, detail = '') => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? ' — ' + detail : ''}`); if (!ok) fail('ASGREP_E2E_STEP', name); return ok; };

const probe = ssh('node --version 2>/dev/null; cargo --version 2>/dev/null; rsync --version 2>/dev/null | head -1');
const [nodeLine, cargoLine] = String(probe.stdout ?? '').split(/\r?\n/u);
const nodeOk = /^\s*v(\d+)\./u.exec(nodeLine ?? '');
const nodeMajor = nodeOk ? Number(nodeOk[1]) : 0;
step('remote toolchain', probe.status === 0 && nodeMajor >= 22 && Boolean(cargoLine), `node ${nodeLine?.trim() || '?'} / ${cargoLine?.trim() || 'no cargo'}`);

console.log('[e2e-linux] syncing tree (incremental rsync; first run carries node_modules)');
const rsync = sh('rsync', ['-az', '--delete',
  '--exclude', '/.git/', '--exclude', '/target/', '--exclude', '/**/target/',
  '--exclude', '/.asgrep/', '--exclude', '/spark-tmp-dir/', '--exclude', '/.DS_Store',
  `${root}/`, `${sshTarget}:${remoteDir}/`]);
step('rsync tree', rsync.status === 0, rsync.status === 0 ? '' : String(rsync.stderr ?? '').trim().split('\n').slice(-1)[0]);

const build = ssh(`cd ${remoteDir} && cargo build --release -p ast-sgrep-cli -p ast-sgrep-watch -p ast-sgrep-codemode-napi 2>&1 | tail -2; echo BUILD_EXIT=\${PIPESTATUS[0]}`);
const buildExit = /BUILD_EXIT=(\d+)/u.exec(String(build.stdout ?? ''))?.[1];
step('remote cargo build', buildExit === '0', String(build.stdout ?? '').trim().split('\n').slice(-2).join(' | ').slice(0, 160));

const stage = ssh(`cd ${remoteDir} && mkdir -p spark-native && cp -f target/release/asgrep target/release/asgrep-watch spark-native/ && cp -f target/release/libast_sgrep_codemode_napi.so spark-native/ast-sgrep-codemode.node && echo STAGED`);
step('stage natives', /STAGED/u.test(String(stage.stdout ?? '')));

console.log('[e2e-linux] running the packaged release-gate e2e (node on the remote drives npm pack, install, and every stage)');
const e2e = ssh(`cd ${remoteDir} && ASGREP_TEST_NATIVE_DIR=$PWD/spark-native node packages/pi/scripts/release-gate-e2e.mjs`, { stdio: 'inherit', shell: false });
console.log('');
step('release-gate e2e', e2e.status === 0);
console.log('[e2e-linux] OK — Linux packaged behavior verified before any CI minutes are spent');
