import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const extensionDir = resolve(dirname(fileURLToPath(import.meta.url)), "../../extension");

test("packaged skill discloses access, data lifecycle, external embeddings, and privacy", () => {
  const skill = readFileSync(join(extensionDir, "skills/ast-sgrep/SKILL.md"), "utf8");
  for (const disclosure of [
    /full system access/iu,
    /not a sandbox/iu,
    /writes `\.asgrep` data inside the project/iu,
    /package removal preserves that project data/iu,
    /uses no telemetry or credentials/iu,
    /external embeddings provider may send source text and queries/iu,
  ]) assert.match(skill, disclosure);
});

test("published extension runtime has no telemetry, credential integration, or network downloader", () => {
  const forbidden = /(fetch\s*\(|https?:\/\/|API_KEY|PASSWORD|SECRET|process\.env\.(?:TOKEN|KEY|CREDENTIAL)|telemetry|analytics|sentry|opentelemetry)/iu;
  for (const relative of ["dist/index.js", "dist/runtime.js"]) {
    assert.doesNotMatch(readFileSync(join(extensionDir, relative), "utf8"), forbidden, relative);
  }
});
