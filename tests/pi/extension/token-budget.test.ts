/**
 * Token budget ratchet for the model-visible surface.
 *
 * Every character counted here is sent to the model: tool definitions ride in
 * the system prompt of every request, and result text is re-sent with the whole
 * transcript on each turn. The budgets are the measured floor after the lean
 * pass (BPE-calibrated with cl100k_base: 4.0 chars/token on this text);
 * raising one is a deliberate decision, not an accident.
 */
import assert from "node:assert/strict";
import test from "node:test";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerAstSgrepTools } from "../../../packages/pi/extension/src/index.js";
import { formatCodemodeResult, formatEditResult, formatReadResult, formatSearchResult } from "../../../packages/pi/extension/src/ui/present.js";

type Registered = {
  name: string;
  description?: string;
  promptSnippet?: string;
  promptGuidelines?: string[];
  parameters?: unknown;
};

function registered(): Registered[] {
  const tools: Registered[] = [];
  // Emulate a normal Pi host (built-in read/edit active) so the budget
  // measures the common path, including the host-aware guideline variant.
  const pi = { registerTool(tool: Registered) { tools.push(tool); }, on() {}, getActiveTools: () => ["read", "edit"] } as unknown as ExtensionAPI;
  registerAstSgrepTools(pi, { run: async () => ({ tool: "asgrep", schema_version: "1.0.0", ok: true }) }, { ensureFresh: async () => "", markAffectedPath() {} });
  return tools;
}

const toolCost = (tool: Registered): number =>
  (tool.name?.length ?? 0) +
  (tool.description?.length ?? 0) +
  JSON.stringify(tool.parameters ?? {}).length +
  (tool.promptSnippet?.length ?? 0) +
  (tool.promptGuidelines?.join("\n").length ?? 0);

test("tool definitions stay inside the always-on token budget", () => {
  const tools = registered();
  const perTool = Object.fromEntries(tools.map((tool) => [tool.name, toolCost(tool)]));
  const total = Object.values(perTool).reduce((sum, cost) => sum + cost, 0);
  // 750 BPE tokens measured (1714 before the first lean pass, 902 before the
  // second: deduped instructions, host-aware guidelines, schema trims).
  assert.ok(total <= 2999, `static tool surface grew: ${total} chars (${JSON.stringify(perTool)})`);
  assert.ok(perTool.asgrep <= 1061, `asgrep schema grew: ${perTool.asgrep}`);
  assert.deepEqual(tools.map((tool) => tool.name), ["asgrep", "asgrep_search", "asgrep_edit", "asgrep_read", "asgrep_index"]);

  // On a host that already ships read/edit built in, the one-shot duplicates
  // are inactive: pi sends only active tools, so the effective surface is the
  // three unique tools (479 BPE tokens measured, 750 with everything active).
  const unique = ["asgrep", "asgrep_search", "asgrep_index"].reduce((sum, name) => sum + perTool[name], 0);
  assert.ok(unique <= 1916, `unique-tool surface grew: ${unique} chars`);
});

const HITS = Array.from({ length: 8 }, (_, index) => ({
  file: `crates/ast-sgrep-core/src/search/mod.rs`,
  start_line: 100 + index * 7,
  symbol: `symbol_${index}`,
  kind: "def",
  preview: `pub fn symbol_${index}(args: &SearchOptions) -> Result<Vec<SearchHit>> {`,
}));

test("result text stays inside the per-call token budget", () => {
  const search = formatSearchResult({ hits: HITS }, { command: "search" });
  const empty = formatSearchResult({ hits: [], suggested_next: ["defs:Foo", "callers:Foo"] }, { command: "search" });
  const read = formatReadResult({ windows: [{ path: "src/lib.rs", start: 1, end: 40, text: Array.from({ length: 40 }, (_, i) => `line ${i} of the window`).join("\n") }] });
  const edit = formatEditResult({ edits: [{ path: "src/lib.rs", line: 12, changed: true }] });
  // Explicit Code Mode returns retain the complete hit data, unlike the lean
  // one-shot search summary. Their small-payload budget includes that data.
  const codemode = formatCodemodeResult({ hits: [{ file: "a.rs", symbol: "x" }] }, { stats: { calls: 3, batchedCalls: 0, parallelSpawnCalls: 0, stickyCalls: 3, waves: 1 }, wallMs: 12, backend: "napi" });
  // Floors measured when the budget was set (BPE tokens: search 227, empty 18,
  // read 220, edit 12). Code Mode now budgets complete selected data instead
  // of the old 24-character lossy hit row.
  assert.ok(search.length <= 1030, `search content grew: ${search.length} chars`);
  assert.equal(search.split("\n")[0], "search: 8 hits");
  assert.ok(empty.length <= 49, `empty-result content grew: ${empty.length}`);
  assert.ok(read.length <= 887, `read content grew: ${read.length}`);
  assert.ok(edit.length <= 33, `edit content grew: ${edit.length}`);
  assert.ok(codemode.length <= 128, `codemode content grew: ${codemode.length}`);
  assert.ok(codemode.includes(JSON.stringify([{ file: "a.rs", symbol: "x" }], null, 2)));
  // Excerpts are only present when the caller asked (excerptLines): the budget
  // bounds that explicitly requested growth.
  const withExcerpts = formatSearchResult(
    { hits: HITS.map((hit) => ({ ...hit, excerpt: Array.from({ length: 6 }, (_, i) => `    body line ${i} of the excerpt`).join(String.fromCharCode(10)) })) },
    { command: "search", excerptLines: 6 },
  );
  // Capsules carry excerpts whether or not they were requested: without the
  // flag they must be ignored (a defs answer was paying 2.5k tokens for them).
  const unrequested = formatSearchResult(
    { hits: HITS.map((hit) => ({ ...hit, excerpt: "body text nobody asked for" })) },
    { command: "search" },
  );
  assert.equal(unrequested.length, formatSearchResult({ hits: HITS }, { command: "search" }).length);
  assert.ok(withExcerpts.length <= 2710, `excerpt content grew: ${withExcerpts.length}`);
  assert.match(withExcerpts, /body line 0 of the excerpt/);
});

