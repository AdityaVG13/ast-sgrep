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

function pathContained(root: string, candidate: string): boolean {
  const rel = path.relative(root, candidate);
  return (
    rel === '' ||
    (rel !== '..' && !rel.startsWith(`..${path.sep}`) && !path.isAbsolute(rel))
  );
}

/** Pick the workspace folder that owns a document path; fail closed in multi-root. */
export function folderForUriPath(
  documentPath: string | undefined,
  folders: FolderLike[],
): FolderLike | undefined {
  if (!folders.length) return undefined;
  if (documentPath) {
    const document = path.resolve(documentPath);
    let best: { folder: FolderLike; root: string } | undefined;
    for (const folder of folders) {
      const root = path.resolve(folder.fsPath);
      const contains = pathContained(root, document);
      if (contains && (!best || root.length > best.root.length)) {
        best = { folder, root };
      }
    }
    if (best) return best.folder;
  }
  return folders.length === 1 ? folders[0] : undefined;
}

/** Resolve a hit only inside the folder whose language server returned it. */
export function resolveHitPath(file: string, preferred: FolderLike): string {
  const root = path.resolve(preferred.fsPath);
  const candidate = path.resolve(root, file);
  if (!pathContained(root, candidate)) {
    throw new Error(`ast-sgrep hit path is outside workspace root: ${file}`);
  }
  return candidate;
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
