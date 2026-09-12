const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');

const projectRoot = path.resolve(__dirname, '..');
const trackedFiles = execFileSync('git', ['ls-files', '-z'], {
  cwd: projectRoot,
  encoding: 'utf8'
})
  .split('\0')
  .filter(Boolean)
  .map(file => file.replaceAll('\\', '/'))
  .filter(file => fs.existsSync(path.join(projectRoot, file)));

const errors = [];
const requiredFiles = [
  'src-tauri/web/index.html',
  'web/asset-manifest.json',
  'web/mod/js/main.bundle.js',
  'web/mod/data/exit_prompts.yaml',
  'web/mod/data/transportRoutes.json'
];

const assetManifestPath = path.join(projectRoot, 'web/asset-manifest.json');
if (fs.existsSync(assetManifestPath)) {
  const assetManifest = JSON.parse(fs.readFileSync(assetManifestPath, 'utf8'));
  for (const asset of Object.values(assetManifest.files || {})) {
    const relative = String(asset).replace(/^\.\//, '');
    const normalized = relative.replace(/\.([0-9a-f]{20})\.\1\./i, '.$1.');
    if (!fs.existsSync(path.join(projectRoot, 'web', relative))
      && !fs.existsSync(path.join(projectRoot, 'web', normalized))) {
      errors.push(`官方 asset manifest 指向不存在的資源：${relative}`);
    }
  }
}

const tauriConfigPath = path.join(projectRoot, 'src-tauri/tauri.conf.json');
const tauriConfig = JSON.parse(fs.readFileSync(tauriConfigPath, 'utf8'));
if (tauriConfig.build?.frontendDist !== 'web') {
  errors.push('Tauri frontendDist 必須保持為 src-tauri/web，避免把完整 web/ 與 Mod 原始碼嵌入程式');
}

for (const required of requiredFiles) {
  if (!trackedFiles.includes(required)) {
    errors.push(`缺少發版必要檔案：${required}`);
  }
}

for (const file of trackedFiles) {
  const lower = file.toLowerCase();
  if (file.startsWith('web/mod/js/')) {
    const relative = file.slice('web/mod/js/'.length);
    const isRootFile = !relative.includes('/');
    const isCompiledBundle = relative.endsWith('.bundle.js')
      || relative.endsWith('.bundle.js.LICENSE.txt');
    if (!isRootFile || !isCompiledBundle) {
      errors.push(`公開倉庫不得包含 Mod JS 原始碼：${file}`);
    }
  }

  if (
    lower.endsWith('.pfx')
    || lower.endsWith('.p12')
    || lower.endsWith('.pem')
    || lower.endsWith('.key')
    || lower.endsWith('.db')
    || lower.endsWith('.db-wal')
    || lower.endsWith('.db-shm')
    || lower.endsWith('/accounts.json')
    || lower.endsWith('/config.json')
    || lower === 'accounts.json'
    || lower === 'config.json'
  ) {
    errors.push(`公開倉庫不得包含憑證或使用者資料：${file}`);
  }

  if (lower.endsWith('.map') && file.startsWith('web/mod/')) {
    errors.push(`公開倉庫不得包含 Mod source map：${file}`);
  }
}

const bundlePath = path.join(projectRoot, 'web/mod/js/main.bundle.js');
if (fs.existsSync(bundlePath)) {
  const bundle = fs.readFileSync(bundlePath, 'utf8');
  if (/sourceMappingURL\s*=/.test(bundle)) {
    errors.push('main.bundle.js 不得引用 source map');
  }
}

const desktopBridgePath = path.join(projectRoot, 'web/desktop_bridge.js');
if (fs.existsSync(desktopBridgePath)) {
  try {
    // Parse only. The bridge needs browser globals and must not execute in Node.
    new Function(fs.readFileSync(desktopBridgePath, 'utf8'));
  } catch (error) {
    errors.push(`desktop_bridge.js 語法錯誤：${error.message}`);
  }
}

if (errors.length > 0) {
  console.error(errors.map(error => `- ${error}`).join('\n'));
  process.exit(1);
}

console.log(`發版檢查通過：${trackedFiles.length} 個追蹤檔案，Mod 僅包含編譯 bundle。`);
