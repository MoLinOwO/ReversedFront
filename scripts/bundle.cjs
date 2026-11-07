const fs = require('node:fs');
const path = require('node:path');
const { parseArgs } = require('node:util');
const { spawnSync } = require('node:child_process');

function packagingConfig(root, { debug = false, target, formats } = {}, platform = process.platform) {
  const config = JSON.parse(fs.readFileSync(path.join(root, 'packager.json'), 'utf8'));
  const profile = debug ? 'debug' : 'release';
  if (target && !/^[A-Za-z0-9_-]+$/.test(target)) throw new Error('Invalid target: ' + target);
  config.binariesDir = path.join(root, 'desktop/target', target || '', profile);
  config.outDir = path.join(root, 'desktop/target/packages', profile);
  config.formats = formats ? formats.split(',') : {
    win32: ['nsis', 'wix'], darwin: ['app', 'dmg'], linux: ['appimage', 'deb']
  }[platform];
  if (!config.formats?.length) throw new Error('Unsupported packaging platform: ' + platform);
  if (target) config.targetTriple = target;
  config.icons = config.icons.map(icon => path.join(root, icon));
  config.resources = config.resources.map(resource => ({ ...resource, src: path.join(root, resource.src) }));
  config.wix.template = path.join(root, config.wix.template);

  if (platform === 'win32') {
    const script = fs.readFileSync(path.join(root, 'scripts/install-webview2.ps1'), 'utf8');
    const encoded = Buffer.from(script, 'utf16le').toString('base64');
    const flags = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand ' + encoded;
    // Documented NSIS preinstallSection and WiX fragments share one signed bootstrapper.
    config.nsis.preinstallSection = [
      'Section WebView2',
      "nsExec::ExecToStack /TIMEOUT=600000 '\"$SYSDIR\\WindowsPowerShell\\v1.0\\powershell.exe\" " + flags + "'",
      'Pop $0',
      'Pop $1',
      'StrCmp $0 "0" rf_webview2_ready',
      'MessageBox MB_ICONSTOP "WebView2 setup failed. Check your connection and retry: $1"',
      'Abort',
      'rf_webview2_ready:',
      'SectionEnd'
    ].join('\n');
    config.wix.fragments = [
      '<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi"><Fragment>' +
      '<CustomAction Id="RFInstallWebView2" Directory="INSTALLDIR" Execute="deferred" Impersonate="no" Return="check" ' +
      'ExeCommand="&quot;[SystemFolder]WindowsPowerShell\\v1.0\\powershell.exe&quot; ' + flags + '" />' +
      '<InstallExecuteSequence><Custom Action="RFInstallWebView2" Before="InstallFinalize">NOT REMOVE</Custom></InstallExecuteSequence>' +
      '</Fragment></Wix>'
    ];
    config.wix.customActionRefs = ['RFInstallWebView2'];
  }
  // ponytail: Linux CI is x86_64; add the matching WebKit helper path before adding ARM64 runners.
  return config;
}

if (require.main === module) {
  const root = path.resolve(__dirname, '..');
  const { values } = parseArgs({ options: {
    debug: { type: 'boolean' }, target: { type: 'string' }, formats: { type: 'string' }
  } });
  const config = packagingConfig(root, values);
  const generated = path.join(root, 'desktop/target', 'packager-' + process.pid + '.json');
  fs.mkdirSync(path.dirname(generated), { recursive: true });
  fs.writeFileSync(generated, JSON.stringify(config, null, 2));
  try {
    const result = spawnSync(process.execPath, [require.resolve('@crabnebula/packager/packager.js'), '--config', generated], {
      cwd: root,
      stdio: 'inherit',
      env: {
        ...process.env,
        DEPLOY_GTK_VERSION: '3',
        // ponytail: matches the pinned packager's Windows tool cache; recheck on upgrades.
        ...(process.platform === 'win32' && process.env.LOCALAPPDATA
          ? { NSISDIR: path.join(process.env.LOCALAPPDATA, '.cargo-packager/NSIS') } : {})
      }
    });
    if (result.error) console.error(result.error.message);
    process.exitCode = result.status ?? 1;
  } finally {
    fs.unlinkSync(generated);
  }
}

module.exports = { packagingConfig };
