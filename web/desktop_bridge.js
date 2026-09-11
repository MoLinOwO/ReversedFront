// Desktop compatibility layer for the current mobile-first RF bundle.
// The official web bundle expects a small native object to exist when it is
// running inside the Android WebView. Tauri supplies the real desktop APIs;
// this file only supplies the missing mobile-shaped values and callbacks.
(function () {
  'use strict';

  var localeKey = 'rf.desktop.locale';
  var normalizeLocale = function (locale) {
    switch (String(locale || '').replace('-', '_')) {
      case 'zh_Hant':
      case 'zh_TW':
      case 'zh_TW_#Hant':
        return 'zh_TW';
      case 'zh_Hans':
      case 'zh_CN':
      case 'zh_CN_#Hans':
        return 'zh_CN';
      case 'ja':
      case 'ja_JP':
      case 'jp':
        return 'jp';
      case 'en':
      case 'en_US':
      case 'en_GB':
        return 'en';
      default:
        return 'zh_TW';
    }
  };
  var storedLocale = null;
  try {
    storedLocale = window.localStorage.getItem(localeKey);
  } catch (_) {}
  var locale = normalizeLocale(storedLocale);

  if (!window.deviceInfo) {
    window.deviceInfo = {
      deviceType: 'desktop',
      platform: 'windows',
      uniqueId: 'rf-desktop',
      locale: locale,
      // The desktop shell uses the normal local/remote resource path and has
      // no mobile foreground-download prompt.
      promptedExtraDownload: 'background',
      assetsDL_path: ''
    };
  }

  // The current RF bundle uses this object for mobile-only notifications.
  // Supplying an empty download queue prevents a mobile extra-resource modal
  // from blocking the desktop login screen.
  if (!window.dlInfo) {
    window.dlInfo = { q_files: 0, total_size: 0 };
  }

  function emitAppMessage(action, data) {
    // React Native sends desktop callbacks back to the RF bundle with `code`,
    // not `action`.  The current bundle ignores PRELOAD:progress when this
    // field is wrong, leaving the post-login loading screen without a route.
    window.dispatchEvent(new MessageEvent('message', {
      data: JSON.stringify({ code: action, data: data })
    }));
  }

  function invokeTauri(command, args) {
    if (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) {
      return window.__TAURI__.core.invoke(command, args || {});
    }
    return Promise.resolve(null);
  }

  function preloadImages(images) {
    var list = Array.isArray(images) ? images : [];
    if (!list.length) {
      emitAppMessage('PRELOAD:progress', { ratio: 1 });
      return;
    }

    var completed = 0;
    var update = function () {
      completed += 1;
      emitAppMessage('PRELOAD:progress', {
        ratio: Math.min(1, completed / list.length)
      });
    };

    list.forEach(function (path) {
      var image = new Image();
      image.onload = update;
      image.onerror = update;
      var relativePath = String(path || '').replace(/^\/+/, '');
      image.src = './passionfruit/' + relativePath;
    });
  }

  if (!window.ReactNativeWebView) {
    window.ReactNativeWebView = {
      postMessage: function (payload) {
        var message;
        try {
          message = typeof payload === 'string' ? JSON.parse(payload) : payload;
        } catch (_) {
          return;
        }

        if (!message || !message.action) return;

        switch (message.action) {
          case 'SETTING:locale':
            if (message.data && message.data.locale) {
              window.deviceInfo.locale = normalizeLocale(message.data.locale);
              try {
                window.localStorage.setItem(localeKey, window.deviceInfo.locale);
              } catch (_) {}
            }
            break;
          case 'SETTING:promptedExtraDownload':
            if (message.data) {
              window.deviceInfo.promptedExtraDownload = message.data.promptedExtraDownload;
            }
            break;
          case 'PRELOAD:images':
            preloadImages(message.data && message.data.preloadImages);
            break;
          case 'APP:exit':
            invokeTauri('exit_app');
            break;
          case 'APP:openUrl':
            if (message.data && message.data.url) {
              window.open(message.data.url, '_blank', 'noopener,noreferrer');
            }
            break;
          case 'APP:fixWebviewHeight':
          case 'DOWNLOAD:dlInfo_show':
          case 'WSS:playerChannelJoined':
          case 'IAP:getProducts':
          case 'IAP:buyProduct':
          case 'LOGIN:google':
          case 'LOGIN:apple':
          case 'LOGIN:facebook':
            // These are mobile-only callbacks. Desktop login remains the
            // normal RF form login and does not need a native response here.
            break;
          default:
            break;
        }
      }
    };
  }
})();
