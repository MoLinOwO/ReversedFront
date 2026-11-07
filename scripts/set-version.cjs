const fs = require('node:fs');
const path = require('node:path');

function setVersion(root, tag) {
  const version = tag.replace(/^v/i, '').trim();
  if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error('Invalid release tag: ' + tag);
  }
  const read = file => fs.readFileSync(path.join(root, file), 'utf8');
  const pkg = JSON.parse(read('package.json'));
  const lock = JSON.parse(read('package-lock.json'));
  const config = JSON.parse(read('packager.json'));
  const cargo = read('desktop/Cargo.toml');
  const cargoLock = read('desktop/Cargo.lock');
  const packageSection = cargo.match(/^\[package\]\r?\n[\s\S]*?(?=^\[|(?![\s\S]))/m)?.[0];
  const versionPattern = /(^version\s*=\s*")[^"]+(")/m;
  const lockPattern = /(\[\[package\]\]\r?\nname\s*=\s*"rf-desktop"\r?\nversion\s*=\s*")[^"]+(")/g;
  if (!packageSection || !versionPattern.test(packageSection) || [...cargoLock.matchAll(lockPattern)].length !== 1) {
    throw new Error('Expected one rf-desktop package version in Cargo.toml and Cargo.lock');
  }
  const updatedCargo = cargo.replace(packageSection,
    packageSection.replace(versionPattern, (_, prefix, suffix) => prefix + version + suffix));
  const updatedLock = cargoLock.replace(lockPattern, (_, prefix, suffix) => prefix + version + suffix);

  // Validate every input before writing; only the root package lock entry changes.
  pkg.version = version;
  lock.version = version;
  if (!lock.packages?.['']) throw new Error('Missing npm root lockfile entry');
  lock.packages[''].version = version;
  config.version = version;
  for (const [file, value] of [['package.json', pkg], ['package-lock.json', lock], ['packager.json', config]]) {
    fs.writeFileSync(path.join(root, file), JSON.stringify(value, null, 2) + '\n');
  }
  fs.writeFileSync(path.join(root, 'desktop/Cargo.toml'), updatedCargo);
  fs.writeFileSync(path.join(root, 'desktop/Cargo.lock'), updatedLock);
  return version;
}

if (require.main === module) {
  const version = setVersion(path.resolve(__dirname, '..'), process.env.RELEASE_TAG || process.argv[2] || '');
  console.log('Release version set to ' + version);
}

module.exports = { setVersion };
