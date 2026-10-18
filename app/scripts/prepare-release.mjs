import { copyFile, mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const config = JSON.parse(await readFile(path.join(appRoot, 'src-tauri/tauri.conf.json'), 'utf8'));
const version = config.version;
const tag = process.env.RELEASE_TAG || `v${version}`;
const repository = process.env.GITHUB_REPOSITORY || 'Lyeifal/unzip-rs';
if (tag !== `v${version}`) throw new Error(`Tag ${tag} must match tauri.conf.json version v${version}`);
if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) throw new Error('Invalid GitHub repository');
const bundleDir = path.join(appRoot, 'src-tauri/target/release/bundle/nsis');
const installers = (await readdir(bundleDir)).filter(name => name === `${config.productName}_${version}_x64-setup.exe`);
if (installers.length !== 1) throw new Error(`Expected one x64 installer, found ${installers.length}`);
const installer = path.join(bundleDir, installers[0]);
const signature = (await readFile(`${installer}.sig`, 'utf8')).trim();
if (!signature) throw new Error('Missing updater signature');
const output = path.join(appRoot, 'src-tauri/target/release/update');
await mkdir(output, { recursive: true });
const assetName = `unzip-rs_${version}_x64-setup.exe`;
await copyFile(installer, path.join(output, assetName));
await copyFile(`${installer}.sig`, path.join(output, `${assetName}.sig`));
const manifest = {
  version,
  notes: `https://github.com/${repository}/releases/tag/${tag}`,
  pub_date: new Date().toISOString(),
  platforms: {
    'windows-x86_64': {
      signature,
      url: `https://github.com/${repository}/releases/download/${tag}/${assetName}`,
    },
  },
};
await writeFile(path.join(output, 'latest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`Prepared signed Windows update ${version} in ${output}`);
