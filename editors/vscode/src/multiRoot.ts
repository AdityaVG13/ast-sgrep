/**
 * Multi-root folder binding and hit path resolve.
 * Pure helpers (no vscode module) so node tests can exercise the same rules
 * the extension uses when opening search hits.
 */
import * as path from 'path';

export interface FolderLike {
  name: string;
  fsPath: string;
}

/** Pick the workspace folder that owns a document path; fail closed in multi-root. */
export function folderForUriPath(
  documentPath: string | undefined,
  folders: FolderLike[],
): FolderLike | undefined {
  if (!folders.length) return undefined;
  if (documentPath) {
    const normalized = path.resolve(documentPath);
    const match = folders.find((f) => {
      const root = path.resolve(f.fsPath);
      return normalized === root || normalized.startsWith(root + path.sep);
    });
    if (match) return match;
  }
  return folders.length === 1 ? folders[0] : undefined;
}

/**
 * Resolve a hit file against the preferred search folder, then other roots.
 * Absolute paths pass through. Missing files fall back to preferred join
 * (never silently prefer folders[0] when preferred misses).
 */
export function resolveHitPath(
  file: string,
  preferred: FolderLike,
  folders: FolderLike[],
  exists: (p: string) => boolean,
): string {
  if (path.isAbsolute(file)) return file;
  const ordered = [preferred, ...folders.filter((f) => f.fsPath !== preferred.fsPath)];
  for (const folder of ordered) {
    const candidate = path.join(folder.fsPath, file);
    if (exists(candidate)) return candidate;
  }
  return path.join(preferred.fsPath, file);
}

export function hitFilePath(hit: {
  path?: string;
  file?: string;
  file_path?: string;
}): string | undefined {
  return hit.path ?? hit.file_path ?? hit.file;
}

export function hitLineNumber(hit: {
  line?: number;
  start_line?: number;
  line_start?: number;
}): number {
  return hit.line_start ?? hit.start_line ?? hit.line ?? 1;
}
