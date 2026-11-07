const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { stageInstallers, createUpdateManifest, packageCompiledMod } = require('./release-artifacts.cjs');
const { validateRelease, validateResources } = require('./validate-release.cjs');
const { setVersion } = require('./set-version.cjs');
const { packagingConfig } = require('./bundle.cjs');

const projectRoot = path.resolve(__dirname, '..');
const tag = 'v3.2.7';
const repository = 'MoLinOwO/ReversedFront';
const commit = 'a'.repeat(40);

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'rf-release-test-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  return root;
}

function write(root, file, content = 'public fixture') {
  const destination = path.join(root, file);
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.writeFileSync(destination, content);
}

test('Windows packaging requires an NSIS installer and never substitutes the raw exe', t => {
  const root = fixture(t);
  write(root, 'release/rf-desktop.exe');
  assert.throws(() => stageInstallers(path.join(root, 'release/bundle'), path.join(root, 'out'), 'win32', tag), /nsis/);
  write(root, 'release/bundle/rf-desktop_3.2.7_x64-setup.exe', 'NSIS fixture');
  assert.throws(() => stageInstallers(path.join(root, 'release/bundle'), path.join(root, 'out'), 'win32', tag), /wix/);
  write(root, 'release/bundle/rf-desktop_3.2.7_x64_en-US.msi');
  assert.deepEqual(stageInstallers(path.join(root, 'release/bundle'), path.join(root, 'out'), 'win32', tag),
    ['ReversedFront_v3.2.7.exe', 'ReversedFront_v3.2.7.msi']);
  assert.equal(fs.readFileSync(path.join(root, 'out/ReversedFront_v3.2.7.exe'), 'utf8'), 'NSIS fixture');
  write(root, 'release/bundle/stale-setup.exe');
  assert.throws(() => stageInstallers(path.join(root, 'release/bundle'), path.join(root, 'ambiguous'), 'win32', tag), /exactly one/);
});

test('Linux and macOS retain installer naming and require every mandatory format', t => {
  const root = fixture(t);
  write(root, 'bundle/rf-desktop.AppImage');
  assert.throws(() => stageInstallers(path.join(root, 'bundle'), path.join(root, 'linux'), 'linux', tag), /deb/);
  write(root, 'bundle/rf-desktop.deb');
  assert.deepEqual(stageInstallers(path.join(root, 'bundle'), path.join(root, 'linux'), 'linux', tag),
    ['ReversedFront_v3.2.7.AppImage', 'ReversedFront_v3.2.7.deb']);
  write(root, 'bundle/ReversedFront_universal.dmg');
  assert.deepEqual(stageInstallers(path.join(root, 'bundle'), path.join(root, 'mac'), 'darwin', tag), ['ReversedFront_v3.2.7.dmg']);
  assert.throws(() => stageInstallers(path.join(root, 'bundle'), path.join(root, 'linux'), 'linux', tag), /must be empty/);
  assert.throws(() => stageInstallers(path.join(root, 'bundle'), path.join(root, 'invalid'), 'win32', '../bad'), /Invalid release tag/);
});

function releaseAsset(extension) {
  const name = 'ReversedFront_' + tag + extension;
  return { name, browser_download_url: 'https://github.com/' + repository + '/releases/download/' + tag + '/' + name, size: 100, state: 'uploaded' };
}

test('the static manifest requires published platform installers and omits absent optional app archives', () => {
  const release = { tag_name: tag, assets: ['.exe', '.msi', '.dmg', '.AppImage', '.deb'].map(releaseAsset) };
  const manifest = createUpdateManifest(release, repository, tag, commit);
  assert.equal(manifest.tag_name, tag);
  assert.equal(manifest.assets.length, 5);
  assert.equal(manifest.assets.some(asset => asset.name.endsWith('.msi')), true);
  assert.equal(manifest.assets.some(asset => asset.name.endsWith('.app.tar.gz')), false);
  assert.deepEqual(manifest.mod, { remoteRef: commit,
    filename: 'ReversedFront_Mod_v3.2.7.zip',
    downloadUrl: 'https://github.com/MoLinOwO/ReversedFront/releases/download/v3.2.7/ReversedFront_Mod_v3.2.7.zip' });
  release.assets.push(releaseAsset('.app.tar.gz'));
  assert.equal(createUpdateManifest(release, repository, tag, commit).assets.length, 6);
  release.assets[0].browser_download_url = 'https://example.com/raw.exe';
  assert.throws(() => createUpdateManifest(release, repository, tag, commit), /Invalid release asset/);
  release.assets.shift();
  assert.throws(() => createUpdateManifest(release, repository, tag, commit), /Missing or duplicate installer/);
});

