# ReversedFront

ReversedFront 是以 Rust + Tauri 2 封裝的跨平台桌面版，載入新版遊戲前端與編譯後的 Mod。應用程式及主視窗名稱統一為 `ReversedFront`。

## 開發

```powershell
npm install
npm run mod:build
npm test
npm run dev
```

Node 依賴只由根目錄的 `package.json` 管理。`npm run mod:build` 會把本機 Mod 原始碼編譯為 `web/mod/js/main.bundle.js` 與必要的分割 bundle。

## 多程序與資料

- 每個 ReversedFront 程序使用獨立的隨機 loopback port 與鎖定槽位；槽位各自有持久 WebView 資料目錄，因此可同時登入不同帳號並記住各自選擇。
- 帳號與各帳號的 Mod 設定存於 SQLite `accounts.db`，不會打包進安裝檔。
- Windows 可攜版在執行檔目錄可寫時把帳號與下載素材放在該目錄；標準安裝目錄不可寫時會安全退回應用程式資料目錄。
- macOS 與 Linux 不寫入唯讀的 app bundle／系統安裝目錄，會使用平台的應用程式資料目錄。
- 遊戲關卡資料由執行中的官方前端通訊動態擷取；`RFcity.yaml` 不再是必要資料來源。靜態 `transportRoutes.json` 僅作航線資料的備援。

## 公開版本與更新

目前 Git remote `origin` 指向 `MoLinOwO/ReversedFront_Public`。公開倉庫只追蹤編譯後的 Mod bundle 與資料，不追蹤 `web/mod/js` 下的原始模組。原始模組只存在本機工作目錄並受 `.gitignore` 保護；若需要異地備份，應另建私人倉庫。

`npm run validate:release` 會阻止 Mod 原始碼、source map、帳號資料、資料庫或憑證進入發版。GitHub Actions 在收到 `v*` tag 後，先執行此檢查，再為 Windows、Linux、macOS 打包並建立同一個 GitHub Release。

桌面版與 Mod 都從 `ReversedFront_Public` 檢查更新，前端只顯示一個整合更新通知。Mod 下載只接受公開倉庫中編譯好的 bundle 與白名單資料檔；內建 Mod 基準版本會在編譯時自動對齊 Git commit。

## 發版

```powershell
npm run mod:build
npm test
git add -A
git commit -m "release: prepare vX.Y.Z"
git push origin main
git tag vX.Y.Z
git push origin vX.Y.Z
```

不要手動加入被忽略的 Mod 原始碼，也不要提交 `accounts.db`、`config.json`、憑證或 `web/passionfruit`。
