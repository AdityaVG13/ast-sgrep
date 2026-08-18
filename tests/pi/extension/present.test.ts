import assert from "node:assert/strict";
import test from "node:test";
import {
  formatCodemodeCall,
  formatCodemodeResult,
  formatIndexCall,
  formatSearchCall,
  formatSearchResult,
  formatStatusCall,
  formatStatusResult,
  presentText,
} from "../../../packages/pi/extension/src/present.js";

test("search call chrome names the tool, query, and mode", () => {
  const text = formatSearchCall({ query: "auth refresh", mode: "defs", limit: 8 });
  assert.equal(text, 'asgrep  ·  search  ·  "auth refresh"  ·  defs  ·  limit 8');
});

test("search result chrome lists file:line and symbol instead of a JSON blob", () => {
  const text = formatSearchResult(
    { hits: [{ file: "src/auth.rs", start_line: 42, symbol: "refresh_token", kind: "function" }] },
    { command: "search", query: "auth refresh", mode: "natural", activationMs: 0.42, backend: "napi" },
  );
  assert.match(text, /^asgrep {2}· {2}search {2}· {2}"auth refresh" {2}· {2}natural {2}· {2}1 hit {2}· {2}0\.42ms {2}· {2}napi$/m);
  assert.match(text, /src\/auth\.rs:42 {2}refresh_token {2}function/);
  assert.doesNotMatch(text, /\{"hits"/);
});

test("index, status, and codemode calls stay one line", () => {
  assert.equal(formatIndexCall(false), "asgrep  ·  index");
  assert.equal(formatIndexCall(true), "asgrep  ·  reindex");
  assert.equal(formatStatusCall(), "asgrep  ·  status");
  assert.match(formatCodemodeCall("async () => asgrep.search({ query: 'auth' })"), /asgrep {2}· {2}codemode {2}· {2}async/);
});

test("codemode result uses hit rows when the program returned hits", () => {
  const text = formatCodemodeResult(
    { hits: [{ path: "src/a.ts", line: 3, symbol: "ensureFresh" }] },
    { wallMs: 2, backend: "napi" },
  );
  assert.match(text, /asgrep {2}· {2}codemode/);
  assert.match(text, /src\/a\.ts:3 {2}ensureFresh/);
});

test("codemode result lists shaped keys instead of dumping JSON", () => {
  const text = formatCodemodeResult({ symbol: "refresh_token", n: 2 }, { stats: { calls: 2, batchedCalls: 0, parallelSpawnCalls: 0, stickyCalls: 2, waves: 1 }, wallMs: 3, backend: "napi" });
  assert.match(text, /asgrep {2}· {2}codemode {2}· {2}in-process {2}· {2}native 2 {2}· {2}3ms/);
  assert.match(text, /symbol: refresh_token/);
  assert.match(text, /n: 2/);
  assert.doesNotMatch(text, /\{"symbol"/);
});

test("status result is a single header line", () => {
  const text = formatStatusResult({ ok: true, status: "ready", counts: { files: 12, symbols: 34 }, backend: "fastembed" });
  assert.equal(text, "asgrep  ·  status  ·  ready  ·  files=12  symbols=34  ·  fastembed");
});

test("presentText reuses the last component", () => {
  const first = presentText("one", undefined);
  const second = presentText("two", first);
  assert.equal(first, second);
  assert.deepEqual(second.render(80), ["two"]);
});

function testVisibleWidth(text: string): number {
  let width = 0;
  for (let i = 0; i < text.length; ) {
    if (text.charCodeAt(i) === 0x1b) {
      const csi = text.slice(i).match(/^\x1b\[[0-9;?]*[ -/]*[@-~]/);
      if (csi) {
        i += csi[0].length;
        continue;
      }
      const osc = text.slice(i).match(/^\x1b\].*?(?:\x07|\x1b\\)/);
      if (osc) {
        i += osc[0].length;
        continue;
      }
      i += Math.min(2, text.length - i);
      continue;
    }
    const code = text.charCodeAt(i);
    width += code <= 0x7e ? 1 : 2;
    i += code >= 0xd800 && code <= 0xdbff ? 2 : 1;
  }
  return width;
}

test("AsgrepText truncates a long search header to the terminal width", () => {
  const query =
    "In pi/packages/pi-zsx/index.js, find where the zero tool is registered, including its name, description, parameters, system prompt or agent policy injection, and examples. Return relevant symbols and bodies.";
  const theme = {
    bold: (text: string) => `\x1b[1m${text}\x1b[22m`,
    fg: (_role: string, text: string) => `\x1b[38;2;182;183;250m${text}\x1b[39m`,
  };
  const component = presentText(formatSearchCall({ query, mode: "natural" }, theme), undefined);
  const lines = component.render(91);
  assert.equal(lines.length, 1);
  assert.ok(testVisibleWidth(lines[0]) <= 91, `visible width ${testVisibleWidth(lines[0])} > 91`);
  assert.match(lines[0], /asgrep/);
  assert.match(lines[0], /\.\.\./);
});

test("AsgrepText keeps short lines unchanged", () => {
  const component = presentText('asgrep  ·  search  ·  "auth"  ·  natural', undefined);
  assert.deepEqual(component.render(91), ['asgrep  ·  search  ·  "auth"  ·  natural']);
});
