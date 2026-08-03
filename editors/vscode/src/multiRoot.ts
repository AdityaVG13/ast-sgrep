/**
 * Pure helpers mirrored from extension.ts for multi-root path binding tests
 * without loading the vscode module in node:test.
 */
import * as path from 'path';

export interface FolderLike {
  name: string;
  fsPath: string;
}

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
