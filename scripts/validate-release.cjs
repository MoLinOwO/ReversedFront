const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');

const projectRoot = path.resolve(__dirname, '..');
const resourceMappings = {
  'desktop/resources/index.html': 'index.html',
  'desktop/resources/desktop_bridge.js': 'desktop_bridge.js',
  'desktop/resources/favicon.ico': 'favicon.ico',
  'desktop/resources/transporter.html': 'transporter.html',
  'desktop/resources/static': 'static',
  'desktop/resources/tiles': 'tiles',
  'desktop/resources/mod': 'mod'
};

function isPrivatePath(file) {
  return /\.(?:pfx|p12|pem|key|db(?:-wal|-shm|-journal)?|sqlite3?(?:-wal|-shm|-journal)?)$/i.test(file)
    || /(?:^|\/)(?:accounts|config)\.json$/i.test(file);
}

function isCompiledModJs(file) {
  return !file.includes('/') && /\.bundle\.js(?:\.LICENSE\.txt)?$/.test(file);
}

function isPublicModData(file) {
  return !file.includes('/') && !isPrivatePath(file) && /\.(?:json|yaml|yml|svg)$/.test(file);
}

function resourceFiles(directory, errors, prefix = '') {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const relative = prefix + entry.name;
    if (entry.isDirectory()) {
      return resourceFiles(path.join(directory, entry.name), errors, relative + '/');
    }
    if (!entry.isFile()) {
      errors.push('打包資源不得包含連結或特殊檔案：' + relative);
      return [];
    }
    return [relative];
  });
}

function validateResources(directory) {
  const errors = [];
  if (!fs.existsSync(directory)) return ['缺少 desktop/resources，請先執行 npm run build'];
  const files = resourceFiles(directory, errors);
  for (const required of ['index.html', 'desktop_bridge.js', 'favicon.ico', 'transporter.html', 'mod/js/main.bundle.js']) {
    if (!files.includes(required)) errors.push('缺少打包資源：' + required);
  }
  for (const file of files) {
    const allowed = ['index.html', 'desktop_bridge.js', 'favicon.ico', 'transporter.html'].includes(file)
      || file.startsWith('static/') || file.startsWith('tiles/')
      || (file.startsWith('mod/js/') && isCompiledModJs(file.slice('mod/js/'.length)))
      || (file.startsWith('mod/data/') && isPublicModData(file.slice('mod/data/'.length)));
    if (!allowed || isPrivatePath(file)) errors.push('不得打包原始碼或私人資料：' + file);
    if (file.startsWith('mod/js/') && file.endsWith('.bundle.js')
      && /sourceMappingURL\s*=/.test(fs.readFileSync(path.join(directory, file), 'utf8'))) {
      errors.push('Mod bundle 不得引用 source map：' + file);
    }
  }
  return errors;
}

