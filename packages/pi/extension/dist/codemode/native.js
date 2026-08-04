/**
 * Load the in-process Code Mode NAPI addon.
 *
 * Same model as MCP: Rust `CodeModeSession` runs inside the Node process.
 * No `asgrep` CLI spawn on the hot path.
 *
 * Resolution order:
 * 1. `ASGREP_CODEMODE_NAPI_PATH` (dev override)
 * 2. `@ast-sgrep/<platform>/ast-sgrep-codemode.node` via launcher (release install)
 * 3. Local `extension/native/` / cargo `target/release` (dev builds)
 */
import { createRequire } from "node:module";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
let cached;
function platformTriple() {
    const { platform, arch } = process;
    if (platform === "linux" && arch === "x64")
        return "linux-x64-gnu";
    if (platform === "linux" && arch === "arm64")
        return "linux-arm64-gnu";
    if (platform === "darwin" && arch === "arm64")
        return "darwin-arm64";
    if (platform === "darwin" && arch === "x64")
        return "darwin-x64";
    if (platform === "win32" && arch === "x64")
        return "win32-x64-msvc";
    return null;
}
function platformPackageAddon() {
    const require = createRequire(import.meta.url);
    try {
        const launcher = require("ast-sgrep");
        if (typeof launcher.resolveCodemodeAddon === "function") {
            return launcher.resolveCodemodeAddon();
        }
    }
    catch {
        // Launcher may be unavailable in isolated unit tests.
    }
    return null;
}
function candidatePaths() {
    const here = dirname(fileURLToPath(import.meta.url));
    const triple = platformTriple();
    const names = triple
        ? [
            `ast-sgrep-codemode.${triple}.node`,
            `ast-sgrep-codemode.node`,
        ]
        : [`ast-sgrep-codemode.node`];
    const dirs = [
        // Built next to extension (dev / packaged)
        join(here, "..", "..", "native"),
        join(here, "..", "native"),
        // Workspace release output
        join(here, "..", "..", "..", "..", "target", "release"),
    ];
    const out = [];
    const override = process.env.ASGREP_CODEMODE_NAPI_PATH;
    if (override)
        out.push(override);
    const packaged = platformPackageAddon();
    if (packaged)
        out.push(packaged);
    for (const dir of dirs) {
        for (const name of names)
            out.push(join(dir, name));
        // cargo cdylib name
        out.push(join(dir, "libast_sgrep_codemode_napi.so"));
        out.push(join(dir, "libast_sgrep_codemode_napi.dylib"));
        out.push(join(dir, "ast_sgrep_codemode_napi.dll"));
    }
    return out;
}
/** Load the NAPI binding once. Returns null if unavailable on this host. */
export function loadCodemodeNative() {
    if (cached !== undefined)
        return cached;
    // Force CLI sticky / argv path (unit tests, degraded installs).
    if (process.env.ASGREP_CODEMODE_BACKEND === "cli") {
        cached = null;
        return null;
    }
    const require = createRequire(import.meta.url);
    for (const path of candidatePaths()) {
        if (!existsSync(path))
            continue;
        try {
            const binding = require(path);
            if (typeof binding?.isNative === "function" && binding.isNative()) {
                cached = binding;
                return cached;
            }
        }
        catch {
            // try next candidate
        }
    }
    cached = null;
    return null;
}
export function nativeAvailable() {
    return loadCodemodeNative() !== null;
}
/** Reset cache (tests). */
export function resetNativeCache() {
    cached = undefined;
}
