# 給下一個 AI / 維護者的說明

> 此檔案是專門寫給「之後接手的 AI 助手或維護者」看的，請優先閱讀完再動手改東西。

---

## 1. 專案總覽

- 專案名稱：ReversedFront PC
- 技術棧：Rust + Tauri 2 + 前端 Web (Webpack 打包)
- 目標：提供《逆統戰：烽火》的桌面強化工具（多帳號、地圖、通知等）。
- 主要目錄：
  - `web/`：前端靜態資源與 JS（只在私有庫保留完整原始碼）。
  - `src-tauri/`：Tauri 2 / Rust 後端程式與打包設定。
  - `.github/workflows/release.yml`：三平台自動打包 Release 的 CI。

**重要觀念：**
- 此專案有 **私有 repo**（含完整 JS 原始碼）與 **公開 repo**（只放 bundle，不能洩漏 JS 原始碼）。
- 目前這個工作目錄已經設定好兩個 remote：
  - `origin` → 私有 `ReversedFront_PC`
  - `public` → 公開 `ReversedFront_Public`

---

## 2. 打包與資源路徑設計

關鍵檔案：
- `src-tauri/tauri.conf.json`
- `src-tauri/build.rs`
- `src-tauri/src/lib.rs`

### 2.1 build.rs 的責任

`build.rs` 在 **Rust 編譯階段** 把 `web/` 下面的必要資源複製到 `src-tauri/resources/`：

- 從 `web/` 複製：
  - 目錄：`tiles/`, `dexopt/`, `static/`
  - 檔案：`index.html`, `manifest.json`, `transporter.html`
- 對 `web/mod`：
  - 建立 `resources/mod/js`，只複製特定 JS 檔：
    - `main.bundle.js`
    - `706.main.bundle.js`
    - `main.bundle.js.LICENSE.txt`
    - `index.js`
    - `tauri_bridge.js`
  - 複製 `web/mod/data` → `resources/mod/data`（遞迴，但有過濾條件）。
- 遞迴複製時會 **刻意略過**：
  - 任意 `.pfx` 檔
  - `config.json`
  - `node_modules`
  - `package.json`, `package-lock.json`
  - `webpack.config.cjs`

> 若未來新增資源，建議依照目前邏輯擴充清單，**不要**直接整個 `web/` 打包，以免把不該公開的東西帶進安裝檔。

### 2.2 tauri.conf.json 的 bundle.resources

`src-tauri/tauri.conf.json` 中：

```jsonc
"bundle": {
  "resources": {
    "resources/index.html": "./",
    "resources/manifest.json": "./",
    "resources/transporter.html": "./",
    "resources/static": "./static",
    "resources/tiles": "./tiles",
    "resources/dexopt": "./dexopt",
    "resources/mod": "./mod"
  },
  "targets": "all"
}
```

設計重點：
- 安裝後 **不要再有一層 `resources/` 目錄**，而是直接展平成：
  - 根目錄：`index.html`, `manifest.json`, `transporter.html`
  - 子目錄：`static/`, `tiles/`, `dexopt/`, `mod/`
- 三平台（Windows / macOS / Linux）共享同一份資源結構，便於維護與解除安裝清理。

### 2.3 lib.rs 的 web_root 邏輯

`src-tauri/src/lib.rs` 裡面，在 `setup` 時決定 `web_root`：

- 開發模式 (`cfg!(debug_assertions)` = true)：
  - `web_root = 專案根目錄/web` → 直讀原始檔方便開發。
- 發佈模式：
  - `web_root = app.path().resource_dir()` → 對應打包後安裝路徑中的根目錄（已被 `bundle.resources` 展平成上面那樣）。

HTTP server（Warp）會用這個 `web_root` 提供靜態檔案，路由順序大致為：
1. `/status` → 回傳 ResourceManager 狀態。
2. `/passionfruit/...` → 從隱藏資料夾抓資源。
3. 其他路徑 → `handle_static_file` 從 `web_root` 提供前端檔案。

---

## 3. 退出流程與 Tauri Commands

關鍵檔案：
- `src-tauri/src/commands.rs`
- 前端 JS（不一定在公開庫裡，但流程如下）

### 3.1 exit_app 指令

在 `commands.rs` 中：

- `exit_app(app: AppHandle)` 使用 `app.exit(0)` 優雅地關閉應用，而不是 `std::process::exit`。
- 前端會透過 `window.__TAURI__.core.invoke('exit_app')` 來呼叫這個指令。

### 3.2 前端退出確認

大致流程（位於 `web/mod/js/ui/exitConfirm.js` 等）：

1. 顯示一個自訂的退出確認對話框（文本來自 `exit_prompts.yaml`）。
2. 使用者點「是」：
   - 顯示「正在關閉應用...」，並 disable 按鈕。
   - `await invoke('exit_app')`。
   - 若失敗才用 `window.close()` 備援。

> 若未來要改退出行為，記得同時檢查 commands.rs 和前端 UI JS，保持一致。

