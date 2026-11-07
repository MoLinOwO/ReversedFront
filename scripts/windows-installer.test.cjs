const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { packagingConfig } = require('./bundle.cjs');

const project = path.resolve(__dirname, '..');
const nsis = path.join(process.env.LOCALAPPDATA || '', '.cargo-packager/NSIS');
const literal = value => value.replaceAll('$', () => '$$');

// Run the actual generated NSIS section with a harmless prerequisite script.
// This catches runtime string truncation, which successful compilation cannot detect.
for (const exitCode of [0, 17, 'installed runtime']) {
  test('NSIS executes a complete embedded script and handles exit ' + exitCode, { skip: process.platform !== 'win32' }, t => {
    assert.ok(fs.existsSync(path.join(nsis, 'makensis.exe')), 'Run Windows bundling first to install the pinned NSIS compiler');
    const productionScript = fs.readFileSync(path.join(project, 'scripts/install-webview2.ps1'), 'utf8');
    if (typeof exitCode === 'string') {
      // Check only the existing-runtime branch before exercising the real script; never install WebView2 during tests.
      const detection = productionScript.split('$bootstrapRoot =')[0] + '\r\nexit 77';
      const powershell = path.join(process.env.WINDIR, 'SysWOW64/WindowsPowerShell/v1.0/powershell.exe');
      const probe = spawnSync(powershell, ['-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(detection, 'utf16le').toString('base64')], { windowsHide: true, timeout: 15000 });
      assert.ifError(probe.error);
      if (probe.status === 77) { t.skip('WebView2 is not installed; do not change the host runtime'); return; }
      assert.equal(probe.status, 0, 'Existing-runtime detection must succeed before running the real prerequisite');
    }
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'rf-installer-test-'));
    t.after(() => {
      assert.equal(path.dirname(path.resolve(root)), path.resolve(os.tmpdir()));
      fs.rmSync(root, { recursive: true, force: true });
    });
    const fixture = path.join(root, 'setup $ with spaces');
    fs.mkdirSync(path.join(fixture, 'scripts'), { recursive: true });
    fs.copyFileSync(path.join(project, 'packager.json'), path.join(fixture, 'packager.json'));
    const completed = path.join(root, 'script-complete.txt');
    const installed = path.join(root, 'main-section.txt');
    const script = typeof exitCode === 'string' ? productionScript : '# ' + 'long script '.repeat(1000) + '\r\n' +
      "[IO.File]::WriteAllText('" + completed.replaceAll("'", "''") + "', 'complete')\r\nexit " + exitCode + '\r\n';
    fs.writeFileSync(path.join(fixture, 'scripts/install-webview2.ps1'), script);
    const config = packagingConfig(fixture, {}, 'win32');
    const command = config.nsis.preinstallSection.split('\n').find(line => line.startsWith('nsExec::'));
    assert.ok(command.length < 500, 'Leave room for expanded system and temporary paths in the NSIS string buffer');
    const exe = path.join(root, 'prerequisite.exe');
    const source = path.join(root, 'prerequisite.nsi');
    fs.writeFileSync(source, '\ufeff' + [
      'Unicode true', 'Name "ReversedFront prerequisite regression"',
      'OutFile "' + literal(exe) + '"', 'RequestExecutionLevel user', 'SilentInstall silent',
      config.nsis.preinstallSection,
      'Section Main', 'FileOpen $2 "' + literal(installed) + '" w',
      'FileWrite $2 "installed"', 'FileClose $2', 'SectionEnd'
    ].join('\n'));
    const compiled = spawnSync(path.join(nsis, 'makensis.exe'), ['/V2', source], {
      env: { ...process.env, NSISDIR: nsis }, encoding: 'utf8', windowsHide: true, timeout: 30000
    });
    assert.ifError(compiled.error);
    assert.equal(compiled.status, 0, compiled.stdout + compiled.stderr);
    const ran = spawnSync(exe, ['/S'], { encoding: 'utf8', windowsHide: true, timeout: 30000 });
    assert.ifError(ran.error);
    assert.equal(ran.status, exitCode === 17 ? 1 : 0, ran.stdout + ran.stderr);
    if (typeof exitCode === 'number') assert.equal(fs.readFileSync(completed, 'utf8'), 'complete');
    assert.equal(fs.existsSync(installed), exitCode !== 17, 'A failed prerequisite must prevent the main installation section');
  });
}
