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