function validateRelease(root = projectRoot, { trackedFiles, resources = false, releaseTag = process.env.RELEASE_TAG } = {}) {
  const errors = [];
  trackedFiles ??= execFileSync('git', ['ls-files', '-z'], { cwd: root, encoding: 'utf8' }).split('\0');
  trackedFiles = trackedFiles.filter(Boolean).map(file => file.replaceAll('\\', '/'))
    .filter(file => fs.existsSync(path.join(root, file)));
  const requiredFiles = [
    'package.json', 'package-lock.json', 'rust-toolchain.toml',
    'desktop/Cargo.toml', 'desktop/Cargo.lock', 'desktop/build.rs', 'packager.json',
    'desktop/src/gpui_host.rs', 'desktop/src/host.rs', 'desktop/src/ipc.rs',
    'desktop/src/native_bridge.js', 'scripts/native-bridge.test.cjs',
    'scripts/install-webview2.ps1', 'scripts/windows-installer.wxs',
    'web/index.html', 'web/desktop_bridge.js', 'web/asset-manifest.json',
    'web/mod/js/main.bundle.js', 'web/mod/data/exit_prompts.yaml', 'web/mod/data/transportRoutes.json'
  ];
  // New migration files can be untracked while the user reviews the changes.
  for (const file of requiredFiles) {
    if (!fs.statSync(path.join(root, file), { throwIfNoEntry: false })?.isFile()) {
      errors.push('缺少發版必要檔案：' + file);
    }
  }
  if (errors.length) return { errors, trackedFiles };

  const readJson = file => JSON.parse(fs.readFileSync(path.join(root, file), 'utf8'));
  const pkg = readJson('package.json');
  const lock = readJson('package-lock.json');
  if (Object.entries(lock.packages || {}).some(([name, entry]) =>
    (name && !name.startsWith('node_modules/')) || entry.link || /^(?:file:|[A-Za-z]:)/.test(entry.resolved || ''))) {
    errors.push('npm lockfile 不得包含本機路徑或連結');
  }
  const config = readJson('packager.json');
  const cargo = fs.readFileSync(path.join(root, 'desktop/Cargo.toml'), 'utf8');
  const packageSection = cargo.match(/^\[package\]\r?\n([\s\S]*?)(?=^\[|(?![\s\S]))/m)?.[1] || '';
  const cargoVersion = packageSection.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  const cargoLockVersion = fs.readFileSync(path.join(root, 'desktop/Cargo.lock'), 'utf8')
    .match(/\[\[package\]\]\r?\nname\s*=\s*"rf-desktop"\r?\nversion\s*=\s*"([^"]+)"/)?.[1];
  if (pkg.name !== 'rf-desktop' || !/^name\s*=\s*"rf-desktop"/m.test(packageSection)
    || config.productName !== 'ReversedFront' || config.identifier !== 'com.reversedfront.desktop'
    || config.name !== 'rf-desktop' || config.binaries?.length !== 1
    || config.binaries[0].path !== 'rf-desktop' || !config.binaries[0].main) {
    errors.push('必須保留 rf-desktop 執行檔與 ReversedFront 安裝識別');
  }
  if (![lock.version, lock.packages?.['']?.version, config.version, cargoVersion, cargoLockVersion].every(version => version === pkg.version)) {
    errors.push('npm、lockfile、Cargo 與安裝包版本必須一致');
  }
  if (releaseTag && releaseTag !== 'v' + pkg.version) errors.push('Release tag 必須為 v' + pkg.version + '：' + releaseTag);
  if (!/^channel\s*=\s*"1\.99\.1"\s*$/m.test(fs.readFileSync(path.join(root, 'rust-toolchain.toml'), 'utf8'))) {
    errors.push('Rust toolchain 必須釘選 1.99.1');
  }
  const mappings = config.resources;
  const webkitDirectory = '/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1';
  if (!Array.isArray(mappings) || mappings.length !== Object.keys(resourceMappings).length
    || !Object.entries(resourceMappings).every(([source, destination]) => mappings.some(resource => resource.src === source && resource.target === destination))
    || config.externalBinaries?.length || Object.keys(config.deb?.files || {}).length
    || Object.keys(config.appimage?.files || {}).length !== 1
    || config.appimage.files[webkitDirectory] !== webkitDirectory.slice(1)) {
    errors.push('安裝包只能包含 build.rs 產生的指定 resources，不得加入原始碼或私人資料');
  }

  const webRoot = path.join(root, 'web');
  const manifest = readJson('web/asset-manifest.json');
  for (const asset of Object.values(manifest.files || {})) {
    const relative = String(asset).replace(/^\.\//, '');
    const normalized = relative.replace(/\.([0-9a-f]{20})\.\1\./i, '.$1.');
    const resolved = path.relative(webRoot, path.resolve(webRoot, relative));
    if (resolved.startsWith('..') || path.isAbsolute(resolved)
      || (!fs.existsSync(path.join(webRoot, relative)) && !fs.existsSync(path.join(webRoot, normalized)))) {
      errors.push('官方 asset manifest 指向不存在或不安全的資源：' + relative);
    }
  }

  for (const file of trackedFiles) {
    if (file.startsWith('web/mod/js/') && !isCompiledModJs(file.slice('web/mod/js/'.length))) {
      errors.push('公開倉庫不得包含 Mod JS 原始碼：' + file);
    }
    if (isPrivatePath(file)) errors.push('公開倉庫不得包含憑證或使用者資料：' + file);
    if (file.startsWith('web/mod/') && file.toLowerCase().endsWith('.map')) {
      errors.push('公開倉庫不得包含 Mod source map：' + file);
    }
  }
  for (const file of fs.readdirSync(path.join(webRoot, 'mod/js')).filter(file => file.endsWith('.bundle.js'))) {
    if (/sourceMappingURL\s*=/.test(fs.readFileSync(path.join(webRoot, 'mod/js', file), 'utf8'))) {
      errors.push('Mod bundle 不得引用 source map：' + file);
    }
  }
  for (const file of ['web/desktop_bridge.js', 'desktop/src/native_bridge.js']) {
    try {
      // Parse only: these scripts require browser globals.
      new Function(fs.readFileSync(path.join(root, file), 'utf8'));
    } catch (error) {
      errors.push(file + ' 語法錯誤：' + error.message);
    }
  }
  if (resources) errors.push(...validateResources(path.join(root, 'desktop/resources')));
  return { errors, trackedFiles };
}

if (require.main === module) {
  try {
    const { errors, trackedFiles } = validateRelease(projectRoot, { resources: process.argv.includes('--resources') });
    if (errors.length) throw new Error(errors.map(error => '- ' + error).join('\n'));
    console.log('發版檢查通過：' + trackedFiles.length + ' 個追蹤檔案，Mod 僅包含編譯 bundle。');
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

module.exports = { validateRelease, validateResources, isPrivatePath, isCompiledModJs, isPublicModData };
