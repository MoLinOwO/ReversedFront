// Injected by the native host before the official game and compiled Mod run.
(() => {
  'use strict';
  if (window !== window.top || location.origin !== __RF_ORIGIN__) return;
  const capability = __RF_CAPABILITY__;
  const page = crypto.randomUUID();
  const pending = new Map();
  let sequence = 0;
  function invoke(command, args = {}) {
    if (pending.size >= 256) return Promise.reject(new Error('桌面請求過多，請稍後重試'));
    return new Promise((resolve, reject) => {
      const id = ++sequence;
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error('桌面請求逾時：' + command));
      }, /update|download/.test(command) ? 15 * 60 * 1000 : 30000);
      pending.set(id, { resolve, reject, timer });
      try {
        window.ipc.postMessage(JSON.stringify({ capability, page, id, command, args }));
      } catch (error) {
        clearTimeout(timer);
        pending.delete(id);
        reject(error);
      }
    });
  }
  Object.defineProperty(window, '__rfNativeReply', { value(reply) {
    if (reply.page !== page) return;
    const request = pending.get(reply.id);
    if (!request) return;
    pending.delete(reply.id);
    clearTimeout(request.timer);
    if (reply.ok) request.resolve(reply.value);
    else request.reject(new Error(reply.error));
  } });
  window.__RF_DESKTOP__ = Object.freeze({ framework: 'gpui', invoke });
  // Public compatibility contract for released Mod bundles; no Tauri runtime.
  window.__TAURI__ = Object.freeze({ core: Object.freeze({ invoke }) });
})();
