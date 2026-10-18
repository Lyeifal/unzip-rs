// Browser regression for unpack source modes and settings dialogs.
// PLAYWRIGHT_MODULE may point to an existing Playwright ESM install.
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
    if (!localStorage.getItem('unpack_mock_config')) localStorage.setItem('unpack_mock_config', JSON.stringify({
      source_dir: '', output_dir: '', failed_dir: 'C:/failed', seven_zip: '', unrar: '', lz4: '',
      passwords: ['old'], password_rules: [], password_strategy: 'recent_first', hash_match: true,
      recursive: true, product_exts: ['.apk'], max_depth: 5,
    }));
    window.unpackTest = { starts: [], saves: [], retries: [], picker: null, picks: 0 };
  });
  await page.route('**/src/main.ts*', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: `
import {mockIPC,mockWindows} from '/node_modules/@tauri-apps/api/mocks.js';
mockWindows('main');
mockIPC(async(cmd,args)=>{
 const t=window.unpackTest;
 const read=()=>JSON.parse(localStorage.getItem('unpack_mock_config'));
 if(cmd==='load_config')return read();
 if(cmd==='save_config'){t.saves.push(JSON.parse(JSON.stringify(args.cfg)));localStorage.setItem('unpack_mock_config',JSON.stringify(args.cfg));return true;}
 if(cmd==='start_run'){t.starts.push(JSON.parse(JSON.stringify(args)));return;}
 if(cmd==='retry_unpack_password'){t.retries.push(JSON.parse(JSON.stringify(args)));return;}
 if(cmd==='pick_files')return ['C:/fixture/selected.zip', 'C:/fixture/second.zip'];
 if(cmd==='pick_directories_multi')return ['C:/fixture/selected-folder'];
 if(cmd==='pick_directory'){t.picks++;return t.picker;}
 if(cmd==='take_shell_requests')return [];
 if(cmd==='get_shell_integration'||cmd==='plugin:window|is_maximized')return false;
 return null;
}, {shouldMockEvents:true});
window.finishUnpack=async()=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');await emit('uz-done',{ok:[],failed:[],skipped:[],warns:[]});};
window.failWithPassword=async()=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');await emit('uz-done',{ok:[],failed:[['secret.7z','密码不匹配'],['second.7z','密码不匹配']],skipped:[],warns:[],retryable:[{slot:0,failed_index:0,name:'secret.7z'},{slot:1,failed_index:1,name:'second.7z'}]});};
window.finishRetry=async()=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');await emit('uz-retry-done',{slot:0,failedIndex:0,result:{ok:[['secret.7z','密码：new-secret']],failed:[],skipped:[],warns:[],retryable:[]}});};
window.finishSecondRetry=async()=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');await emit('uz-retry-done',{slot:1,failedIndex:1,result:{ok:[['second.7z','密码：second-secret']],failed:[],skipped:[],warns:[],retryable:[]}});};
` + await response.text() });
  });
  await page.goto(server.resolvedUrls.local[0]);
  await page.getByRole('radiogroup', { name: '解压来源方式' }).waitFor();
  await page.waitForFunction(() => [...document.querySelectorAll('input')].some(input => input.value === 'C:/failed'));
  await page.getByRole('radio', { name: '仅选中文件' }).check();
  await page.getByRole('button', { name: '添加文件', exact: true }).click();
  await page.getByText('C:/fixture/selected.zip').waitFor();
  assert.equal(await page.getByRole('radio', { name: '仅选中文件' }).isChecked(), true);
  const start = () => page.getByRole('button', { name: '开始解压', exact: true }).click();
  const finish = () => page.evaluate(() => finishUnpack());
  await start();
  await page.waitForFunction(() => unpackTest.picks === 1);
  assert.equal(await page.evaluate(() => unpackTest.starts.length), 0);
  await page.getByText('[提示] 请选择解压输出目录，用于保存解压后的文件', { exact: true }).waitFor();
  await page.evaluate(() => { unpackTest.picker = 'C:/out'; });
  await start();
  await page.waitForFunction(() => unpackTest.starts.length === 1);
  assert.deepEqual((await page.evaluate(() => unpackTest.starts[0])).files, ['C:/fixture/selected.zip', 'C:/fixture/second.zip']);
  assert.equal((await page.evaluate(() => unpackTest.starts[0])).sourceDir, '');
  assert.equal((await page.evaluate(() => unpackTest.starts[0])).outputDir, 'C:/out');
  assert.equal((await page.evaluate(() => unpackTest.starts[0])).sourceMode, 'selected');
  await finish();

  await page.getByRole('radio', { name: '仅扫描源目录' }).check();
  await page.getByLabel('压缩包所在目录', { exact: true }).fill('C:/scan');
  await start();
  await page.waitForFunction(() => unpackTest.starts.length === 2);
  assert.equal((await page.evaluate(() => unpackTest.starts[1])).files, null);
  assert.equal((await page.evaluate(() => unpackTest.starts[1])).sourceMode, 'directory');
  await finish();

  await page.getByRole('radio', { name: '两者一起' }).check();
  await page.getByRole('button', { name: '添加文件夹' }).click();
  await start();
  await page.waitForFunction(() => unpackTest.starts.length === 3);
  assert.deepEqual((await page.evaluate(() => unpackTest.starts[2])).files, ['C:/fixture/selected.zip', 'C:/fixture/second.zip', 'C:/fixture/selected-folder']);
  assert.equal((await page.evaluate(() => unpackTest.starts[2])).sourceMode, 'both');
  await finish();

  await page.getByRole('button', { name: /^密码库/ }).click();
  await page.getByRole('dialog', { name: '密码库配置' }).waitFor();
  await page.getByRole('combobox', { name: '密码排序规则' }).click();
  await page.getByRole('option', { name: '严格按列表顺序' }).click();
  await page.getByRole('dialog', { name: '密码库配置' }).getByRole('button', { name: '添加' }).click();
  await page.locator('.overlay').last().locator('input').fill('new-password');
  await page.getByRole('button', { name: '确定' }).click();
  await page.getByRole('dialog', { name: '密码库配置' }).getByRole('button', { name: '保存并关闭' }).click();
  await page.getByRole('dialog', { name: '密码库配置' }).waitFor({ state: 'hidden' });
  let saved = await page.evaluate(() => JSON.parse(localStorage.getItem('unpack_mock_config')));
  assert.equal(saved.password_strategy, 'list_order');
  assert.deepEqual(saved.passwords, ['old', 'new-password']);

  await page.getByRole('button', { name: /^密码策略规则/ }).click();
  await page.getByRole('dialog', { name: '密码策略规则配置' }).getByLabel('按 SHA-256 匹配记录密码').uncheck();
  await page.getByRole('dialog', { name: '密码策略规则配置' }).getByRole('button', { name: '添加' }).click();
  await page.getByPlaceholder('.lz4（后缀）').fill('.zip');
  await page.getByPlaceholder('多个密码用逗号或空格分隔，按顺序优先尝试').fill('priority');
  await page.getByRole('button', { name: '确定' }).click();
  await page.getByRole('button', { name: '停用规则 1' }).click();
  await page.getByRole('dialog', { name: '密码策略规则配置' }).getByRole('button', { name: '保存并关闭' }).click();
  saved = await page.evaluate(() => JSON.parse(localStorage.getItem('unpack_mock_config')));
  assert.equal(saved.hash_match, false);
  assert.equal(saved.password_rules[0].enabled, false);
  assert.deepEqual(saved.password_rules[0].passwords, ['priority']);

  await page.getByLabel('启用解压失败目录').uncheck();
  assert.equal(await page.getByLabel('解压失败文件所在目录', { exact: true }).inputValue(), 'C:/failed');
  await page.getByRole('button', { name: '解压选项', exact: true }).click();
  await page.getByRole('dialog', { name: '解压选项配置' }).getByLabel('递归扫描源目录').uncheck();
  await page.getByLabel('产物后缀').fill('.apk .iso');
  await page.getByRole('dialog', { name: '解压选项配置' }).getByRole('button', { name: '保存并关闭' }).click();
  saved = await page.evaluate(() => JSON.parse(localStorage.getItem('unpack_mock_config')));
  assert.equal(saved.recursive, false);
  assert.equal(saved.failed_dir_enabled, false);
  assert.equal(await page.getByPlaceholder('失败的压缩包连同分卷移入此处').isDisabled(), true);
  assert.deepEqual(saved.product_exts, ['.apk', '.iso']);
  await page.getByLabel('启用解压失败目录').check();
  assert.equal(await page.getByLabel('解压失败文件所在目录', { exact: true }).inputValue(), 'C:/failed');
  assert.equal(await page.getByLabel('解压失败文件所在目录', { exact: true }).isEnabled(), true);
  await page.getByLabel('启用解压失败目录').uncheck();
  await start();
  await page.waitForFunction(() => unpackTest.starts.length === 4);
  const runConfig = await page.evaluate(() => unpackTest.saves.at(-1));
  assert.equal(runConfig.failed_dir_enabled, false);
  assert.equal(runConfig.failed_dir, 'C:/failed');
  await finish();
  await page.getByRole('button', { name: /^密码策略规则/ }).click();
  await page.getByRole('button', { name: '启用规则 1' }).click();
  await page.getByRole('dialog', { name: '密码策略规则配置' }).getByRole('button', { name: '取消' }).click();
  await page.evaluate(() => failWithPassword());
  await page.getByRole('button', { name: '添加新密码并重试' }).first().click();
  await page.getByRole('dialog', { name: '添加新密码并重试' }).getByRole('textbox', { name: '新密码' }).fill('new-secret');
  await page.getByRole('dialog', { name: '添加新密码并重试' }).getByRole('button', { name: '重试' }).click();
  await page.waitForFunction(() => unpackTest.retries.length === 1);
  assert.deepEqual(await page.evaluate(() => unpackTest.retries[0]), { slot: 0, password: 'new-secret', outputDir: 'C:/out' });
  await page.evaluate(() => finishRetry());
  await page.getByRole('button', { name: '添加新密码并重试' }).click();
  await page.getByRole('dialog', { name: '添加新密码并重试' }).getByRole('textbox', { name: '新密码' }).fill('second-secret');
  await page.getByRole('dialog', { name: '添加新密码并重试' }).getByRole('button', { name: '重试' }).click();
  await page.waitForFunction(() => unpackTest.retries.length === 2);
  assert.equal(await page.evaluate(() => unpackTest.retries[1].slot), 1);
  await page.evaluate(() => finishSecondRetry());
  await page.getByRole('button', { name: '添加新密码并重试' }).waitFor({ state: 'hidden' });
  assert.equal((await page.evaluate(() => JSON.parse(localStorage.getItem('unpack_mock_config')))).password_rules[0].enabled, false);
  await page.getByRole('button', { name: /^密码策略规则/ }).click();
  await page.getByRole('button', { name: '启用规则 1' }).waitFor();
  await page.getByRole('dialog', { name: '密码策略规则配置' }).getByRole('button', { name: '取消' }).click();
  await page.getByRole('button', { name: '应用设置' }).click();
  await page.getByRole('navigation', { name: '设置导航' }).getByRole('button', { name: '界面外观' }).click();
  assert.equal(await page.locator('#settings-appearance').isVisible(), true);
  await page.waitForFunction(() => document.querySelector('.panel-content')?.scrollTop > 0);
  assert.equal(await page.locator('.app-header.has-background').count(), 0);
  await page.getByLabel('显示标题栏底色').check();
  assert.equal(await page.locator('.app-header.has-background').count(), 1);
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('unzip_wallpaper_config')).titlebarBackground), true);
  await page.locator('.close-btn').click();
  if (process.env.UI_SCREENSHOT) await page.screenshot({ path: process.env.UI_SCREENSHOT, fullPage: false });
  await page.reload();
  await page.getByRole('heading', { name: 'unzip-rs', exact: true }).waitFor();
  await page.locator('.app-header.has-background').waitFor();
  await page.getByRole('button', { name: '应用设置', exact: true }).click();
  await page.getByRole('navigation', { name: '设置导航' }).getByRole('button', { name: '界面外观' }).click();
  assert.equal(await page.getByLabel('显示标题栏底色').isChecked(), true);
  await page.getByLabel('显示标题栏底色').uncheck();
  assert.equal(await page.locator('.app-header.has-background').count(), 0);
  assert.deepEqual(errors, []);
  console.log('unpack settings UI: source modes, password/rule options, failed-directory toggle, two targeted retries, settings navigation passed');
} finally {
  await browser.close();
  await server.close();
}
