'use strict';
// Exercise real extension TS with deterministic host/client boundaries, not VS Code itself.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { stripTypeScriptTypes } = require('node:module');
const { test } = require('node:test');

const root = path.resolve(__dirname, '../..');
const folder = name => ({ name, uri: { fsPath: `/workspace/${name}`, toString: () => `file:///workspace/${name}` } });
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const drain = () => new Promise(resolve => setImmediate(resolve));

async function load(folders, start = async () => {}, stop = async () => {}) {
  const state = { folders, clients: [], errors: [], requests: [], changed: null, search: null };
  const context = { subscriptions: [] };
  const host = {
    workspace: {
      get workspaceFolders() { return state.folders; },
      getConfiguration() { return { get: (_key, fallback) => fallback }; },
      getWorkspaceFolder() { return undefined; },
      onDidChangeWorkspaceFolders(fn) { state.changed = fn; return { dispose() {} }; },
    },
    commands: { registerCommand(_name, fn) { state.search = fn; return { dispose() {} }; } },
    Disposable: class { constructor(fn) { this.dispose = fn; } },
    window: {
      showErrorMessage(message) { state.errors.push(message); },
      async showInputBox() { return 'target'; },
      showInformationMessage() {},
    },
  };
  class LanguageClient {
    constructor(id) { this.id = id; this.running = false; state.clients.push(this); }
    async start() {
      this.running = true; // Startup may allocate resources before it rejects.
      await start(this);
      this.running = true; // Stopping before this settles does not cancel startup.
    }
    async stop() { await stop(this); this.running = false; }
    async dispose() { this.disposed = true; await this.stop(); }
    async sendRequest(method, params) { state.requests.push({ client: this, method, params }); return { hits: [] }; }
  }
  const synthetic = (id, values) => new vm.SyntheticModule(Object.keys(values), function () {
    for (const [key, value] of Object.entries(values)) this.setExport(key, value);
  }, { identifier: id });
  const readTs = rel => stripTypeScriptTypes(fs.readFileSync(path.join(root, rel), 'utf8'));
  const helper = new vm.SourceTextModule(readTs('editors/vscode/src/multiRoot.ts'));
  await helper.link(() => synthetic('path', path));
  await helper.evaluate();
  const extension = new vm.SourceTextModule(readTs('editors/vscode/src/extension.ts'));
  await extension.link(id => {
    if (id === 'vscode') return synthetic(id, host);
    if (id === 'vscode-languageclient/node') return synthetic(id, {
      LanguageClient, Executable: undefined, LanguageClientOptions: undefined, ServerOptions: undefined,
    });
    if (id === './multiRoot') return helper;
    throw new Error(`unexpected import ${id}`);
  });
  await extension.evaluate();
  return { state, context, api: extension.namespace };
}

for (const removed of ['a', 'b']) {
  test(`removal of ${removed} during deferred startup cannot leave an obsolete client`, { timeout: 1000 }, async () => {
    const started = deferred(), ready = deferred();
    const a = folder('a'), b = folder('b');
    const { state, context, api } = await load([a, b], client => {
      if (client.id === 'asgrep-a') { started.resolve(); return ready.promise; }
    });
    const active = api.activate(context);
    await started.promise;
    state.folders = removed === 'a' ? [b] : [a];
    // For A, delay event delivery to expose cleanup at startup completion itself.
    // For B, overlap repeated events with the old reconcile's folder snapshot.
    if (removed === 'b') {
      state.changed();
      state.changed();
    }
    await drain();
    ready.resolve();
    try {
      await active;
      await drain();
      assert.deepEqual(state.clients.filter(client => client.running).map(client => client.id),
        removed === 'a' ? ['asgrep-b'] : ['asgrep-a']);
      assert.deepEqual(state.clients.map(client => client.id), removed === 'a' ? ['asgrep-a', 'asgrep-b'] : ['asgrep-a']);
      await state.search();
      assert.equal(state.requests.length, 1);
      assert.equal(state.requests[0].client.id, removed === 'a' ? 'asgrep-b' : 'asgrep-a');
    } finally { await api.deactivate(); }
  });
}

