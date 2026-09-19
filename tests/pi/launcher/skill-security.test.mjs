import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const extensionDir = resolve(dirname(fileURLToPath(import.meta.url)), "../../../packages/pi/extension");
/** Recursively list files under dir matching suffix. */
function walkSources(dir, suffix) {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...walkSources(full, suffix));
    else if (entry.name.endsWith(suffix)) out.push(full);
  }
  return out;
}
const relativePath = (base, full) => relative(base, full);

test("published extension README discloses access, data lifecycle, and local-only embeddings", () => {
  const readme = readFileSync(join(extensionDir, "README.md"), "utf8");
  for (const disclosure of [
    /full OS-user access|permissions of the OS user/iu,
    /not an operating-system security boundary|not a sandbox/iu,
    /\.asgrep\//iu,
    /Removal preserves|preserves each project's/iu,
    /no telemetry/iu,
    /never send source text to a remote embedding API/iu,
  ]) assert.match(readme, disclosure);
});

test("published extension runtime has no telemetry, credential integration, or network downloader", () => {
  const forbidden = /(fetch\s*\(|https?:\/\/|API_KEY|PASSWORD|SECRET|process\.env\.(?:TOKEN|KEY|CREDENTIAL)|telemetry|analytics|sentry|opentelemetry)/iu;
  for (const relative of walkSources(join(extensionDir, "dist"), ".js").map((p) => relativePath(extensionDir, p))) {
    assert.doesNotMatch(readFileSync(join(extensionDir, relative), "utf8"), forbidden, relative);
  }
});
