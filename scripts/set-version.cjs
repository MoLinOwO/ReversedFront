const fs = require('fs');
const path = require('path');

const tag = process.env.RELEASE_TAG || process.argv[2] || '';
const version = tag.replace(/^v/i, '').trim();

if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error(`Invalid release tag: ${tag}`);
}

const root = path.resolve(__dirname, '..');

function readJson(file) {
    return JSON.parse(fs.readFileSync(path.join(root, file), 'utf8'));
}

function writeJson(file, value) {
    fs.writeFileSync(path.join(root, file), `${JSON.stringify(value, null, 2)}\n`);
}

const packageJson = readJson('package.json');
packageJson.version = version;
writeJson('package.json', packageJson);

const lockFile = path.join(root, 'package-lock.json');
if (fs.existsSync(lockFile)) {
    const lockJson = JSON.parse(fs.readFileSync(lockFile, 'utf8'));
    if (lockJson.packages && lockJson.packages['']) {
        lockJson.packages[''].version = version;
    }
    lockJson.version = lockJson.version || 3;
    writeJson('package-lock.json', lockJson);
}

const tauriConfigPath = path.join(root, 'src-tauri', 'tauri.conf.json');
const tauriConfig = JSON.parse(fs.readFileSync(tauriConfigPath, 'utf8'));
tauriConfig.version = version;
fs.writeFileSync(tauriConfigPath, `${JSON.stringify(tauriConfig, null, 2)}\n`);

const cargoPath = path.join(root, 'src-tauri', 'Cargo.toml');
const cargo = fs.readFileSync(cargoPath, 'utf8');
const cargoVersionPattern = /^version\s*=\s*"[^"]+"/m;
if (!cargoVersionPattern.test(cargo)) {
    throw new Error('Could not update src-tauri/Cargo.toml package version');
}
const updatedCargo = cargo.replace(cargoVersionPattern, `version = "${version}"`);
fs.writeFileSync(cargoPath, updatedCargo);

console.log(`Release version set to ${version}`);
