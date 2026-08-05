import * as assert from 'assert';
import * as path from 'path';
import { folderForUriPath, hitFilePath, hitLineNumber, resolveHitPath } from './multiRoot';

function test(name: string, fn: () => void): void {
  try {
    fn();
    console.log(`ok - ${name}`);
  } catch (err) {
    console.error(`not ok - ${name}`);
    throw err;
  }
}

const folders = [
  { name: 'alpha', fsPath: '/workspaces/alpha' },
  { name: 'beta', fsPath: '/workspaces/beta' },
];

test('folderForUriPath binds active document to its root', () => {
  const folder = folderForUriPath('/workspaces/beta/src/main.rs', folders);
  assert.strictEqual(folder?.name, 'beta');
});

test('folderForUriPath chooses the most specific nested root', () => {
  const nested = [
    { name: 'parent', fsPath: '/workspaces/project' },
    { name: 'child', fsPath: '/workspaces/project/packages/child' },
  ];
  const folder = folderForUriPath('/workspaces/project/packages/child/src/main.ts', nested);
  assert.strictEqual(folder?.name, 'child');
});

test('folderForUriPath fails closed when multi-root and no document', () => {
  assert.strictEqual(folderForUriPath(undefined, folders), undefined);
});

test('folderForUriPath allows single-root without document', () => {
  const folder = folderForUriPath(undefined, [folders[0]]);
  assert.strictEqual(folder?.name, 'alpha');
});

test('resolveHitPath never crosses into another workspace root', () => {
  const resolved = resolveHitPath('lib.rs', folders[0]);
  assert.strictEqual(resolved, path.join('/workspaces/alpha', 'lib.rs'));
});

test('resolveHitPath does not silently use folders[0] when preferred misses', () => {
  const resolved = resolveHitPath('missing.rs', folders[1]);
  assert.strictEqual(resolved, path.join('/workspaces/beta', 'missing.rs'));
});

test('resolveHitPath rejects traversal and outside absolute paths', () => {
  assert.throws(() => resolveHitPath('../secret.txt', folders[0]), /outside workspace root/);
  const outside = path.resolve(folders[0].fsPath, '..', 'secret.txt');
  assert.throws(() => resolveHitPath(outside, folders[0]), /outside workspace root/);
});

test('hitFilePath / hitLineNumber prefer canonical fields', () => {
  assert.strictEqual(hitFilePath({ path: 'a.rs', file: 'b.rs' }), 'a.rs');
  assert.strictEqual(hitLineNumber({ line_start: 9, line: 1 }), 9);
});

console.log('multi-root helpers: all tests passed');