for (const shutdown of ['deactivate', 'dispose']) {
  test(`${shutdown} drains startup and prevents late clients`, { timeout: 1000 }, async () => {
    const started = deferred(), ready = deferred(), stopped = deferred(), finishStop = deferred();
    const { state, context, api } = await load([folder('a'), folder('b')], client => {
      if (client.id === 'asgrep-a') { started.resolve(); return ready.promise; }
    }, () => { stopped.resolve(); return finishStop.promise; });
    const active = api.activate(context);
    await started.promise;
    state.changed(); // Queue another reconcile before shutdown.
    if (shutdown === 'dispose') context.subscriptions.forEach(item => item.dispose());
    let done = false;
    const inactive = shutdown === 'deactivate'
      ? api.deactivate().then(() => { done = true; })
      : Promise.resolve();
    state.changed(); // A late event must not reopen the lifecycle.
    await drain();
    const returnedBeforeStart = done;
    ready.resolve(); // Do not await shutdown before releasing the startup it drains.
    await stopped.promise;
    await drain();
    const returnedBeforeStop = done;
    finishStop.resolve();
    await Promise.all([active, inactive]);
    await drain();
    if (shutdown === 'deactivate') {
      assert.equal(returnedBeforeStart, false, 'shutdown must drain startup');
      assert.equal(returnedBeforeStop, false, 'shutdown must drain cleanup');
    }
    assert.deepEqual(state.clients.map(client => client.id), ['asgrep-a'], 'no late B creation');
    assert.deepEqual(state.clients.filter(client => client.running), [], 'no client survives shutdown');
    state.changed();
    await drain();
    assert.deepEqual(state.clients.map(client => client.id), ['asgrep-a']);
  });
}

test('failed startup is cleaned up, rejects activation, and permits an event retry', { timeout: 1000 }, async () => {
  const started = deferred(), ready = deferred(), retryStarted = deferred(), retryReady = deferred();
  let attempts = 0;
  const { state, context, api } = await load([folder('a')], () => {
    if (++attempts === 1) { started.resolve(); return ready.promise; }
    retryStarted.resolve();
    return retryReady.promise;
  });
  const active = assert.rejects(api.activate(context), /controlled startup failure/);
  await started.promise;
  ready.reject(new Error('controlled startup failure'));
  await active;
  try {
    assert.equal(state.clients[0].running, false, 'failed startup resources must be stopped');
    assert.equal(state.clients[0].disposed, true, 'failed client must be disposed');
    state.changed();
    await retryStarted.promise;
    state.changed();
    await state.search();
    assert.equal(state.requests.length, 0, 'an incomplete startup is not a searchable client');
  } finally { retryReady.resolve(); }
  await drain();
  try {
    assert.equal(attempts, 2);
    await state.search();
    assert.equal(state.requests.length, 1);
    assert.equal(state.requests[0].client, state.clients[1]);
  } finally { await api.deactivate(); }
});

test('workspace-event startup failure is reported and a later event can retry', { timeout: 1000 }, async () => {
  const started = deferred(), ready = deferred();
  let attempts = 0;
  const { state, context, api } = await load([], () => {
    if (++attempts === 1) { started.resolve(); return ready.promise; }
  });
  await api.activate(context);
  state.folders = [folder('a')];
  state.changed();
  await started.promise;
  ready.reject(new Error('controlled event failure'));
  await drain();
  try {
    assert.ok(state.errors.some(message => message.includes('controlled event failure')));
    assert.equal(state.clients[0].running, false);
    state.changed();
    await drain();
    await state.search();
    assert.equal(state.requests.length, 1);
    assert.equal(state.requests[0].client, state.clients[1]);
  } finally { await api.deactivate(); }
});
