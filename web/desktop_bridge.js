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

  // 玩家大廳由桌面端交給作業系統的預設瀏覽器開啟，避免 Tauri WebView
  // 將 window.open 當成應用程式內的新視窗而被攔截或留在空白頁。
  function openExternalUrl(url) {
    if (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) {
      window.__TAURI__.core.invoke('open_external_url', { url: url }).catch(function () {
        openInGameOverlay(url, 'Discord 社群', 'Discord 社群', true);
      });
      return;
    }
    window.open(url, '_blank', 'noopener,noreferrer');
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

  // The official bundle asks the native mobile billing SDK for these records.
  // A desktop WebView has no Play/App Store SDK, so return the same fallback
  // catalogue embedded by the official web build. Unknown future product IDs
  // are still returned with neutral fields so their UI does not disappear.
  var desktopIapCatalog = {
    iapb_341: { localizedPrice: '$100.00', price: '100' },
    iapbc_4: { localizedPrice: '$290.00', price: '290' },
    iapb_7: { localizedPrice: '$250.00', price: '250' },
    iapbc_7: { localizedPrice: '$300.00', price: '300' },
    iapb_67: { localizedPrice: '$190.00', price: '190' },
    iapb_6: { localizedPrice: '$200.00', price: '200' },
    iapb_8: { localizedPrice: '$250.00', price: '250' },
    iapb_347: { localizedPrice: '$450.00', price: '450' }
  };

  function desktopIapProducts(productIds) {
    var seen = Object.create(null);
    return (Array.isArray(productIds) ? productIds : []).filter(function (productId) {
      if (!productId || seen[productId]) return false;
      seen[productId] = true;
      return true;
    }).map(function (productId) {
      var known = desktopIapCatalog[productId] || {};
      return {
        countryCode: 'TWN',
        currency: 'TWD',
        description: '',
        discounts: [],
        introductoryPrice: '',
        introductoryPriceAsAmountIOS: '',
        introductoryPriceNumberOfPeriodsIOS: '',
        introductoryPricePaymentModeIOS: '',
        introductoryPriceSubscriptionPeriodIOS: '',
        localizedPrice: known.localizedPrice || '',
        price: known.price || '',
        productId: productId,
        subscriptionPeriodNumberIOS: '0',
        subscriptionPeriodUnitIOS: '',
        title: '',
        type: 'iap'
      };
    });
  }

  function openInGameOverlay(url, titleText, ariaLabel, floating) {
    var existing = document.getElementById('rf-in-app-overlay');
    if (existing) {
      existing.style.display = 'flex';
      var existingFrame = existing.querySelector('iframe');
      if (existingFrame) {
        existingFrame.src = url;
        existingFrame.title = titleText;
        existingFrame.focus();
      }
      return;
    }

    var overlay = document.createElement('section');
    overlay.id = 'rf-in-app-overlay';
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-label', ariaLabel);
    Object.assign(overlay.style, {
      position: 'fixed',
      zIndex: '2147483646',
      display: 'flex',
      flexDirection: 'column',
      background: '#080808',
      ...(floating ? {
        left: '50%',
        top: '50%',
        width: 'min(900px, calc(100vw - 32px))',
        height: 'min(700px, calc(100vh - 32px))',
        transform: 'translate(-50%, -50%)',
        border: '1px solid #806d42',
        borderRadius: '12px',
        boxShadow: '0 12px 48px #000c',
        overflow: 'hidden'
      } : {
        inset: '0'
      })
    });

    var toolbar = document.createElement('header');
    Object.assign(toolbar.style, {
      boxSizing: 'border-box',
      height: '48px',
      flex: '0 0 48px',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'space-between',
      padding: '0 10px 0 18px',
      color: '#f4ead2',
      background: '#17130d',
      borderBottom: '1px solid #806d42',
      font: '600 18px "Noto Serif TC", serif',
      cursor: floating ? 'move' : 'default',
      userSelect: 'none'
    });

    var title = document.createElement('span');
    title.textContent = titleText;

    var close = document.createElement('button');
    close.type = 'button';
    close.textContent = '×';
    close.setAttribute('aria-label', '關閉' + titleText);
    Object.assign(close.style, {
      width: '38px',
      height: '38px',
      padding: '0',
      border: '1px solid #806d42',
      borderRadius: '6px',
      color: '#f4ead2',
      background: '#2a2115',
      cursor: 'pointer',
      font: '30px/32px sans-serif'
    });

    var frame = document.createElement('iframe');
    frame.src = url;
    frame.title = titleText;
    frame.referrerPolicy = 'strict-origin-when-cross-origin';
    frame.allow = 'clipboard-read; clipboard-write; payment';
    Object.assign(frame.style, {
      width: '100%',
      minHeight: '0',
      flex: '1 1 auto',
      border: '0',
      background: '#fff'
    });

    var previousOverflow = document.body.style.overflow;
    var closeOverlay = function () {
      cleanupDrag();
      document.body.style.overflow = previousOverflow;
      overlay.remove();
    };
    var cleanupDrag = function () {};

    if (floating) {
      var isDragging = false;
      var offsetX = 0;
      var offsetY = 0;
      var onMouseMove = function (event) {
        if (!isDragging) return;
        var maxLeft = Math.max(0, window.innerWidth - overlay.offsetWidth);
        var maxTop = Math.max(0, window.innerHeight - overlay.offsetHeight);
        var left = Math.max(0, Math.min(maxLeft, event.clientX - offsetX));
        var top = Math.max(0, Math.min(maxTop, event.clientY - offsetY));
        overlay.style.left = left + 'px';
        overlay.style.top = top + 'px';
      };
      var onMouseUp = function () {
        isDragging = false;
        document.body.style.userSelect = '';
      };
      toolbar.addEventListener('mousedown', function (event) {
        if (event.button !== 0 || event.target.closest('button')) return;
        var rect = overlay.getBoundingClientRect();
        isDragging = true;
        offsetX = event.clientX - rect.left;
        offsetY = event.clientY - rect.top;
        overlay.style.transform = 'none';
        overlay.style.left = rect.left + 'px';
        overlay.style.top = rect.top + 'px';
        document.body.style.userSelect = 'none';
        event.preventDefault();
      });
      window.addEventListener('mousemove', onMouseMove);
      window.addEventListener('mouseup', onMouseUp);
      cleanupDrag = function () {
        window.removeEventListener('mousemove', onMouseMove);
        window.removeEventListener('mouseup', onMouseUp);
        document.body.style.userSelect = '';
      };
    }
    close.addEventListener('click', closeOverlay);
    overlay.addEventListener('keydown', function (event) {
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopImmediatePropagation();
        closeOverlay();
      }
    }, true);

    toolbar.appendChild(title);
    toolbar.appendChild(close);
    overlay.appendChild(toolbar);
    overlay.appendChild(frame);
    document.body.style.overflow = 'hidden';
    document.body.appendChild(overlay);
    close.focus();
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
            // 官方前端的 lobby_url 可能由舊伺服器資料提供，桌面版統一
            // 導向目前有效的 Discord 社群邀請，不依賴該欄位的內容。
            openExternalUrl('https://discord.com/invite/wyFj4N2mJZ');
            break;
          case 'APP:fixWebviewHeight':
          case 'DOWNLOAD:dlInfo_show':
          case 'WSS:playerChannelJoined':
            break;
          case 'IAP:getProducts':
            emitAppMessage('IAP:products', {
              products: desktopIapProducts(message.data && message.data.productIds)
            });
            break;
          case 'IAP:buyProduct':
            openInGameOverlay(
              'https://reversedfront.tw/market/',
              'ReversedFront · 遊戲商城',
              'ReversedFront 遊戲商城',
              false
            );
            // The game shows its processing modal until the native billing
            // bridge reports phase-one completion. Desktop hands payment off
            // to the embedded web market, so finish that native phase without
            // presenting an extra error dialog.
            window.setTimeout(function () {
              emitAppMessage('IAP:purchaseError', { errorData: {} });
            }, 0);
            break;
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
