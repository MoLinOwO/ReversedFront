# 桌面架構與維護

## GPUI 遷移狀態（2026-11-07）

開發版已由 Tauri runtime 改為 GPUI Kit 0.9.0 + gpui-webview 0.9.0 + lb-wry 0.53.3。GPUI 建立並管理原生視窗；Wry 在視窗內載入官方遊戲與已編譯 Mod。GPUI 不負責執行遊戲 JavaScript。

此遷移納入 3.3.0 發版。Rust 專案位於 `desktop/`，安裝包使用 `@crabnebula/packager` 0.11.2 與根目錄 `packager.json`；原有 Tauri 執行期、CLI 與設定檔均已移除。安裝識別和更新格式維持相容。

## 資料流與相容契約

```text
GPUI 視窗 → 原生 WebView → 本機 HTTP 遊戲頁面／已編譯 Mod
                                   ↓ invoke(command, args)
                           native_bridge.js → ipc.rs
                                   ↓ 驗證來源及參數
                          commands.rs → DesktopHost
                                   ↓
                   SQLite／素材下載／GitHub 更新／視窗操作
```

- `gpui_host.rs` 持有 WebView、獨立 profile、Tokio runtime 及程序槽位鎖；GUI 操作回到 GPUI 執行緒。
- `host.rs` 提供後端所需路徑、資源管理與退出／全螢幕通道。
- `commands.rs` 保留既有 24 個桌面命令、camelCase 參數及回傳格式。
- `native_bridge.js` 在可信任的本機頂層頁面注入；`__RF_DESKTOP__` 為新入口，`__TAURI__.core.invoke` 保留給既有 Mod。商城及 Discord iframe 不取得原生橋接。
- IPC 驗證本機 origin、每次程序啟動的 capability、頁面識別、參數與大小；過期頁面回覆會被忽略。HTTP 靜態服務不提供帳號庫、設定 JSON 或 Mod 原始碼。
- 城鎮資訊、勢力圖、飛機／港口航線、商城和 Discord 懸浮窗沿用既有網頁模組。Windows 原生 WebView 會覆蓋同範圍的 GPUI 控制項，因此遊戲浮層繼續由 DOM 顯示。
- 帳號庫與素材沿用既有路徑：開發版位於 `web/`；Windows 發行版優先使用可寫入的執行檔目錄，其他情況使用既有應用程式資料位置。`accounts.db` 不進入安裝包。
- 每個程序持有獨立槽位和 WebView profile；SQLite WAL 與帳號設定契約維持原狀。更新仍需先關閉其他程序，更新鎖保留至父程序真正退出，下載檔仍進系統下載資料夾。

## 開發與驗證

安裝 Node.js、Rustup 及平台原生建置工具後，於專案根目錄執行：

```sh
npm ci
npm run dev
npm test
npm run build
npm run bundle
```

Rust 由 `rust-toolchain.toml` 固定為 1.99.1。Mod 原始碼只留本機；公開倉庫及 GitHub Actions 使用既有編譯 bundle，發版檢查會拒絕私人資料及 Mod source map。

開發執行檔位於 `desktop/target/debug/rf-desktop.exe`（Windows），正式安裝包輸出至 `desktop/target/packages/release/`。Windows 產生 NSIS `.exe` 與 WiX `.msi`；macOS 產生 `.app`／`.dmg`；Linux 產生 `.AppImage`／`.deb`。Windows 安裝流程會在缺少 WebView2 時驗證並執行 Microsoft 的引導安裝器。

正式發布前使用 `node scripts/set-version.cjs vX.Y.Z` 同步 npm、packager、Cargo 及 lockfile 版號，新增 `docs/releases/vX.Y.Z.md` 更新說明，再提交已編譯 Mod。GitHub 工作流程要求 tag 和已提交版號一致；各平台產物先存為 Release 草稿，待更新清單與 Mod 上傳完成才正式公開，避免使用者收到不完整版本。

Windows 可選擇在啟動前設定 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9227`，再於另一個終端執行：

```sh
node scripts/desktop-smoke.cjs
node scripts/desktop-smoke.cjs --require-map
```

這些檢查連到桌面端內的 WebView，不開外部瀏覽器，不輸出帳密。`--require-map` 需要先停留於城鎮地圖；停留主大廳時失敗屬預期。平常啟動不設定除錯埠。

本機已驗證 Windows 原生視窗、自動登入、主大廳、城鎮地圖、Mod 載入、現有語言／音量設定讀取、退出提示資料與橋接拒絕非法命令。多程序鎖、更新鎖生命週期、命令參數與橋接例外有回歸測試。兩個不同真實帳號同時登入、實際更新安裝、商城付款、macOS／Linux 實機仍待驗證。

獨立打包器已成功產生 Windows 開發測試用 NSIS／MSI，MSI 內容檢查未發現帳號庫、私人設定、憑證或 source map。這些測試包位於 `desktop/target/packages/debug/`，尚未安裝、發布，不能當作正式 Release 散佈。MSI 升級識別依既有工具的預設算法保留；跨版本實際安裝仍須另行驗證。

安裝圖示使用有效的 `icon.png`（512×512）及 `icon.ico`，macOS ICNS 由打包器轉換。舊 `icon.icns` 實際為 ICO 格式，不可直接指定給 macOS 安裝包。

## 平台限制及依據

- Windows 使用 WebView2，按上游範例關閉 GPUI DirectComposition，讓原生子視窗正常顯示。
- macOS 使用 WKWebView，目前打包最低版本設定為 macOS 15。
- Linux 在建立持久化 WebContext 前初始化 GTK，並明確選擇 X11；Wayland 工作階段需有 XWayland，並安裝 WebKitGTK 4.1、GTK 3 與 GPUI 所需圖形函式庫。
- `gpui-webview` 上游標記為實驗性整合，不能將 Windows 驗證結果視為所有平台已通過。

參考：[GPUI 官方介紹](https://gpui.rs/)、[GPUI Kit WebView 0.9.0](https://gpui-kit.com/docs/webview/)、[上游 WebView 範例](https://github.com/longbridge/gpui-kit/tree/main/examples/webview)、[獨立打包器文件](https://docs.crabnebula.dev/packager/)。