test('Mod archives include compiled chunks, licenses and public data while excluding private/source files', t => {
  const root = fixture(t);
  for (const file of ['js/main.bundle.js', 'js/vendor.bundle.js', 'js/vendor.bundle.js.LICENSE.txt', 'data/routes.json', 'data/names.yaml',
    'js/core/source.js', 'js/main.bundle.js.map', 'data/accounts.json', 'data/config.json', 'data/accounts.db', 'data/accounts.db-journal', 'data/client.pfx']) {
    write(root, 'web/mod/' + file);
  }
  const destination = path.join(root, 'mod');
  packageCompiledMod(root, destination);
  assert.deepEqual(fs.readdirSync(path.join(destination, 'js')).sort(), ['main.bundle.js', 'vendor.bundle.js', 'vendor.bundle.js.LICENSE.txt']);
  assert.deepEqual(fs.readdirSync(path.join(destination, 'data')).sort(), ['names.yaml', 'routes.json']);
});

test('validation accepts untracked GPUI migration files with standalone packaging and retains tracked privacy checks', t => {
  const root = fixture(t);
  const currentTag = 'v' + JSON.parse(fs.readFileSync(path.join(projectRoot, 'package.json'), 'utf8')).version;
  for (const file of ['package.json', 'package-lock.json', 'rust-toolchain.toml', 'desktop/Cargo.toml', 'desktop/Cargo.lock', 'desktop/build.rs', 'packager.json',
    'desktop/src/gpui_host.rs', 'desktop/src/host.rs', 'desktop/src/ipc.rs', 'desktop/src/native_bridge.js', 'scripts/native-bridge.test.cjs',
    'scripts/install-webview2.ps1', 'scripts/windows-installer.wxs']) {
    write(root, file, fs.readFileSync(path.join(projectRoot, file)));
  }
  for (const file of ['web/index.html', 'web/mod/js/main.bundle.js', 'web/mod/data/exit_prompts.yaml', 'web/mod/data/transportRoutes.json']) write(root, file);
  write(root, 'web/desktop_bridge.js', 'void 0;');
  write(root, 'web/asset-manifest.json', JSON.stringify({ files: { index: './index.html' } }));
  assert.deepEqual(validateRelease(root, { trackedFiles: [], releaseTag: currentTag }).errors, []);
  assert.match(validateRelease(root, { trackedFiles: [], releaseTag: 'v0.0.0' }).errors.join('\n'), /Release tag/);
  write(root, 'web/mod/js/core/source.js');
  write(root, 'web/accounts.db');
  assert.equal(validateRelease(root, { trackedFiles: [] }).errors.length, 0);
  assert.equal(validateRelease(root, { trackedFiles: ['web/mod/js/core/source.js', 'web/accounts.db'] }).errors.length, 2);
  write(root, 'web/asset-manifest.json', JSON.stringify({ files: { missing: './static/missing.js', escape: '../package.json' } }));
  assert.equal(validateRelease(root, { trackedFiles: [] }).errors.length, 2);
});

test('staged resource checks catch ignored databases, source files and source maps before bundling', t => {
  const root = fixture(t);
  for (const file of ['index.html', 'desktop_bridge.js', 'favicon.ico', 'transporter.html', 'mod/js/main.bundle.js', 'mod/data/routes.json']) write(root, file);
  assert.deepEqual(validateResources(root), []);
  for (const file of ['mod/data/accounts.db', 'static/accounts.db-journal', 'mod/js/core/source.js']) write(root, file);
  write(root, 'mod/js/vendor.bundle.js', '//# sourceMappingURL=private.js.map');
  assert.equal(validateResources(root).length, 4);
});

