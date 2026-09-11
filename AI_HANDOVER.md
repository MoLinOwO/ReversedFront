# ReversedFront 維護交接

## 現況

- 專案：Rust + Tauri 2 桌面殼、官方靜態前端、Webpack Mod。
- 唯一 Git remote：`origin` → `https://github.com/MoLinOwO/ReversedFront_Public.git`。
- 公開倉庫只允許 `web/mod/js/*.bundle.js` 與對應 LICENSE 檔；Mod 原始碼目錄及 `index.js` 僅留在本機並由 `.gitignore` 排除。
- GitHub Actions 由 `v*` tag 觸發 Windows、Linux、macOS 打包。
- 功能完成前不要執行完整 Tauri 打包；可以執行 `npm run mod:build`、`npm test`、`cargo check` 與 `npm run dev`。

## 架構邊界

- `src-tauri/src/lib.rs`：選擇可寫資料路徑、啟動每程序獨立的本機 HTTP server、建立主 WebView。
- `resource_manager.rs`：按需下載並快取 `passionfruit` 素材；同程序去重、跨程序使用唯一暫存檔。
- `account_manager.rs`：SQLite 帳號與每帳號設定，使用 WAL、busy timeout、transaction 支援多程序。
- `mod_updater.rs`：從 GitHub main 只下載編譯 bundle 與白名單資料，以跨程序檔案鎖套用更新。
- `commands.rs`：Tauri API 與資料檔名白名單。
- `web/mod/js/core/gameDataBridge.js`：擷取官方 Phoenix/WebSocket 資料，提供城市、原名、獎勵與關卡資訊。
- `web/mod/js/map/`：地圖標記、勢力圖層、城市資訊、飛機／港口航線與士兵圖示。

## 必須維持的規則

1. 主視窗與應用程式名稱保持 `ReversedFront`；退出對話框中的遊戲名為「逆統戰：地下世界」。
2. 不得恢復固定 port；多個程序必須取得不同的跨程序槽位，且 WebView cookie/localStorage 相互隔離。
3. `accounts.db` 不得進入專案或安裝資源。Windows 可攜目錄可寫時可放執行檔旁；macOS/Linux 與唯讀安裝位置必須退回平台資料目錄。
4. `RFcity.yaml` 不得成為必要依賴。城市獎勵優先由遊戲即時資料取得，主線劇情掉落需排除；航線可以使用 `transportRoutes.json` 備援。
5. 不得把 Mod 原始 JS 或 source map 推送至公開倉庫。每次提交／tag 前執行 `npm run validate:release`。
6. `src-tauri/build.rs` 僅複製官方前端資源、編譯 bundle 與白名單 Mod data，不能直接複製整個開發目錄。

## 驗證順序

```powershell
npm install
npm run mod:build
npm test
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run dev
```

桌面煙霧測試至少確認：自動登入能到 `#/home`、地圖能到 `#/portalmap`、城市名稱可見、兩個 `rf-desktop` 程序使用不同 loopback port 與不同 WebView 資料目錄。測試後關閉所有遊戲程序；正式打包交由 GitHub Actions。
