// Run with node app/tests/tool-path-settings.mjs. PLAYWRIGHT_MODULE may point to
// an existing Playwright ESM installation; the test owns its Vite/browser only.
import { createServer } from 'vite';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import assert from 'node:assert/strict';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const server = await createServer({ root, configFile: path.join(root, 'vite.config.ts'), server: { host: '127.0.0.1', port: 0, strictPort: false } });
await server.listen();
const browser = await chromium.launch({ channel: 'msedge', headless: true });
const page = await browser.newPage();
const errors = [], checks = [];
page.on('pageerror', error => errors.push(error.message));
try {
  await page.addInitScript(() => {
    window.__TAURI_INTERNALS__ = { metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' } } };
    if (!localStorage.getItem('mock_config')) localStorage.setItem('mock_config', JSON.stringify({ source_dir: 'SRC', output_dir: 'OUT', failed_dir: 'FAILED', seven_zip: 'old7z.exe', unrar: 'oldrar.exe', lz4: '', passwords: [], password_rules: [], password_strategy: 'recent_first', recursive: true, product_exts: ['.apk'], max_depth: 5 }));
    localStorage.setItem('pack_options', JSON.stringify({ output_dir: 'PACKOUT' }));
    window.testTools = { calls: [], starts: [], hold: false, reject: false, failConfig: false, picked: null };
  });
  await page.route('**/src/main.ts*', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: `
import {mockIPC,mockWindows} from '/node_modules/@tauri-apps/api/mocks.js';
mockWindows('main');
mockIPC(async(cmd,args)=>{
 const t=window.testTools; t.calls.push({cmd,args});
 const read=()=>JSON.parse(localStorage.getItem('mock_config'));
 if(cmd==='load_config')return read();
 if(cmd==='save_tool_paths'){
   if(t.hold)await new Promise(resolve=>t.release=resolve);
   if(t.reject)throw new Error('工具路径保存失败');
   localStorage.setItem('mock_config',JSON.stringify({...read(),...args.paths}));return args.paths;
 }
 if(cmd==='save_config'){
   if(t.failConfig)return false;
   const current=read();localStorage.setItem('mock_config',JSON.stringify({...args.cfg,seven_zip:current.seven_zip,unrar:current.unrar,lz4:current.lz4}));return true;
 }
 if(cmd==='start_pack'||cmd==='start_run'){const cfg=read();t.starts.push({cmd,tools:[cfg.seven_zip,cfg.unrar,cfg.lz4]});return;}
 if(cmd==='pick_file')return t.picked;
 if(cmd==='pick_files')return ['C:/fixture/a.txt'];
 if(cmd==='take_shell_requests')return [];
 if(cmd==='get_shell_integration'||cmd==='plugin:window|is_maximized')return false;
}, {shouldMockEvents:true});
window.fakeDone=async kind=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');await emit(kind+'-done',{ok:[],failed:[],skipped:[],warns:[]});};
` + await response.text() });
  });
  await page.goto(server.resolvedUrls.local[0]);
  const openSettings = async () => {
    await page.getByRole('button', { name: '应用设置', exact: true }).click();
    await page.getByLabel('7-Zip 路径', { exact: true }).waitFor();
    await page.waitForFunction(() => !document.querySelector('#tool-path-seven_zip').disabled && !document.querySelector('.tool-fields').disabled);
  };
  const closeSettings = () => page.locator('.close-btn').click();
  const save = () => page.getByRole('button', { name: '保存工具路径', exact: true }).click();
  const latest = () => page.evaluate(() => testTools.starts.at(-1));
  const done = kind => page.evaluate(kind => fakeDone(kind), kind);
  assert.equal(await page.getByLabel('7-Zip 路径', { exact: true }).count(), 0);
  await openSettings();
  assert.equal(await page.getByLabel('7-Zip 路径', { exact: true }).inputValue(), 'old7z.exe');
  await page.getByLabel('7-Zip 路径', { exact: true }).fill('new7z.exe');
  await page.getByLabel('UnRAR 路径', { exact: true }).fill('newrar.exe');
  await page.getByLabel('lz4.exe 回退路径', { exact: true }).fill('newlz4.exe');
  await save(); await page.getByText('已保存，下一次压缩或解压生效，无需重启。', { exact: true }).waitFor();
  await closeSettings();
  await page.getByRole('button', { name: '🗜️ 打包压缩', exact: true }).click();
  await page.getByRole('button', { name: '添加文件', exact: true }).click();
  await page.getByRole('button', { name: '开始打包', exact: true }).click();
  await page.waitForFunction(() => testTools.starts.length === 1);
  assert.deepEqual((await latest()).tools, ['new7z.exe', 'newrar.exe', 'newlz4.exe']); await done('pack');
  await page.getByRole('button', { name: '📦 自动解压', exact: true }).click();
  await page.getByRole('button', { name: '开始解压', exact: true }).click();
  await page.waitForFunction(() => testTools.starts.length === 2);
  assert.deepEqual((await latest()).tools, ['new7z.exe', 'newrar.exe', 'newlz4.exe']); await done('uz');
  checks.push('global path editor; explicit save shared immediately by pack and stale unpack page');

  await openSettings();
  assert.equal(await page.getByLabel('7-Zip 路径', { exact: true }).inputValue(), 'new7z.exe');
  await page.evaluate(() => testTools.reject = true);
  await page.getByLabel('7-Zip 路径', { exact: true }).fill('bad.exe'); await save();
  await page.getByRole('alert').filter({ hasText: '保存失败' }).waitFor();
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('mock_config')).seven_zip), 'new7z.exe');
  await closeSettings();
  await page.evaluate(() => testTools.reject = false);
  await openSettings();
  assert.equal(await page.getByLabel('7-Zip 路径', { exact: true }).inputValue(), 'new7z.exe');
  await page.evaluate(() => testTools.picked = 'picked.exe');
  await page.getByRole('button', { name: '浏览7-Zip 路径', exact: true }).click();
  assert.equal(await page.getByLabel('7-Zip 路径', { exact: true }).inputValue(), 'picked.exe');
  await page.getByLabel('lz4.exe 回退路径', { exact: true }).fill('');
  await page.evaluate(() => testTools.hold = true);
  await save(); await page.waitForFunction(() => !!testTools.release);
  await closeSettings();
  await page.getByRole('button', { name: '🗜️ 打包压缩', exact: true }).click();
  await page.getByRole('button', { name: '开始打包', exact: true }).click();
  assert.equal(await page.getByRole('button', { name: '开始打包', exact: true }).isDisabled(), true);
  assert.equal(await page.evaluate(() => testTools.starts.length), 2);
  await page.evaluate(() => { testTools.hold = false; testTools.release(); });
  await page.waitForFunction(() => testTools.starts.length === 3);
  assert.deepEqual((await latest()).tools, ['picked.exe', 'newrar.exe', '']); await done('pack');
  checks.push('save error visible without changing saved paths; browse and clear; pack waits for in-flight settings save');

  await openSettings();
  await page.getByLabel('7-Zip 路径', { exact: true }).fill('failed.exe');
  await page.evaluate(() => {testTools.hold = true; testTools.reject = true; testTools.release = null;});
  await save(); await page.waitForFunction(() => !!testTools.release);
  await closeSettings(); await page.getByRole('button', { name: '📦 自动解压', exact: true }).click();
  await page.getByRole('button', { name: '开始解压', exact: true }).click();
  assert.equal(await page.getByRole('button', { name: '开始解压', exact: true }).isDisabled(), true);
  await page.evaluate(() => {testTools.hold = false; testTools.release();});
  await page.getByText('[失败] Error: 工具路径保存失败', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => testTools.starts.length), 3);
  assert.equal(await page.getByRole('button', { name: '开始解压', exact: true }).isDisabled(), false);
  await page.evaluate(() => {testTools.reject = false; testTools.failConfig = true;});
  await page.getByRole('button', { name: '开始解压', exact: true }).click();
  await page.getByText('[失败] Error: 解压设置保存失败，请检查配置目录写入权限或磁盘空间', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => testTools.starts.length), 3);
  checks.push('unpack waits for settings save, cancels on its failure, and honors false save_config result');
  await page.reload(); await openSettings();
  assert.equal(await page.getByLabel('7-Zip 路径', { exact: true }).inputValue(), 'picked.exe');
  assert.equal(await page.getByLabel('lz4.exe 回退路径', { exact: true }).inputValue(), '');
  checks.push('saved shared paths reload persistently');
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ checks, errors }, null, 2));
} catch(error) { console.error(JSON.stringify({ errors, body: (await page.locator('body').innerText()).slice(0,3000) })); throw error; }
finally { await browser.close(); await server.close(); }
