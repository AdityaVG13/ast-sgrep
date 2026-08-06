import * as vscode from 'vscode';
import { Executable, LanguageClient, LanguageClientOptions, ServerOptions } from 'vscode-languageclient/node';
import {
  folderForUriPath,
  hitFilePath,
  hitLineNumber,
  resolveHitPath,
  type FolderLike,
} from './multiRoot';

interface SearchHit {
  path?: string;
  file?: string;
  file_path?: string;
  excerpt?: string;
  line?: number;
  start_line?: number;
  line_start?: number;
  column?: number;
  start_column?: number;
}
interface SearchResponse { hits: SearchHit[]; }
interface SearchQuickPickItem extends vscode.QuickPickItem {
  hit: SearchHit;
  folder: vscode.WorkspaceFolder;
}

/** Exported for unit tests — pick the workspace folder that owns a document URI. */
export function folderForUri(
  uri: vscode.Uri | undefined,
  folders: readonly vscode.WorkspaceFolder[] | undefined,
): vscode.WorkspaceFolder | undefined {
  if (!folders?.length) return undefined;
  if (uri) {
    const match = vscode.workspace.getWorkspaceFolder(uri);
    if (match) return match;
  }
  // Same fail-closed rule as multiRoot.folderForUriPath (single-root only without a doc).
  const picked = folderForUriPath(uri?.fsPath, toFolderLikes(folders));
  return picked ? folders.find((f) => f.uri.fsPath === picked.fsPath) : undefined;
}

function toFolderLikes(folders: readonly vscode.WorkspaceFolder[]): FolderLike[] {
  return folders.map((f) => ({ name: f.name, fsPath: f.uri.fsPath }));
}

const DOCUMENT_SELECTOR: LanguageClientOptions['documentSelector'] = [
  { scheme: 'file', language: 'rust' },
  { scheme: 'file', language: 'python' },
  { scheme: 'file', language: 'typescript' },
  { scheme: 'file', language: 'typescriptreact' },
  { scheme: 'file', language: 'javascript' },
  { scheme: 'file', language: 'javascriptreact' },
  { scheme: 'file', language: 'go' },
  { scheme: 'file', language: 'java' },
  { scheme: 'file', language: 'ruby' },
  { scheme: 'file', language: 'csharp' },
];

const clients = new Map<string, LanguageClient>();

function folderKey(folder: vscode.WorkspaceFolder): string {
  return folder.uri.toString();
}

function clientOptionsFor(folder: vscode.WorkspaceFolder, indexPath: string): LanguageClientOptions {
  return {
    documentSelector: DOCUMENT_SELECTOR,
    workspaceFolder: folder,
    initializationOptions: { asgrep: indexPath ? { indexPath } : {} },
  };
}

async function startClientForFolder(
  folder: vscode.WorkspaceFolder,
  serverPath: string,
  indexPath: string,
): Promise<LanguageClient> {
  const key = folderKey(folder);
  const existing = clients.get(key);
  if (existing) return existing;
  const executable: Executable = {
    command: serverPath,
    args: ['--stdio'],
    options: { cwd: folder.uri.fsPath },
  };
  const serverOptions: ServerOptions = executable;
  const client = new LanguageClient(
    `asgrep-${folder.name}`,
    `ast-sgrep Language Server (${folder.name})`,
    serverOptions,
    clientOptionsFor(folder, indexPath),
  );
  clients.set(key, client);
  await client.start();
  return client;
}

async function stopClient(key: string): Promise<void> {
  const client = clients.get(key);
  if (!client) return;
  clients.delete(key);
  await client.stop();
}

async function reconcileClients(): Promise<void> {
  const configuration = vscode.workspace.getConfiguration('asgrep');
  const serverPath = configuration.get<string>('serverPath', 'asgrep-lsp').trim() || 'asgrep-lsp';
  const indexPath = configuration.get<string>('indexPath', '').trim();
  const folders = vscode.workspace.workspaceFolders ?? [];
  const live = new Set(folders.map(folderKey));
  for (const key of [...clients.keys()]) {
    if (!live.has(key)) await stopClient(key);
  }
  for (const folder of folders) {
    await startClientForFolder(folder, serverPath, indexPath);
  }
}

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  context.subscriptions.push(
    vscode.commands.registerCommand('asgrep.search', searchWorkspace),
    vscode.workspace.onDidChangeWorkspaceFolders(() => {
      void reconcileClients();
    }),
    new vscode.Disposable(() => {
      void Promise.all([...clients.keys()].map((key) => stopClient(key)));
    }),
  );
  await reconcileClients();
}

