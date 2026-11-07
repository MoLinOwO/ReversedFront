// Read-only checks against an explicitly enabled WebView2 debugging endpoint.
// Launch a development build with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=
// --remote-debugging-port=9227; this script never opens an external browser.
const assert = require('node:assert/strict');

async function main() {
  const args = process.argv.slice(2);
  const endpoint = args.find(arg => !arg.startsWith('--')) || 'http://127.0.0.1:9227';
  const targets = await (await fetch(endpoint + '/json/list')).json();
  const target = targets.find(target => target.type === 'page' && /^http:\/\/127\.0\.0\.1:\d+\//.test(target.url));
  assert(target, 'No ReversedFront game WebView found');
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
  let sequence = 0;
  const requests = new Map();
  socket.onmessage = event => {
    const message = JSON.parse(event.data);
    const request = requests.get(message.id);
    if (!request) return;
    requests.delete(message.id);
    clearTimeout(request.timer);
    if (message.error) request.reject(new Error(message.error.message));
    else request.resolve(message.result);
  };
  function send(method, params) {
    return new Promise((resolve, reject) => {
      const id = ++sequence;
      const timer = setTimeout(() => { requests.delete(id); reject(new Error(method + ' timed out')); }, 20000);
      requests.set(id, { resolve, reject, timer });
      socket.send(JSON.stringify({ id, method, params }));
    });
  }
  try {
    const { result, exceptionDetails } = await send('Runtime.evaluate', {
      expression: `(async () => {
        const invoke = window.__RF_DESKTOP__?.invoke;
        if (!invoke) return { bridge: false, title: document.title };
        const accounts = await invoke('get_accounts');
        const locale = await invoke('get_locale');
        const targetAccount = await window.getRFCurrentAccountData?.();
        const volume = await invoke('get_config_volume', { targetAccount: targetAccount || null });
        const prompts = await invoke('load_yaml', { filename: 'exit_prompts.yaml' });
        let unknownRejected = false;
        try { await invoke('invalid_smoke_command'); } catch (_) { unknownRejected = true; }
        return { bridge: true, framework: window.__RF_DESKTOP__.framework,
          title: document.title, route: location.pathname + location.hash,
          gameRendered: !!document.querySelector('#root')?.children.length,
          modLoaded: !!document.getElementById('custom-controls'),
          accountCount: accounts.length, locale, volume,
          exitPromptsLoaded: !!prompts, unknownRejected,
          loginForm: location.hash.includes('/users/log_in') && !!document.querySelector('input[type=password]'),
          mapVisible: !!document.querySelector('[class*=PortalMap_mapBox]') };
      })()`, awaitPromise: true, returnByValue: true
    });
    assert(!exceptionDetails, 'WebView evaluation failed');
    const status = result.value;
    console.log(JSON.stringify(status, null, 2));
    assert(status.bridge && status.framework === 'gpui', 'GPUI bridge missing');
    assert(status.gameRendered && status.modLoaded, 'Game or Mod did not render');
    assert(status.exitPromptsLoaded && status.unknownRejected, 'Native contract check failed');
    if (args.includes('--require-map')) assert(status.mapVisible, 'City map has not loaded');
    const origin = new URL(target.url).origin;
    for (const privatePath of ['/accounts.db', '/mod/data/config.json', '/mod/js/core/api.js']) {
      assert.equal((await fetch(origin + privatePath)).status, 403, 'Private asset exposed: ' + privatePath);
    }
    console.log('Desktop smoke checks passed. Login/map only verified if mapVisible=true.');
  } finally { socket.close(); }
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
