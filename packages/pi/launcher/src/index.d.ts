export interface ResolveBinaryOptions {
  binaryPath?: string;
  env?: Readonly<Record<string, string | undefined>>;
  platform?: NodeJS.Platform;
  arch?: string;
  libc?: string;
  requireResolve?: (specifier: string) => string;
  fs?: Pick<typeof import("node:fs"), "accessSync" | "existsSync" | "readFileSync" | "statSync">;
}

export interface ResolveCodemodeAddonOptions extends ResolveBinaryOptions {
  addonPath?: string;
}

export declare class AstSgrepBinaryError extends Error {
  readonly code: string;
  readonly path?: string;
}

export declare const NAPI_ADDON: "ast-sgrep-codemode.node";

export declare function resolveBinary(options?: ResolveBinaryOptions): string;

/** Resolve the platform-packaged Code Mode NAPI addon, or null when absent. */
export declare function resolveCodemodeAddon(options?: ResolveCodemodeAddonOptions): string | null;
