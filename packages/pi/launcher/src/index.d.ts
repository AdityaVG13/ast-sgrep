export interface ResolveBinaryOptions {
  binaryPath?: string;
  env?: Readonly<Record<string, string | undefined>>;
  platform?: NodeJS.Platform;
  arch?: string;
  libc?: string;
  requireResolve?: (specifier: string) => string;
  fs?: Pick<typeof import("node:fs"), "accessSync" | "readFileSync" | "statSync">;
}

export declare class AstSgrepBinaryError extends Error {
  readonly code: string;
  readonly path?: string;
}

export declare function resolveBinary(options?: ResolveBinaryOptions): string;
