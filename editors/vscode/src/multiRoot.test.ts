import * as assert from 'assert';
import * as path from 'path';
import { folderForUriPath, resolveHitPath } from './multiRoot';

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

test('folderForUriPath fails closed when multi-root and no document', () => {
  assert.strictEqual(folderForUriPath(undefined, folders), undefined);
});

test('folderForUriPath allows single-root without document', () => {
  const folder = folderForUriPath(undefined, [folders[0]]);
  assert.strictEqual(folder?.name, 'alpha');
});

test('resolveHitPath prefers the search folder then other roots', () => {
  const exists = (p: string) => p === path.join('/workspaces/beta', 'lib.rs');
  const resolved = resolveHitPath('lib.rs', folders[0], folders, exists);
  assert.strictEqual(resolved, path.join('/workspaces/beta', 'lib.rs'));
});

test('resolveHitPath does not silently use folders[0] when preferred misses', () => {
  const exists = () => false;
  const resolved = resolveHitPath('missing.rs', folders[1], folders, exists);
  assert.strictEqual(resolved, path.join('/workspaces/beta', 'missing.rs'));
});

console.log('multi-root helpers: all tests passed');
