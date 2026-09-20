// fff-mcp warm grep leg: time ranked content search on one warm server.
//
// Usage: node benchmarks/fff_grep_leg.mjs <corpus-root> [needle] [runs]
// Requires: fff-mcp on PATH (pin recorded in benchmarks/results/speed.md).
//
// fff is a warm-server ranked file finder, not an exhaustive grep: it
// reports "20/54 matches shown" where ripgrep finds ~300 literal lines on
// the same corpus. Compare latency only, never match sets.
import { spawn } from 'node:child_process';

const root = process.argv[2] || '.';
const needle = process.argv[3] || 'SearchHit';
const N = Number(process.argv[4] || 15);

const tSpawn = process.hrtime.bigint();
const child = spawn('fff-mcp', ['--no-watch', '--no-update-check', root], {
  stdio: ['pipe', 'pipe', 'ignore'],
});
let buf = '';
let id = 0;
const pending = new Map();
child.stdout.on('data', (d) => {
  buf += d.toString();
  let i;
  while ((i = buf.indexOf('\n')) >= 0) {
    const line = buf.slice(0, i).trim();
    buf = buf.slice(i + 1);
    if (!line) continue;
    let msg;
    try {
      msg = JSON.parse(line);
    } catch {
      continue;
    }
    if (msg.id !== undefined && pending.has(msg.id)) {
      pending.get(msg.id)(msg);
      pending.delete(msg.id);
    }
  }
});
const send = (method, params) =>
  new Promise((res) => {
    const myId = ++id;
    pending.set(myId, res);
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: myId, method, params })}\n`);
  });

await send('initialize', {
  protocolVersion: '2024-11-05',
  capabilities: {},
  clientInfo: { name: 'fff_grep_leg', version: '0' },
});
child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' })}\n`);
const call = (q) => send('tools/call', { name: 'grep', arguments: { query: q, maxResults: 20 } });

const w0 = process.hrtime.bigint();
const warm = await call(needle);
const coldMs = Number(process.hrtime.bigint() - tSpawn) / 1e6;
const warmMs = Number(process.hrtime.bigint() - w0) / 1e6;
const txt = warm.result?.content?.[0]?.text ?? JSON.stringify(warm.error);
console.log(`spawn-to-first-result: ${coldMs.toFixed(0)}ms (first call itself ${warmMs.toFixed(1)}ms)`);
console.log('coverage:', (txt.match(/\d+\/\d+ matches shown/) ?? ['?'])[0]);

const ts = [];
for (let i = 0; i < N; i++) {
  const t0 = process.hrtime.bigint();
  await call(needle);
  ts.push(Number(process.hrtime.bigint() - t0) / 1e6);
}
ts.sort((a, b) => a - b);
const q = (p) => ts[Math.floor(((ts.length - 1) * p) / 100)];
console.log(
  `warm grep n=${N} p50=${q(50).toFixed(1)}ms p95=${q(95).toFixed(1)}ms min=${ts[0].toFixed(1)} max=${ts[N - 1].toFixed(1)}`
);
child.kill();
