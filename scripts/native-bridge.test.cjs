const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const crypto = require('node:crypto');
const source = fs.readFileSync(require('node:path').join(__dirname, '../desktop/src/native_bridge.js'), 'utf8');

function page({ origin = 'http://127.0.0.1:4567', iframe = false, failure = false } = {}) {
  const messages = [];
  const timers = new Map();
  let timerId = 0;
  const window = { ipc: { postMessage(text) {
    if (failure) throw new Error('closed');
    messages.push(JSON.parse(text));
  } } };
  window.top = iframe ? {} : window;
  vm.runInNewContext(source.replace('__RF_ORIGIN__', JSON.stringify('http://127.0.0.1:4567'))
    .replace('__RF_CAPABILITY__', JSON.stringify('capability')), {
      window, location: { origin }, crypto,
      setTimeout: fn => { timers.set(++timerId, fn); return timerId; },
      clearTimeout: id => timers.delete(id)
    });
  return { window, messages, timers };
}

test('only the trusted top-level game receives a bridge', () => {
  for (const options of [{ iframe: true }, { origin: 'https://discord.com' }]) {
    assert.equal(page(options).window.__RF_DESKTOP__, undefined);
    assert.equal(page(options).window.__TAURI__, undefined);
  }
});

test('released Mod invokes preserve arguments and return native results', async () => {
  const { window, messages, timers } = page();
  const promise = window.__RF_DESKTOP__.invoke('get_config_volume', { targetAccount: { account: 'demo' } });
  assert.equal(window.__RF_DESKTOP__.framework, 'gpui');
  assert.equal(window.__TAURI__.core.invoke, window.__RF_DESKTOP__.invoke);
  assert.deepEqual(messages[0].args, { targetAccount: { account: 'demo' } });
  const request = messages[0];
  window.__rfNativeReply({ ...request, page: 'old-document', ok: true, value: 'stale' });
  assert.equal(timers.size, 1);
  window.__rfNativeReply({ ...request, ok: true, value: { bgm: 0.2 } });
  assert.deepEqual(await promise, { bgm: 0.2 });
  assert.equal(timers.size, 0);
});

test('native errors, timeouts and posting failures reject and clean up', async () => {
  const { window, messages, timers } = page();
  const promise = window.__RF_DESKTOP__.invoke('unknown');
  window.__rfNativeReply({ ...messages[0], ok: false, error: 'Unsupported command' });
  await assert.rejects(promise, /Unsupported command/);
  const timeout = window.__RF_DESKTOP__.invoke('get_accounts');
  [...timers.values()][0]();
  await assert.rejects(timeout, /逾時/);
  const failed = page({ failure: true });
  await assert.rejects(failed.window.__RF_DESKTOP__.invoke('get_accounts'), /closed/);
  assert.equal(failed.timers.size, 0);
});
