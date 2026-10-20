// Explorer cold/hot startup, queue coalescing, busy retry, and unchanged GUI settings.
import { createServer } from 'vite';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
import assert from 'node:assert/strict';
const modulePath = process.env.PLAYWRIGHT_MODULE;
const { chromium } = await import(modulePath && path.isAbsolute(modulePath) ? pathToFileURL(modulePath).href : modulePath || 'playwright');
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
    localStorage.setItem('unpack_source_mode_v1', 'directory');
    window.shellTest = {
      cfg: { source_dir: '', output_dir: '', failed_dir: 'C:/failed', failed_dir_enabled: false, seven_zip: '', unrar: '', lz4: '', passwords: ['saved-password'], password_rules: [{ suffix: '.zip', keyword: '', enabled: true, passwords: ['priority'] }], password_strategy: 'list_order', hash_match: true, recursive: true, product_exts: ['.apk'], max_depth: 5 },
      requests: location.search.includes('quick=1') ? [{ action: 'pack-same-name', paths: ['C:/cold/资料'] }] : [{ action: 'extract-here', paths: ['C:/中文 空格/one.zip'] }, { action: 'extract-here', paths: ['C:/中文 空格/one.zip', 'C:/中文 空格/two.zip'] }],
      calls: [], accepted: [], saves: [], busy: false, picks: 0, cancels: 0, normalStarts: 0, retries: [],
      integrationChanges: [],
      quickCalls: [], quickAccepted: [], formatChanges: [], formatError: false,
      finishes: [],
      displayChanges: [], displayError: false,
    };
  });
  await page.route('**/src/main.ts*', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: `
import {mockIPC,mockWindows} from '/node_modules/@tauri-apps/api/mocks.js';
mockWindows('main');
mockIPC(async(cmd,args)=>{
 const t=window.shellTest;
 if(cmd==='finish_shell_requests'){t.finishes.push(structuredClone(args));return;}
 if(cmd==='get_shell_display')return JSON.parse(localStorage.getItem('shell_display')||'{"showProgress":true,"showConsole":false}');
 if(cmd==='set_shell_display'){if(t.displayError)throw new Error('模拟显示设置保存失败');t.displayChanges.push(structuredClone(args));localStorage.setItem('shell_display',JSON.stringify(args));return;}
 if(cmd==='load_config') return structuredClone(t.cfg);
 if(cmd==='save_config'){t.saves.push(args.cfg);return true;}
 if(cmd==='take_shell_requests') return t.requests.splice(0);
 if(cmd==='start_shell_run'){t.calls.push(structuredClone(args));if(t.busy)return false;t.accepted.push(structuredClone(args));return true;}
 if(cmd==='start_shell_pack'){t.quickCalls.push(structuredClone(args));if(t.busy)return false;t.quickAccepted.push({...structuredClone(args),format:localStorage.getItem('shell_quick_format')||'7z'});return true;}
 if(cmd==='get_quick_pack_format')return localStorage.getItem('shell_quick_format')||'7z';
 if(cmd==='set_quick_pack_format'){if(t.formatError)throw new Error('模拟保存失败');t.formatChanges.push(args.format);localStorage.setItem('shell_quick_format',args.format);return args.format;}
 if(cmd==='start_run'){t.normalStarts++;return;}
 if(cmd==='retry_unpack_password'){t.retries.push(structuredClone(args));return;}
 if(cmd==='cancel_run'){t.cancels++;return;}
 if(cmd==='set_shell_integration'){t.integrationChanges.push(args.enabled);return args.enabled;}
 if(cmd==='pick_directory'){t.picks++;return null;}
 if(cmd==='get_shell_integration'||cmd==='plugin:window|is_maximized')return false;
 return null;
},{shouldMockEvents:true});
window.emitShell=async(requests)=>{window.shellTest.requests.push(...requests);const {emit}=await import('/node_modules/@tauri-apps/api/event.js');await emit('shell-request-pending');};
window.shellDone=async(result)=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');await emit('uz-done',result||{ok:[['one','']],failed:[],skipped:[],warns:[],retryable:[]});};
window.quickDone=async()=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');const format=window.shellTest.quickAccepted.at(-1).format;await emit('pack-done',{ok:[{name:'资料',password:null,size_bytes:10,outputs:['C:/资料.'+format]}],failed:[],warns:[]});};
` + await response.text() });
  });
  await page.goto(server.resolvedUrls.local[0]);
  await page.waitForFunction(() => shellTest.accepted.length === 1);
  assert.deepEqual(await page.evaluate(() => shellTest.accepted[0]), { action: 'extract-here', paths: ['C:/中文 空格/one.zip', 'C:/中文 空格/two.zip'] });
  assert.equal(await page.getByRole('radio', { name: '仅扫描源目录' }).isChecked(), true);
  assert.equal(await page.getByRole('button', { name: '停止', exact: true }).isEnabled(), true);
  assert.equal(await page.evaluate(() => shellTest.picks), 0);
  assert.equal(await page.evaluate(() => shellTest.saves.length), 0);
  assert.equal(await page.evaluate(() => shellTest.normalStarts), 0);
  await page.getByRole('status', { name: '解压任务状态' }).getByText(/解压任务进行中/).waitFor();
  await page.getByRole('status', { name: '解压任务状态' }).getByText(/正在准备右键解压/).waitFor();
  await page.waitForFunction(() => document.querySelector('[aria-label="解压任务状态"]').textContent.includes('已用 0分1秒'));
  await page.waitForTimeout(750);
  assert.equal(await page.evaluate(() => shellTest.finishes.filter(x => !x.reveal).length), 0);

  // A hot request stays queued while the first task is running.
  await page.evaluate(() => emitShell([{ action: 'extract-folder', paths: ['D:/hot/new.7z'] }]));
  await page.getByText(/右键解压等待中/).waitFor();
  assert.equal(await page.evaluate(() => shellTest.accepted.length), 1);
  await page.evaluate(() => shellDone());
  await page.waitForFunction(() => shellTest.accepted.length === 2);
  assert.deepEqual(await page.evaluate(() => shellTest.accepted[1]), { action: 'extract-folder', paths: ['D:/hot/new.7z'] });
  await page.getByRole('button', { name: /打包压缩/ }).click();
  assert.equal(await page.getByRole('button', { name: '开始打包', exact: true }).isDisabled(), true, 'active extraction must block manual packing');
  await page.evaluate(() => shellDone());

  // Switch away and deliver another request; KeepAlive must preserve listeners.
  await page.getByRole('button', { name: /打包压缩/ }).click();
  await page.evaluate(() => { shellTest.busy = true; return emitShell([{ action: 'extract-here', paths: ['C:/busy.zip'] }]); });
  await page.waitForFunction(() => shellTest.calls.length >= 3);
  assert.equal(await page.evaluate(() => shellTest.accepted.length), 2);
  await page.getByText(/右键解压等待中/).waitFor();
  await page.evaluate(() => { shellTest.busy = false; });
  await page.waitForFunction(() => shellTest.accepted.length === 3);
  assert.equal((await page.evaluate(() => shellTest.accepted[2])).action, 'extract-here');
  await page.evaluate(() => shellDone({ ok: [], failed: [['busy.zip', '密码不匹配']], skipped: [], warns: [], retryable: [{ slot: 0, failed_index: 0, name: 'busy.zip' }] }));
  await page.waitForFunction(() => shellTest.finishes.some(x => x.reveal));
  await page.getByRole('button', { name: '添加新密码并重试' }).click();
  await page.getByRole('dialog', { name: '添加新密码并重试' }).getByRole('textbox', { name: '新密码' }).fill('new-secret');
  await page.getByRole('dialog', { name: '添加新密码并重试' }).getByRole('button', { name: '重试', exact: true }).click();
  await page.waitForFunction(() => shellTest.retries.length === 1);
  assert.deepEqual(await page.evaluate(() => shellTest.retries[0]), { slot: 0, password: 'new-secret', outputDir: '' });
  await page.evaluate(() => shellDone());

  // Legacy queue entries still navigate without auto-extracting.
  await page.evaluate(() => emitShell([{ action: 'unpack', paths: ['C:/queued.zip'] }]));
  await page.getByText('C:/queued.zip', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => shellTest.accepted.length), 3);
  await page.evaluate(() => emitShell([{ action: 'pack', paths: ['C:/to-pack.txt'] }]));
  await page.getByText('C:/to-pack.txt', { exact: true }).waitFor();

  // Stop clears future direct requests instead of restarting after completion.
  await page.evaluate(() => emitShell([{ action: 'extract-here', paths: ['C:/stop.zip'] }]));
  await page.waitForFunction(() => shellTest.accepted.length === 4);
  await page.evaluate(() => emitShell([{ action: 'extract-folder', paths: ['C:/cancel-pending.zip'] }]));
  await page.getByText(/右键解压等待中/).waitFor();
  await page.getByRole('button', { name: '停止', exact: true }).click();
  await page.waitForFunction(() => shellTest.cancels === 1);
  await page.evaluate(() => shellDone());
  await page.getByText(/右键解压等待中/).waitFor({ state: 'hidden' });
  assert.equal(await page.evaluate(() => shellTest.saves.length), 0);
  assert.equal(await page.getByLabel('解压输出目录', { exact: true }).inputValue(), '');
  const packPreferences = await page.evaluate(() => Object.fromEntries(Object.entries(localStorage).filter(([key]) => key.startsWith('pack_'))));
  await page.evaluate(() => emitShell([{action:'pack-same-name',paths:['C:/quick/资料']},{action:'pack-same-name',paths:['C:/quick/资料','D:/notes.txt']}]));
  await page.waitForFunction(() => shellTest.quickAccepted.length === 1);
  assert.deepEqual(await page.evaluate(() => shellTest.quickAccepted[0]), {paths:['C:/quick/资料','D:/notes.txt'],format:'7z'});
  assert.equal(await page.evaluate(() => shellTest.picks), 0);
  assert.equal(await page.getByLabel('包名步长').isDisabled(), true, 'running pack options must be frozen');
  await page.getByRole('button', { name: /自动解压/ }).click();
  assert.equal(await page.getByRole('button', { name: '开始解压', exact: true }).isDisabled(), true, 'active packing must block manual extraction');
  await page.getByRole('button', { name: /打包压缩/ }).click();
  await page.evaluate(() => emitShell([{action:'pack-same-name',paths:['C:/next']}]));
  await page.getByText(/右键快捷压缩等待中/).waitFor();
  assert.equal(await page.evaluate(() => shellTest.quickAccepted.length), 1);
  await page.evaluate(() => quickDone());
  await page.waitForFunction(() => shellTest.quickAccepted.length === 2);
  await page.evaluate(() => quickDone());
  await page.evaluate(() => { shellTest.busy=true; return emitShell([{action:'pack-same-name',paths:['C:/busy-pack']}]); });
  await page.waitForFunction(() => shellTest.quickCalls.length >= 3);
  assert.equal(await page.evaluate(() => shellTest.quickAccepted.length), 2);
  await page.evaluate(() => { shellTest.busy=false; });
  await page.waitForFunction(() => shellTest.quickAccepted.length === 3);
  await page.evaluate(() => emitShell([{action:'pack-same-name',paths:['C:/cancel-next-pack']}]));
  await page.getByText(/右键快捷压缩等待中/).waitFor();
  await page.getByRole('button',{name:'停止',exact:true}).click();
  await page.evaluate(() => quickDone());
  await page.getByText(/右键快捷压缩等待中/).waitFor({state:'hidden'});
  assert.deepEqual(await page.evaluate(() => Object.fromEntries(Object.entries(localStorage).filter(([key]) => key.startsWith('pack_')))), packPreferences);
  await page.getByRole('button', { name: '应用设置', exact: true }).click();
  await page.getByRole('navigation', { name: '设置导航' }).getByRole('button', { name: '系统集成' }).click();
  await page.getByLabel('添加 unzip-rs 右键子菜单').check();
  await page.getByLabel('添加 unzip-rs 右键子菜单').uncheck();
  assert.deepEqual(await page.evaluate(() => shellTest.integrationChanges), [true, false]);
  const format = page.getByRole('combobox',{name:'快捷压缩默认格式'});
  const progressToggle = page.getByLabel('右键任务显示进度与日志窗口');
  const consoleToggle = page.getByLabel('右键任务显示终端日志');
  assert.equal(await progressToggle.isChecked(), true);
  assert.equal(await consoleToggle.isChecked(), false);
  await progressToggle.uncheck();
  await consoleToggle.check();
  await page.waitForFunction(() => shellTest.displayChanges.length === 2);
  assert.deepEqual(await page.evaluate(() => shellTest.displayChanges), [{showProgress:false,showConsole:false},{showProgress:false,showConsole:true}]);
  await page.evaluate(() => { shellTest.displayError = true; });
  await consoleToggle.click();
  await page.getByRole('alert').filter({hasText:'模拟显示设置保存失败'}).waitFor();
  assert.equal(await consoleToggle.isChecked(), true);
  await format.click();
  await page.getByRole('option',{name:'zip（兼容性好）',exact:true}).click();
  await page.waitForFunction(() => shellTest.formatChanges.length === 1);
  assert.deepEqual(await page.evaluate(() => shellTest.formatChanges), ['zip']);
  await page.evaluate(() => {shellTest.formatError=true;});
  await format.click();
  await page.getByRole('option',{name:'7z（默认）',exact:true}).click();
  await page.getByRole('alert').filter({hasText:'模拟保存失败'}).waitFor();
  await format.getByText('zip（兼容性好）',{exact:true}).waitFor();
  await page.goto(`${server.resolvedUrls.local[0]}?quick=1`);
  await page.waitForFunction(() => shellTest.quickAccepted.length === 1);
  assert.deepEqual(await page.evaluate(() => shellTest.quickAccepted[0]), {paths:['C:/cold/资料'],format:'zip'});
  assert.equal(await page.evaluate(() => shellTest.accepted.length),0);
  await page.evaluate(() => quickDone());
  await page.getByRole('cell', { name: '资料.zip', exact: true }).waitFor();
  await page.waitForFunction(() => shellTest.finishes.some(x => !x.reveal));
  assert.equal(await page.evaluate(() => shellTest.finishes.some(x => x.reveal)), false);
  await page.getByRole('button', { name: '应用设置', exact: true }).click();
  await page.getByRole('navigation', { name: '设置导航' }).getByRole('button', { name: '系统集成' }).click();
  await page.getByRole('combobox',{name:'快捷压缩默认格式'}).getByText('zip（兼容性好）',{exact:true}).waitFor();
  assert.equal(await page.getByLabel('右键任务显示进度与日志窗口').isChecked(), false);
  assert.equal(await page.getByLabel('右键任务显示终端日志').isChecked(), true);
  if (process.env.UI_SCREENSHOT) {
    await page.getByLabel('右键任务显示进度与日志窗口').scrollIntoViewIfNeeded();
    await page.screenshot({ path: process.env.UI_SCREENSHOT });
  }
  assert.deepEqual(errors, []);
  console.log('Explorer UI passed: extraction and quick pack cold/hot startup, coalescing, busy queue, stop, activity timer, display defaults/persistence/failure rollback, format persistence, unchanged publishing settings');
} finally {
  await browser.close();
  await server.close();
}
