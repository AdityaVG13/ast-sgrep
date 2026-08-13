/**
 * ktog: schema modes ⊆ tested modes ⊆ skill docs.
 * Mirrors packages/pi/extension/src/index.ts searchArgs/queryForMode without TS deps.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..");
const indexTs = readFileSync(path.join(root, "packages/pi/extension/src/index.ts"), "utf8");
const skill = readFileSync(path.join(root, "packages/pi/extension/skills/ast-sgrep/SKILL.md"), "utf8");
const guide = readFileSync(
  path.join(root, "packages/pi/extension/skills/ast-sgrep/references/query-guide.md"),
  "utf8",
);

const SCHEMA_MODES = [
  "natural",
  "pattern",
  "defs",
  "callers",
  "chain",
  "semantic",
  "word",
  "literal",
  "regex",
  "imports",
];

function queryForMode(query, mode) {
  if (
    mode === "pattern" ||
    mode === "defs" ||
    mode === "callers" ||
    mode === "word" ||
    mode === "literal" ||
    mode === "regex" ||
    mode === "imports"
  ) {
    return `${mode}: ${query}`;
  }
  return query;
}

function searchArgs(params) {
  const mode = params.mode ?? "natural";
  const query = queryForMode(params.query, mode);
  const output = [
    "--json",
    "--format",
    "agent-capsule",
    "--limit",
    String(params.limit ?? 8),
    "--excerpt-lines",
    String(params.excerptLines ?? 0),
  ];
  return mode === "chain" || mode === "semantic"
    ? [mode, query, ".", ...output]
    : [...output, query, "."];
}

test("schema mode literals are declared in extension source", () => {
  for (const mode of SCHEMA_MODES) {
    assert.match(indexTs, new RegExp(`Type\\.Literal\\("${mode}"\\)`), mode);
  }
});

test("every schema mode has argv routing coverage", () => {
  const cases = {
    natural: ["needle", "."],
    pattern: ["pattern: needle", "."],
    defs: ["defs: needle", "."],
    callers: ["callers: needle", "."],
    chain: ["chain", "needle", ".", "--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3"],
    semantic: ["semantic", "needle", ".", "--json", "--format", "agent-capsule", "--limit", "25", "--excerpt-lines", "3"],
    word: ["word: needle", "."],
    literal: ["literal: needle", "."],
    regex: ["regex: needle", "."],
    imports: ["imports: needle", "."],
  };
  for (const mode of SCHEMA_MODES) {
    const args = searchArgs({ query: "needle", mode, limit: 25, excerptLines: 3 });
    assert.deepEqual(args.slice(-cases[mode].length), cases[mode], mode);
  }
});

test("skill docs mention every schema mode", () => {
  for (const mode of SCHEMA_MODES) {
    assert.match(skill + "\n" + guide, new RegExp(`\\b${mode}\\b`), mode);
  }
});