---

## 4. 私有 / 公開 Repo 策略

**這段非常重要，請不要隨意改壞。**

目前 Git 設定：
- `main` 分支：完整專案開發分支，對應 **私有** remote `origin`。
- `public` 分支：從 main 同步修改後，刪除 JS 原始碼，只保留 bundle，用來推送到 **公開** remote `public`。

### 4.1 私有（origin）

- Remote：`origin` → `https://github.com/MoLinOwO/ReversedFront_PC.git`
- 分支：`main`
- 內容：
  - 含完整 `web/mod/js/` 原始碼。
  - `.gitignore` 只用來擋 build 產物、憑證檔、config.json 等，不再硬性忽略 JS 原始碼。

**更新流程範例：**

```bash
# 在 main 上開發與測試
cargo tauri build

# 提交並推送私有庫
git add .
git commit -m "feat: ..."
git push origin main
```

### 4.2 公開（public）

- Remote：`public` → `https://github.com/MoLinOwO/ReversedFront_Public.git`
- 分支：遠端使用 `main`，本地用 `public` 分支對應。
- 內容：
  - **不包含** `web/mod/js` 下的原始碼資料夾，只保留 Webpack 打出來的 bundle 檔 (`main.bundle.js`, `706.main.bundle.js` 等)。
  - 不包含 `.pfx`、`config.json`、`web/passionfruit/` 等敏感或體積大的檔案（已有 .gitignore 支援）。

**同步更新公開庫的標準流程（請優先照這個做）：**

```bash
# 1. 確保 main 已經 push 到私有 origin
#    （見上節）

# 2. 切到 public 分支，從 main 合併最新修改
git checkout public
git merge main

# 3. 移除不該公開的 JS 原始碼目錄（若它們又被加回來）
#   這一步有時可以省略，如果 public 裡本來就沒有這些資料夾
rm -rf web/mod/js/account web/mod/js/audio web/mod/js/core web/mod/js/map web/mod/js/ui
rm -f web/mod/js/index.js

# 4. 檢查狀態，確認只剩 bundle & 正常修改
git status

# 5. 提交並推送到公開庫
git add .
git commit -m "chore: refresh public from main without source js"
git push public public:main
```

> 若你是 AI 助手在操作終端，請特別小心 **不要在 public 分支保留多餘的 JS 原始碼**。原則是：只要 bundle 能跑就好，原始碼不必存在公開 repo。

---

## 5. CI / Release 流程

關鍵檔案：`.github/workflows/release.yml`

- 觸發條件：push tag `v*` 到 **公開 repo**（ReversedFront_Public）。
- Matrix 平台：
  - `macos-latest`（`--target universal-apple-darwin`）
  - `ubuntu-22.04`
  - `windows-latest`
- 重點：
  - 已略過 Webpack 步驟，假設 bundle 已經隨 repo 一起 commit。
  - 在 `src-tauri` 跑 `cargo tauri build`。
  - 使用 `softprops/action-gh-release` 上傳各平台產物：`.deb`、`.AppImage`、`.msi`、`.exe`、`.dmg`、`.app.tar.gz`。

**發版標準流程：**

1. 在私有 `main` 開發、測試，並同步到 public（見第 4 節）。
2. 到 GitHub 上的 `ReversedFront_Public` 建立 tag，例如 `v2.11.1`。
3. Actions 自動執行 `Release` workflow，完成後 Release 頁面會掛上所有安裝檔。

---

## 6. 你在修改前應該知道的事

1. **不要隨便改 `bundle.resources` 路徑結構**，除非你也一併調整：
   - `build.rs` 的複製來源與目標
   - `lib.rs` 裡 `web_root` 的解讀方式
2. 若要加入新前端資源：
   - 優先考慮：在 `web/` 新增 → 在 `build.rs` 與 `tauri.conf.json` 對應；
   - 盡量不要把整個 `web/` 或 `node_modules` 打包進去。
3. 有兩套不同關注點：
   - 私有庫（開發與原始碼安全）；
   - 公開庫（發佈與體驗）。
4. 若你是新的 AI 助手：
   - 進 repo 之後，可以先執行：
     - `git remote -v` 確認 origin/public
     - `git branch -a` 看 main/public
     - `cargo tauri build` 確認目前狀態可編譯。

---

## 7. 若未來需要幫助

- 對 JS 原始碼行為不確定時，可以優先查看私有 repo（origin）而不是 public 這一邊。
- 若要新增自動腳本（例如一鍵同步 public），建議放在 `scripts/` 或類似目錄，並在 README 或本檔案補充說明。

> 總之：
> - **不要洩漏原始碼與憑證**；
> - **不要破壞打包路徑結構**；
> - 一切改動，優先考慮「三平台一致」與「安裝/解除安裝乾淨」。

如果你是人類維護者：看到有 AI 想大動 `tauri.conf.json` 或 `build.rs`，請先讓它讀完這份檔案 😄
