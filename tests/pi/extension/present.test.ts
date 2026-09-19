import assert from "node:assert/strict";
import test from "node:test";
import {
  displayWidth,
  formatCodemodeResult,
  formatSearchResult,
  formatStatusResult,
  sanitizeContent,
} from "../../../packages/pi/extension/src/ui/present.js";
import { renderAsgrepResult } from "../../../packages/pi/extension/src/ui/card.js";

/** Painted theme: the card must measure around SGR codes, not through them, and
 * a box background must not change any line's width. */
const THEME = {
  bold: (text: string) => `\u001b[1m${text}\u001b[22m`,
  fg: (_role: string, text: string) => `\u001b[38;2;120;120;120m${text}\u001b[39m`,
  bg: (_role: string, text: string) => `\u001b[48;2;30;46;30m${text}\u001b[49m`,
};

test("search result text is the payload: one header line plus hit rows", () => {
  const text = formatSearchResult(
    { hits: [{ file: "src/auth.rs", start_line: 42, symbol: "refresh_token", kind: "function" }] },
    { command: "search" },
  );
  // Lean by design: query/mode/timing/backend live in the tool call and the
  // TUI card, not in the transcript copy of every result.
  assert.equal(text.split("\n")[0], "search: 1 hit");
  assert.match(text, /src\/auth\.rs:42 refresh_token {2}function/);
  assert.doesNotMatch(text, /\{"hits"/);
  assert.doesNotMatch(text, /asgrep|napi|ms$/);
});

test("empty search results include a recovery hint", () => {
  const withNext = formatSearchResult(
    { hits: [], suggested_next: ["callers:Foo", "defs:Foo"] },
    { command: "search" },
  );
  assert.equal(withNext.split("\n")[0], "search: 0 hits");
  assert.match(withNext, /try: callers:Foo/);
  const withoutNext = formatSearchResult({ hits: [] }, { command: "search" });
  assert.equal(withoutNext, "search: 0 hits");
});

test("undefined Code Mode result tells the model to return", () => {
  const text = formatCodemodeResult(undefined, { wallMs: 1, backend: "napi" });
  assert.match(text, /no return statement/);
});

test("codemode result uses hit rows when the program returned hits", () => {
  const text = formatCodemodeResult(
    { hits: [{ path: "src/a.ts", line: 3, symbol: "ensureFresh" }] },
    { wallMs: 2, backend: "napi" },
  );
  assert.equal(text.split("\n")[0], "codemode: 1 hit");
  assert.match(text, /src\/a\.ts:3 ensureFresh/);
});

test("codemode result lists shaped keys instead of dumping JSON", () => {
  const text = formatCodemodeResult({ symbol: "refresh_token", n: 2 }, { stats: { calls: 2, batchedCalls: 0, parallelSpawnCalls: 0, stickyCalls: 2, waves: 1 }, wallMs: 3, backend: "napi" });
  assert.equal(text.split("\n")[0], "codemode: 2 calls");
  assert.match(text, /symbol: refresh_token/);
  assert.match(text, /n: 2/);
  assert.doesNotMatch(text, /\{"symbol"/);
  assert.doesNotMatch(text, /in-process|napi|3ms/);
});

test("status result is one lean line", () => {
  const text = formatStatusResult({ ok: true, status: "ready", counts: { files: 12, symbols: 34 }, backend: "fastembed" });
  assert.equal(text, "status: ready files=12 symbols=34 fastembed");
});

// --- Card width contract -----------------------------------------------------
// pi kills the process when any rendered line exceeds the terminal width
// ("Rendered line N exceeds terminal width (94 > 91)"). The card's frame math
// is only as good as the width function behind it, so these tests measure every
// rendered line with an oracle that mirrors pi's rules and pin the oracle to
// widths measured from @earendil-works/pi-tui 0.85.1 visibleWidth.

const CHROME_CELL = /^[\u00b7\u00d7\u2022\u2026\u2192\u23f5\u23f8\u2500-\u257f\u25a0-\u25cf\u2591-\u2593\u26d3\u2713\u2714\u2717\u276f\u2588]$/u;
const CELL_SEGMENTER = new Intl.Segmenter(undefined, { granularity: "grapheme" });

function stripTerminalSequences(text: string): string {
  return text
    .replace(/\u001b\[[0-9;?]*[ -/]*[@-~]/gu, "")
    .replace(/\u001b\][^\u0007\u001b]*(?:\u0007|\u001b\\)/gu, "")
    .replace(/\u001b[PX^_][^\u001b]*\u001b\\/gu, "");
}

function isZeroWidthCell(code: number): boolean {
  return (code >= 0x0300 && code <= 0x036f)
    || (code >= 0x200b && code <= 0x200f)
    || (code >= 0x2060 && code <= 0x206f)
    || (code >= 0xfe00 && code <= 0xfe0f)
    || code === 0x00ad
    || code === 0x034f
    || code === 0x061c
    || code === 0x180e
    || code === 0xfeff;
}

/**
 * Width in terminal cells, mirroring pi: tabs are three, wide/emoji are two,
 * combining/VS/ZWSP/ZWJ are zero. Skin-tone modifiers are zero once a base
 * carries the cluster and two when they stand alone (pi widens a lone one).
 */
function piWidth(text: string): number {
  let width = 0;
  for (const { segment } of CELL_SEGMENTER.segment(stripTerminalSequences(text))) {
    if (segment === "\t") { width += 3; continue; }
    let base;
    let loneSkinTone = false;
    for (const character of segment) {
      const code = character.codePointAt(0) ?? 0;
      if (code <= 0x1f || (code >= 0x7f && code <= 0x9f) || isZeroWidthCell(code)) continue;
      if (code >= 0x1f3fb && code <= 0x1f3ff) { loneSkinTone = true; continue; }
      base = character;
      break;
    }
    if (base === undefined) { width += loneSkinTone ? 2 : 0; continue; }
    const code = base.codePointAt(0) ?? 0;
    if (code <= 0x7e) { width += 1; continue; }
    width += CHROME_CELL.test(base) ? 1 : 2;
  }
  return width;
}

test("the width oracle matches pi's visibleWidth on the glyphs the card draws", () => {
  const measuredInPi: Array<[string, number]> = [
    ["\u2026", 1], ["\u00b7", 1], ["\u2713", 1], ["\u2717", 1], ["\u00d7", 1],
    ["\u2502", 1], ["\u2500", 1], ["\u256d", 1], ["\u256e", 1], ["\u2570", 1], ["\u256f", 1],
    ["\u2022", 1], ["\u25cf", 1], ["\u26d3", 1], ["\u276f", 1], ["\u2192", 1],
    ["\u25a0", 1], ["\u2588", 1], ["\u2591", 1], ["\u23f5", 1], ["\u23f8", 1],
    ["\u00ad", 0], ["\u0301", 0], ["\ufe0f", 0], ["\u200b", 0],
    ["\t", 3], ["x\ty", 5], ["\u4e2d\u6587", 4], ["\uff21\uff22", 4],
    ["\ud83d\ude42", 2], ["\ud83d\udc68\u200d\ud83d\udc69\u200d\ud83d\udc67", 2], ["\ud83c\udffb", 2], ["\ud83c\uddfa\ud83c\uddf8", 2],
  ];
  for (const [text, width] of measuredInPi) {
    assert.equal(piWidth(text), width, "oracle drifted from pi for " + JSON.stringify(text));
  }
});

const TABS_AND_LONG_CODE = [
  "\t\tif (lines.length < 24) lines.push(parsed);",
  "\t}",
  "",
  "\tif (lines.length === 0) return undefined;",
  "\treturn { added: counts.added, removed: counts.removed, lines, displayLineCount: counts.displayLineCount }; // long tail",
  "\tconst label = \"\u4e2d\u6587\u6807\u7b7e \ud83d\ude42\"; // wide glyphs in content",
].join("\n");

const CARD_MODELS: Array<[string, Parameters<typeof renderAsgrepResult>[0]]> = [
  ["read window with tab-indented code", {
    content: [{ type: "text", text: "read" }],
    details: {
      ok: true, command: "read", backend: "napi",
      response: {
        tool: "asgrep", schema_version: "1.0.0", ok: true,
        windows: [{ path: "packages/pi-supernova/src/ui/render.js", start: 290, end: 385, text: TABS_AND_LONG_CODE }],
      },
    },
  }],
  ["edit diff with long removed and added lines", {
    content: [{ type: "text", text: "edit" }],
    details: {
      ok: true, command: "edit",
      result: {
        ok: true,
        edits: [{
          path: "crates/ast-sgrep-core/src/store/sqlite/mod.rs",
          line: 4_096,
          removed: ["\t\tif (op.ok === false) return theme.fg(\"error\", \"\u00d7\"); // removed line that runs long"],
          added: ["\t\tif (op.mutationAttempt) return theme.fg(\"warning\", \"\u00b7\"); // added line that runs long"],
        }],
      },
    },
  }],
  ["search hits with long paths and labels", {
    content: [{ type: "text", text: "hits" }],
    details: {
      ok: true, command: "search", backend: "napi", activationMs: 12.5,
      response: {
        tool: "asgrep", schema_version: "1.0.0", ok: true,
        hits: [
          { file: "crates/ast-sgrep-core/src/search/fusion.rs", start_line: 1_234, symbol: "combine_two_search_channels_with_a_long_name", kind: "function" },
          { file: "\u4e2d\u6587/\u8def\u5f84/\ud83d\ude42.rs", start_line: 7, symbol: "\u51fd\u6570\u540d", kind: "function" },
        ],
      },
    },
  }],
  ["codemode trace with long targets and an error", {
    content: [{ type: "text", text: "codemode" }],
    details: {
      ok: false, command: "codemode",
      error: { message: "codemode failed [UNEXPECTED_ERROR]: " + "x".repeat(400) },
      trace: [{ tool: "search", target: "a very long query string that keeps going and going", ok: true, ms: 1_234 }],
    },
  }],
  ["codemode result lines with wide glyphs", {
    content: [{ type: "text", text: "codemode" }],
    details: { ok: true, command: "codemode", result: { summary: "\u4e2d\u6587 summary " + "y".repeat(200), hits: 3 } },
  }],
];

for (const [label, result] of CARD_MODELS) {
  test("card stays inside the terminal width: " + label, () => {
    for (const expanded of [false, true]) {
      const card = renderAsgrepResult(result, { expanded }, THEME);
      for (const width of [40, 60, 80, 91, 100, 120, 200]) {
        const lines = card.render(width);
        assert.ok(lines.length > 0, "card rendered nothing");
        for (const [index, line] of lines.entries()) {
          const measured = piWidth(line);
          assert.ok(
            measured <= width,
            "line " + index + " of " + JSON.stringify(label) + " is " + measured + " cells at width " + width + ": " + JSON.stringify(stripTerminalSequences(line)),
          );
        }
        assert.equal(piWidth(lines[0]!), piWidth(lines[lines.length - 1]!), "top and bottom frame bars must agree");
      }
    }
  });
}


test("no random card content overflows the terminal width (seeded fuzz)", () => {
  let seed = 0x2f6e2b1;
  const rand = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
  const pick = <T,>(list: T[]): T => list[Math.floor(rand() * list.length)]!;
  const alphabet = ["a", "B", "/", ".", "-", "_", " ", "\t", "\t\t", "\u4e2d", "\ud83d\ude42", "\u2026", "\u2713", "\u2502", "\u001b[31m", "\u001b[0m", "\u0301", "\ud83c\udffb"];
  const randomLine = () => Array.from({ length: Math.floor(rand() * 200) }, () => pick(alphabet)).join("");

  for (let round = 0; round < 40; round += 1) {
    const body = Array.from({ length: 1 + Math.floor(rand() * 6) }, randomLine);
    const kind = Math.floor(rand() * 3);
    const result = kind === 0
      ? { content: [{ type: "text", text: "read" }], details: { ok: true, command: "read", response: { tool: "asgrep", schema_version: "1.0.0", ok: true, windows: [{ path: randomLine().slice(0, 40), start: 1, end: 9, text: body.join("\n") }] } } }
      : kind === 1
        ? { content: [{ type: "text", text: "edit" }], details: { ok: true, command: "edit", result: { ok: true, edits: [{ path: randomLine().slice(0, 40), line: 3, removed: body, added: body }] } } }
        : { content: [{ type: "text", text: "hits" }], details: { ok: true, command: "search", response: { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: body.map((b) => ({ file: b, start_line: 1, symbol: b })) } } };
    const card = renderAsgrepResult(result, { expanded: round % 2 === 0 }, THEME);
    for (const width of [20, 61, 80, 91, 137]) {
      for (const [index, rendered] of card.render(width).entries()) {
        const measured = piWidth(rendered);
        assert.ok(
          measured <= width,
          "round " + round + ": line " + index + " is " + measured + " cells at width " + width + ": " + JSON.stringify(stripTerminalSequences(rendered)),
        );
      }
    }
  }
});


test("an unterminated escape cannot shrink a measured width", () => {
  // pi measures ESC as zero and the remainder as text, so skipping a broken
  // sequence whole under-counts and lets the row overflow the terminal.
  assert.equal(piWidth("before\u001b[0"), 8);
  assert.equal(piWidth("tail\u001b[38;2"), 9);
  assert.ok(displayWidth("before\u001b[0") >= 8, "displayWidth must not skip unterminated sequences");
  assert.ok(displayWidth("tail\u001b[38;2") >= 9);
  // Content is sanitized before the card paints it, so raw escapes never reach
  // the terminal inside our own SGR spans.
  assert.equal(sanitizeContent("a\u001b[31mb\u001b[0m"), "ab");
  assert.equal(sanitizeContent("keep\ttabs\nand\u0007controls"), "keep\ttabs\nandcontrols");
});


test("a card stands alone: one frame, one 'asgrep', full width", () => {
  const result = {
    content: [{ type: "text", text: "read" }],
    details: {
      ok: true, command: "read", backend: "napi", activationMs: 9,
      response: {
        tool: "asgrep", schema_version: "1.0.0", ok: true,
        windows: [{ path: "packages/pi-supernova/index.js", start: 1, end: 340, text: "import { createRequire } from \"node:module\";" }],
      },
    },
  };
  const lines = renderAsgrepResult(result, { expanded: false }, THEME).render(140);
  const plain = lines.map(stripTerminalSequences);
  // No call line above the box: the title lives in the top rule, once.
  assert.equal((plain.join("\n").match(/asgrep/gu) ?? []).length, 1, plain.join("\n"));
  assert.ok(plain[0]!.startsWith("\u256d"), plain[0]);
  assert.ok(plain[0]!.includes("asgrep"), plain[0]);
  // renderShell "self": the frame must span every column pi hands us, or the
  // host background shows as a band beside the border.
  for (const line of lines) assert.equal(piWidth(line), 140, JSON.stringify(stripTerminalSequences(line)));
});

test("a codemode body summarizes the envelope instead of dumping it", () => {
  const result = {
    content: [{ type: "text", text: "codemode" }],
    details: {
      ok: true, command: "codemode", wallMs: 27, backend: "napi",
      stats: { calls: 1, batchedCalls: 0, parallelSpawnCalls: 0, stickyCalls: 1, waves: 1 },
      trace: [{ tool: "read", target: "packages/pi-supernova/index.js#L1-L340", ok: true, ms: 26 }],
      result: {
        count: 3, ok: true, tool: "asgrep", command: "read", schema_version: "1.0.0",
        windows: [{ end: 340, path: "packages/pi-supernova/index.js", ref: "packages/pi-supernova/index.js#L1-L340" }],
      },
    },
  };
  const text = renderAsgrepResult(result, { expanded: false }, THEME).render(96).map(stripTerminalSequences).join("\n");
  assert.match(text, /1 window {2}·|1 window/, text);
  for (const noise of ["tool: asgrep", "command: read", "schema_version", "ok: true", "windows: ["]) {
    assert.ok(!text.includes(noise), "transport field leaked into the card: " + noise + "\n" + text);
  }
});


test("a card shows the notes that qualify the answer", () => {
  const result = {
    content: [{ type: "text", text: "search" }],
    details: {
      ok: true,
      command: "search",
      notes: ["index has 0 files: this repository is not indexed -- run /asgrep-index (or asgrep.indexRepo()) and retry"],
      response: { tool: "asgrep", schema_version: "1.0.0", ok: true, hits: [] },
    },
  };
  const text = renderAsgrepResult(result, { expanded: false }, THEME).render(96).map(stripTerminalSequences).join("\n");
  assert.match(text, /! index has 0 files/, text);
});

