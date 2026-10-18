// Browser regression for selectable pack-name parts. PLAYWRIGHT_MODULE may
// point to an existing Playwright ESM install. Only mocked Tauri data is used.
import { createServer } from 'vite';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
import assert from 'node:assert/strict';

const playwrightModule = process.env.PLAYWRIGHT_MODULE;
const { chromium } = await import(playwrightModule && path.isAbsolute(playwrightModule) ? pathToFileURL(playwrightModule).href : playwrightModule || 'playwright');
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const server = await createServer({ root, configFile: path.join(root, 'vite.config.ts'), server: { host: '127.0.0.1', port: 0, strictPort: false } });
await server.listen();
const browser = await chromium.launch({ channel: 'msedge', headless: true });
const page = await browser.newPage();
const errors = [];
page.on('pageerror', error => errors.push(error.message));
try {
  await page.addInitScript(() => {
    window.__TAURI_INTERNALS__ = { metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' } } };
    window.nameTest = { previews: [], starts: [], records: ['G007.7z', 'G009_正式版.zip', 'H010.7z'], picks: 0, picker: null };
  });
  await page.route('**/src/main.ts*', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: `
import {mockIPC,mockWindows} from '/node_modules/@tauri-apps/api/mocks.js';
mockWindows('main');
mockIPC(async(cmd,args)=>{
 const t=window.nameTest;
 if(cmd==='take_shell_requests')return [];
 if(cmd==='pick_files')return ['C:/fixture/a.txt','C:/fixture/b.txt'];
 if(cmd==='pick_directory'){t.picks++;return t.picker;}
 if(cmd==='preview_pack_names'){
   t.previews.push(structuredClone(args));
   const o=args.options;
   if(o.name_mode!=='parts')return Array.from({length:args.count},(_,i)=>i?o.name_start.replace(/\\d+$/,String(Number(o.name_start.match(/\\d+$/)?.[0]||0)+i*o.name_step)):o.name_start);
   const prefix=o.prefix_enabled?o.name_prefix.trim():'';
   const suffix=o.suffix_enabled?o.name_suffix.trim():'';
   const floor=Number(o.number_start);
   const matches=t.records.map(name=>name.replace(/\\.(7z|zip|rar)(\\.\\d+)?$/i,'')).filter(stem=>stem.startsWith(prefix)&&stem.endsWith(suffix)).map(stem=>stem.slice(prefix.length,stem.length-suffix.length||undefined)).filter(mid=>/^\\d+$/.test(mid)).map(Number);
   const first=o.auto_number&&matches.length?Math.max(floor,Math.max(...matches)+o.name_step):floor;
   return Array.from({length:args.count},(_,i)=>prefix+String(first+i*o.name_step).padStart(o.number_start.length,'0')+suffix);
 }
 if(cmd==='start_pack'){t.starts.push(structuredClone(args));return;}
 if(cmd==='get_shell_integration'||cmd==='plugin:window|is_maximized')return false;
 return null;
}, {shouldMockEvents:true});
` + await response.text() });
  });
  await page.goto(server.resolvedUrls.local[0]);
  await page.getByRole('button', { name: '🗜️ 打包压缩', exact: true }).click();
  assert.equal(await page.getByText('命名方式', { exact: true }).count(), 0);
  assert.equal(await page.getByText('旧起始包名', { exact: true }).count(), 0);
  await page.getByText('下一包预览：G008.7z', { exact: false }).waitFor();
  await page.getByRole('button', { name: '添加文件', exact: true }).click();
  await page.getByText('G008.7z', { exact: true }).waitFor();
  await page.getByText('G009.7z', { exact: true }).waitFor();

  await page.getByPlaceholder('例如 _正式版、-备份').fill('_正式版');
  await page.getByRole('button', { name: '添加后缀' }).click();
  await page.getByText('下一包预览：G010_正式版.7z', { exact: false }).waitFor();
  assert.deepEqual(await page.evaluate(() => JSON.parse(localStorage.getItem('pack_name_affixes_v1')).suffixes), ['_正式版']);

  await page.getByLabel('启用前缀').uncheck();
  await page.getByText('下一包预览：001_正式版.7z', { exact: false }).waitFor();
  await page.getByLabel('启用前缀').check();
  await page.getByText('下一包预览：G010_正式版.7z', { exact: false }).waitFor();
  await page.getByLabel('包名步长').fill('2');
  await page.getByText('下一包预览：G011_正式版.7z', { exact: false }).waitFor();
  await page.getByLabel('自动续号').uncheck();
  await page.getByText('下一包预览：G001_正式版.7z', { exact: false }).waitFor();

  await page.getByPlaceholder('例如 G、HG、合集_').fill('NEW_');
  await page.getByRole('button', { name: '添加前缀' }).click();
  await page.getByText('下一包预览：NEW_001_正式版.7z', { exact: false }).waitFor();
  assert.ok((await page.evaluate(() => JSON.parse(localStorage.getItem('pack_name_affixes_v1')).prefixes)).includes('NEW_'));
  await page.getByRole('combobox', { name: '选择包名前缀' }).click();
  await page.getByRole('option', { name: 'G', exact: true }).click();
  await page.getByText('下一包预览：G001_正式版.7z', { exact: false }).waitFor();
  await page.getByRole('combobox', { name: '选择包名前缀' }).click();
  await page.getByRole('option', { name: 'NEW_', exact: true }).click();
  await page.getByText('下一包预览：NEW_001_正式版.7z', { exact: false }).waitFor();
  await page.getByPlaceholder('例如 _正式版、-备份').fill('_终版');
  await page.getByRole('button', { name: '添加后缀' }).click();
  await page.getByText('下一包预览：NEW_001_终版.7z', { exact: false }).waitFor();
  await page.getByRole('combobox', { name: '选择包名后缀' }).click();
  await page.getByRole('option', { name: '_正式版', exact: true }).click();
  await page.getByText('下一包预览：NEW_001_正式版.7z', { exact: false }).waitFor();
  await page.getByLabel('启用后缀').uncheck();
  await page.getByText('下一包预览：NEW_001.7z', { exact: false }).waitFor();
  await page.getByLabel('启用后缀').check();
  await page.getByText('下一包预览：NEW_001_正式版.7z', { exact: false }).waitFor();

  await page.getByRole('button', { name: '开始打包', exact: true }).click();
  await page.waitForFunction(() => nameTest.picks === 1);
  assert.equal(await page.evaluate(() => nameTest.starts.length), 0);
  await page.getByText('[提示] 请选择打包输出目录', { exact: true }).waitFor();
  await page.evaluate(() => { nameTest.picker = 'C:/out'; });
  await page.getByRole('button', { name: '开始打包', exact: true }).click();
  await page.waitForFunction(() => window.nameTest.starts.length === 1);
  const sent = await page.evaluate(() => window.nameTest.starts[0]);
  assert.equal(sent.options.output_dir, 'C:/out');
  assert.equal(sent.options.name_mode, 'parts');
  assert.equal(Object.hasOwn(sent.options, 'name_start'), false);
  assert.equal(sent.options.name_prefix, 'NEW_');
  assert.equal(sent.options.name_suffix, '_正式版');
  assert.equal(sent.options.prefix_enabled, true);
  assert.equal(sent.options.suffix_enabled, true);
  assert.equal(sent.options.name_step, 2);
  assert.equal(sent.options.auto_number, false);
  await page.evaluate(() => localStorage.setItem('pack_options', JSON.stringify({ name_mode: 'legacy', name_start: 'H113', name_step: 2, output_dir: 'C:/out' })));
  await page.reload();
  await page.getByRole('button', { name: '🗜️ 打包压缩', exact: true }).click();
  await page.getByText('下一包预览：H113.7z', { exact: false }).waitFor();
  const migrated = await page.evaluate(() => nameTest.previews.at(-1).options);
  assert.equal(migrated.name_mode, 'parts');
  assert.equal(migrated.name_prefix, 'H');
  assert.equal(migrated.number_start, '113');
  assert.equal(Object.hasOwn(migrated, 'name_start'), false);
  assert.deepEqual(errors, []);
  console.log('pack naming UI: unified parts controls, prefix/suffix preview, legacy setting migration and submit passed');
} finally {
  await browser.close();
  await server.close();
}
