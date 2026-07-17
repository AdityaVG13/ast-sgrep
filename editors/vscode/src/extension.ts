import * as path from 'path';
import * as vscode from 'vscode';
import { Executable, LanguageClient, LanguageClientOptions, ServerOptions } from 'vscode-languageclient/node';

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
interface SearchQuickPickItem extends vscode.QuickPickItem { hit: SearchHit; }
let client: LanguageClient | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const configuration = vscode.workspace.getConfiguration('asgrep');
  const serverPath = configuration.get<string>('serverPath', 'asgrep-lsp').trim() || 'asgrep-lsp';
  const indexPath = configuration.get<string>('indexPath', '').trim();
  const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
  const executable: Executable = {
    command: serverPath,
    args: ['--stdio'],
    options: workspaceFolder ? { cwd: workspaceFolder.uri.fsPath } : undefined,
  };
  const serverOptions: ServerOptions = executable;
  const clientOptions: LanguageClientOptions = {
    documentSelector: [
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
    ],
    initializationOptions: { asgrep: indexPath ? { indexPath } : {} },
  };
  client = new LanguageClient('asgrep', 'ast-sgrep Language Server', serverOptions, clientOptions);
  context.subscriptions.push(vscode.commands.registerCommand('asgrep.search', searchWorkspace), client);
  await client.start();
}

export async function deactivate(): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
}

async function searchWorkspace(): Promise<void> {
  if (!client) {
    void vscode.window.showErrorMessage('ast-sgrep language server is not running.');
    return;
  }
  const query = await vscode.window.showInputBox({
    prompt: 'Search the workspace with ast-sgrep',
    placeHolder: 'Symbol, text, callers:name, defs:name, or pattern:...',
  });
  if (!query?.trim()) return;
  try {
    const semantic = vscode.workspace.getConfiguration('asgrep').get<boolean>('semantic', true);
    const response = await client.sendRequest<SearchResponse>('asgrep/search', {
      query: query.trim(), semantic, limit: 100,
    });
    const hits = Array.isArray(response.hits) ? response.hits : [];
    if (hits.length === 0) {
      void vscode.window.showInformationMessage(`ast-sgrep: no results for “${query.trim()}”.`);
      return;
    }
    const selected = await vscode.window.showQuickPick(hits.map(toQuickPickItem), {
      matchOnDescription: true,
      matchOnDetail: true,
      placeHolder: `${hits.length} ast-sgrep result${hits.length === 1 ? '' : 's'}`,
    });
    if (selected) await openHit(selected.hit);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    void vscode.window.showErrorMessage(`ast-sgrep search failed: ${message}`);
  }
}

function toQuickPickItem(hit: SearchHit): SearchQuickPickItem {
  const file = hitPath(hit) || '(unknown file)';
  const line = hitLine(hit);
  const excerpt = hit.excerpt?.trim() || '(no excerpt)';
  return { label: excerpt.split(/\r?\n/, 1)[0], description: `${file}:${line}`, detail: excerpt, hit };
}

async function openHit(hit: SearchHit): Promise<void> {
  const file = hitPath(hit);
  if (!file) {
    void vscode.window.showWarningMessage('ast-sgrep result did not include a file path.');
    return;
  }
  const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
  const uri = path.isAbsolute(file)
    ? vscode.Uri.file(file)
    : workspaceFolder
      ? vscode.Uri.joinPath(workspaceFolder.uri, file)
      : vscode.Uri.file(path.resolve(file));
  const document = await vscode.workspace.openTextDocument(uri);
  const line = Math.max(0, hitLine(hit) - 1);
  const column = Math.max(0, hit.start_column ?? hit.column ?? 0);
  const position = new vscode.Position(line, column);
  const editor = await vscode.window.showTextDocument(document);
  editor.selection = new vscode.Selection(position, position);
  editor.revealRange(new vscode.Range(position, position), vscode.TextEditorRevealType.InCenterIfOutsideViewport);
}
function hitPath(hit: SearchHit): string | undefined { return hit.path ?? hit.file_path ?? hit.file; }
function hitLine(hit: SearchHit): number { return hit.line_start ?? hit.start_line ?? hit.line ?? 1; }
