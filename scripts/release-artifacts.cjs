const fs = require('node:fs');
const path = require('node:path');
const { isCompiledModJs, isPublicModData } = require('./validate-release.cjs');

const installerFormats = {
  win32: [['nsis', '.exe', true], ['wix', '.msi', true]],
  darwin: [['dmg', '.dmg', true], ['macos', '.app.tar.gz', false]],
  linux: [['appimage', '.AppImage', true], ['deb', '.deb', true]]
};

function assertTag(tag) {
  if (!/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(tag || '')) {
    throw new Error('Invalid release tag: ' + tag);
  }
}

function createEmptyDirectory(directory) {
  fs.mkdirSync(directory, { recursive: true });
  if (fs.readdirSync(directory).length) throw new Error('Artifact directory must be empty: ' + directory);
}

function stageInstallers(bundleDirectory, outputDirectory, platform, tag) {
  assertTag(tag);
  if (!installerFormats[platform]) throw new Error('Unsupported release platform: ' + platform);
  const installers = [];
  for (const [format, extension, required] of installerFormats[platform]) {
    const directory = bundleDirectory;
    const candidates = fs.existsSync(directory)
      ? fs.readdirSync(directory, { withFileTypes: true }).filter(entry => entry.isFile() && entry.name.endsWith(extension))
      : [];
    if (candidates.length > 1 || (required && candidates.length !== 1)) {
      throw new Error('Expected exactly one ' + format + ' installer in ' + directory);
    }
    if (candidates.length) {
      const source = path.join(directory, candidates[0].name);
      if (!fs.statSync(source).size) throw new Error('Empty installer: ' + source);
      installers.push({ source, name: 'ReversedFront_' + tag + extension });
    }
  }
  createEmptyDirectory(outputDirectory);
  // Only collect the standalone packager's output, never Cargo's raw executable.
  for (const installer of installers) fs.copyFileSync(installer.source, path.join(outputDirectory, installer.name));
  return installers.map(installer => installer.name);
}

function createUpdateManifest(release, repository, tag, commit) {
  assertTag(tag);
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository || '') || !/^[0-9a-f]{40}$/i.test(commit || '')) {
    throw new Error('A GitHub repository and full commit SHA are required');
  }
  if (release.tag_name !== tag || !Array.isArray(release.assets)) throw new Error('Release does not match ' + tag);
  const base = 'https://github.com/' + repository + '/releases/download/' + tag;
  const formats = Object.values(installerFormats).flat();
  const assets = [];
  for (const [, extension, required] of formats) {
    const name = 'ReversedFront_' + tag + extension;
    const matches = release.assets.filter(asset => asset.name === name);
    if (matches.length > 1 || (required && matches.length !== 1)) throw new Error('Missing or duplicate installer: ' + name);
    if (!matches.length) continue;
    const asset = matches[0];
    if (asset.browser_download_url !== base + '/' + name || asset.size === 0
      || (asset.state && asset.state !== 'uploaded')) throw new Error('Invalid release asset: ' + name);
    assets.push({ name, browser_download_url: asset.browser_download_url });
  }
  const filename = 'ReversedFront_Mod_' + tag + '.zip';
  return { tag_name: tag, assets, mod: { remoteRef: commit, downloadUrl: base + '/' + filename, filename } };
}

function packageCompiledMod(root, outputDirectory) {
  const sourceDirectory = path.join(root, 'web/mod');
  if (!fs.statSync(path.join(sourceDirectory, 'js/main.bundle.js'), { throwIfNoEntry: false })?.isFile()) {
    throw new Error('Compiled Mod requires js/main.bundle.js');
  }
  createEmptyDirectory(outputDirectory);
  for (const [directory, allowed] of [['js', isCompiledModJs], ['data', isPublicModData]]) {
    const source = path.join(sourceDirectory, directory);
    const destination = path.join(outputDirectory, directory);
    fs.mkdirSync(destination);
    for (const file of fs.readdirSync(source, { withFileTypes: true })) {
      if (file.isFile() && allowed(file.name)) fs.copyFileSync(path.join(source, file.name), path.join(destination, file.name));
    }
  }
}

if (require.main === module) {
  try {
    const root = path.resolve(__dirname, '..');
    const targetRoot = path.join(root, 'desktop/target');
    const tag = process.env.RELEASE_TAG;
    switch (process.argv[2]) {
      case 'stage': {
        const bundleDirectory = path.join(targetRoot, 'packages/release');
        console.log(stageInstallers(bundleDirectory, path.join(targetRoot, 'release-assets'), process.platform, tag).join('\n'));
        break;
      }
      case 'mod':
        packageCompiledMod(root, path.join(targetRoot, 'release-mod'));
        break;
      case 'manifest': {
        const release = JSON.parse(fs.readFileSync(0, 'utf8'));
        const manifest = createUpdateManifest(release, process.env.GITHUB_REPOSITORY, tag, process.env.COMMIT_SHA);
        fs.writeFileSync(path.join(root, 'update.json'), JSON.stringify(manifest, null, 2) + '\n');
        break;
      }
      default:
        throw new Error('Usage: node scripts/release-artifacts.cjs stage | mod | manifest < release.json');
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

module.exports = { stageInstallers, createUpdateManifest, packageCompiledMod };