test('version synchronization updates only the application lock entries and fails before partial writes', t => {
  const root = fixture(t);
  write(root, 'package.json', JSON.stringify({ name: 'rf-desktop', version: '3.2.7' }));
  write(root, 'package-lock.json', JSON.stringify({ version: '3.2.7', lockfileVersion: 3, packages: { '': { version: '3.2.7' } } }));
  write(root, 'packager.json', JSON.stringify({ version: '3.2.7' }));
  write(root, 'desktop/Cargo.toml', '[package]\nname = "rf-desktop"\nversion = "3.2.7"\n\n[dependencies]\npeer = "3.2.7"\n');
  write(root, 'desktop/Cargo.lock', 'version = 4\n\n[[package]]\nname = "peer"\nversion = "3.2.7"\n\n[[package]]\nname = "rf-desktop"\nversion = "3.2.7"\ndependencies = ["peer"]\n');
  assert.equal(setVersion(root, 'v3.2.8'), '3.2.8');
  const read = file => fs.readFileSync(path.join(root, file), 'utf8');
  const json = file => JSON.parse(read(file));
  assert.equal(json('package.json').version, '3.2.8');
  assert.equal(json('package-lock.json').version, '3.2.8');
  assert.equal(json('package-lock.json').lockfileVersion, 3);
  assert.equal(json('package-lock.json').packages[''].version, '3.2.8');
  assert.equal(json('packager.json').version, '3.2.8');
  assert.equal(read('desktop/Cargo.toml'), '[package]\nname = "rf-desktop"\nversion = "3.2.8"\n\n[dependencies]\npeer = "3.2.7"\n');
  assert.equal(read('desktop/Cargo.lock'), 'version = 4\n\n[[package]]\nname = "peer"\nversion = "3.2.7"\n\n[[package]]\nname = "rf-desktop"\nversion = "3.2.8"\ndependencies = ["peer"]\n');
  write(root, 'desktop/Cargo.lock', 'version = 4\n');
  const files = ['package.json', 'package-lock.json', 'packager.json', 'desktop/Cargo.toml', 'desktop/Cargo.lock'];
  const before = files.map(read);
  assert.throws(() => setVersion(root, 'v3.2.9'), /Expected one rf-desktop/);
  assert.deepEqual(files.map(read), before);
});

test('standalone configs preserve runtime resources and embed WebView2 setup in both Windows installers', () => {
  const windows = packagingConfig(projectRoot, { debug: true }, 'win32');
  assert.deepEqual(windows.formats, ['nsis', 'wix']);
  assert.equal(windows.binariesDir, path.join(projectRoot, 'desktop/target/debug'));
  assert.equal(windows.outDir, path.join(projectRoot, 'desktop/target/packages/debug'));
  assert.deepEqual(windows.icons, [path.join(projectRoot, 'desktop/icons/icon.ico')]);
  const encoded = windows.nsis.preinstallSection.match(/-EncodedCommand ([A-Za-z0-9+/=]+)/)[1];
  assert.equal(Buffer.from(encoded, 'base64').toString('utf16le'), fs.readFileSync(path.join(projectRoot, 'scripts/install-webview2.ps1'), 'utf8'));
  assert.ok(windows.wix.fragments[0].includes('-EncodedCommand ' + encoded));
  assert.deepEqual(windows.wix.customActionRefs, ['RFInstallWebView2']);
  assert.match(fs.readFileSync(windows.wix.template, 'utf8'), /UpgradeCode="4BF3EE1C-A078-5E94-A1D9-FE863DD477ED"/);
  const linux = packagingConfig(projectRoot, {}, 'linux');
  assert.deepEqual(linux.formats, ['appimage', 'deb']);
  assert.ok(linux.deb.depends.includes('libwebkit2gtk-4.1-0'));
  assert.ok(linux.deb.depends.includes('libgtk-3-0'));
  assert.ok(linux.appimage.libs.includes('libwebkit2gtk-4.1.so.0'));
  assert.ok(linux.resources.every(resource => resource.src.startsWith(path.join(projectRoot, 'desktop/resources'))));
  const mac = packagingConfig(projectRoot, { target: 'universal-apple-darwin' }, 'darwin');
  assert.equal(mac.targetTriple, 'universal-apple-darwin');
  assert.equal(mac.macos.minimumSystemVersion, '15.0');
  assert.deepEqual(mac.icons, [path.join(projectRoot, 'desktop/icons/icon.png')]);
  assert.deepEqual(linux.icons, mac.icons);
  const png = fs.readFileSync(mac.icons[0]);
  assert.equal(png.subarray(1, 4).toString(), 'PNG');
  assert.equal(png[25], 6, 'ICNS conversion requires the RGBA PNG, not the RGB ICO payload');
});