export async function deactivate(): Promise<void> {
  await Promise.all([...clients.keys()].map((key) => stopClient(key)));
}

async function clientForSearch(): Promise<{ client: LanguageClient; folder: vscode.WorkspaceFolder } | undefined> {
  const folders = vscode.workspace.workspaceFolders;
  if (!folders?.length) {
    void vscode.window.showErrorMessage('ast-sgrep: open a workspace folder before searching.');
    return undefined;
  }
  const activeUri = vscode.window.activeTextEditor?.document.uri;
  const folder = folderForUri(activeUri, folders);
  if (!folder) {
    void vscode.window.showErrorMessage(
      'ast-sgrep: multi-root workspace requires an active editor in the folder you want to search (refuses folders[0] fallback).',
    );
    return undefined;
  }
  const client = clients.get(folderKey(folder));
  if (!client) {
    void vscode.window.showErrorMessage(`ast-sgrep language server is not running for “${folder.name}”.`);
    return undefined;
  }
  return { client, folder };
}

async function searchWorkspace(): Promise<void> {
  const bound = await clientForSearch();
  if (!bound) return;

  const { client, folder } = bound;
  const query = await vscode.window.showInputBox({
    prompt: `Search “${folder.name}” with ast-sgrep`,
    placeHolder: 'Symbol, text, callers:name, defs:name, or pattern:...',
  });
  if (!query?.trim()) return;

  const trimmed = query.trim();
  try {
    const semantic = vscode.workspace.getConfiguration('asgrep').get<boolean>('semantic', true);
    const response = await client.sendRequest<SearchResponse>('asgrep/search', {
      query: trimmed,
      semantic,
      limit: 100,
    });
    const hits = Array.isArray(response.hits) ? response.hits : [];
    if (hits.length === 0) {
      void vscode.window.showInformationMessage(`ast-sgrep: no results for “${trimmed}” in ${folder.name}.`);
      return;
    }
    const selected = await vscode.window.showQuickPick(
      hits.map((hit) => toQuickPickItem(hit, folder)),
      {
        matchOnDescription: true,
        matchOnDetail: true,
        placeHolder: `${hits.length} ast-sgrep result${hits.length === 1 ? '' : 's'} (${folder.name})`,
      },
    );
    if (selected) await openHit(selected.hit, selected.folder);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    void vscode.window.showErrorMessage(`ast-sgrep search failed: ${message}`);
  }
}

function toQuickPickItem(hit: SearchHit, folder: vscode.WorkspaceFolder): SearchQuickPickItem {
  const file = hitFilePath(hit) || '(unknown file)';
  const line = hitLineNumber(hit);
  const excerpt = hit.excerpt?.trim() || '(no excerpt)';
  return {
    label: excerpt.split(/\r?\n/, 1)[0],
    description: `${folder.name}:${file}:${line}`,
    detail: excerpt,
    hit,
    folder,
  };
}

async function openHit(hit: SearchHit, folder: vscode.WorkspaceFolder): Promise<void> {
  const file = hitFilePath(hit);
  if (!file) {
    void vscode.window.showWarningMessage('ast-sgrep result did not include a file path.');
    return;
  }
  const fsPath = resolveHitPath(file, { name: folder.name, fsPath: folder.uri.fsPath });
  const document = await vscode.workspace.openTextDocument(vscode.Uri.file(fsPath));
  const line = Math.max(0, hitLineNumber(hit) - 1);
  const column = Math.max(0, hit.start_column ?? hit.column ?? 0);
  const position = new vscode.Position(line, column);
  const editor = await vscode.window.showTextDocument(document);
  editor.selection = new vscode.Selection(position, position);
  editor.revealRange(new vscode.Range(position, position), vscode.TextEditorRevealType.InCenterIfOutsideViewport);
}
