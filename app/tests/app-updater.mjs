import { createServer } from 'vite';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
import assert from 'node:assert/strict';

const modulePath = process.env.PLAYWRIGHT_MODULE;
const { chromium } = await import(modulePath && path.isAbsolute(modulePath) ? pathToFileURL(modulePath).href : modulePath || 'playwright');
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const server = await createServer({ root, configFile: path.join(root, 'vite.config.ts'), server: { host: '127.0.0.1', port: 0 } });
await server.listen();
const browser = await chromium.launch({ channel: 'msedge', headless: true });
const page = await browser.newPage();
const errors = [];
page.on('pageerror', error => errors.push(error.message));
try {
  await page.addInitScript(() => {
    window.__TAURI_INTERNALS__ = { metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' } } };
    window.updaterTest = { mode: 'latest', busy: false, installFails: false, installs: 0, releases: 0, restarts: 0, checks: 0 };
  });
  await page.route('**/src/main.ts*', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: `
import { mockIPC, mockWindows } from '/node_modules/@tauri-apps/api/mocks.js';
mockWindows('main');
mockIPC(async (cmd, args) => {
  const t = window.updaterTest;
  if (cmd === 'plugin:app|version') return '0.1.0';
  if (cmd === 'plugin:updater|check') {
    t.checks++;
    if (t.mode === 'error') throw new Error('network unavailable');
    if (t.mode === 'latest') return null;
    return { rid: t.checks, currentVersion: '0.1.0', version: '0.2.0', body: '修复解压问题', rawJson: {} };
  }
  if (cmd === 'begin_update_install') { if (t.busy) throw new Error('请先等待压缩或解压任务完成，再安装更新'); return; }
  if (cmd === 'end_update_install') { t.releases++; return; }
  if (cmd === 'plugin:updater|download_and_install') {
    t.installs++;
    if (t.installFails) throw new Error('signature verification failed');
    args.onEvent.onmessage({ event: 'Started', data: { contentLength: 100 } });
    args.onEvent.onmessage({ event: 'Progress', data: { chunkLength: 50 } });
    await new Promise(resolve => { t.finish = resolve; });
    args.onEvent.onmessage({ event: 'Finished' });
    return;
  }
  if (cmd === 'plugin:process|restart') { t.restarts++; return; }
  if (cmd === 'take_shell_requests') return [];
  if (cmd === 'load_config') return { source_dir: '', output_dir: '', failed_dir: '', seven_zip: '', unrar: '', lz4: '', passwords: [], password_rules: [], product_exts: ['.apk'] };
  if (cmd === 'get_shell_integration' || cmd === 'plugin:window|is_maximized') return false;
  return null;
}, { shouldMockEvents: true });
` + await response.text() });
  });
  await page.goto(server.resolvedUrls.local[0]);
  await page.getByRole('button', { name: '应用设置', exact: true }).click();
  const check = () => page.getByRole('button', { name: '检查更新', exact: true }).click();
  await check();
  await page.getByText('当前已是最新版本', { exact: true }).waitFor();
  await page.evaluate(() => { updaterTest.mode = 'error'; });
  await check();
  await page.getByRole('alert').filter({ hasText: '检查更新失败' }).waitFor();
  await page.evaluate(() => { updaterTest.mode = 'available'; updaterTest.busy = true; });
  await check();
  await page.getByText('可更新到 0.2.0', { exact: false }).waitFor();
  await page.getByRole('button', { name: '下载并安装', exact: true }).click();
  await page.getByRole('alert').filter({ hasText: '请先等待压缩或解压任务完成' }).waitFor();
  assert.equal(await page.evaluate(() => updaterTest.installs), 0);
  assert.equal(await page.evaluate(() => updaterTest.releases), 0);
  await page.evaluate(() => { updaterTest.busy = false; updaterTest.installFails = true; });
  await page.getByRole('button', { name: '下载并安装', exact: true }).click();
  await page.getByRole('alert').filter({ hasText: 'signature verification failed' }).waitFor();
  assert.equal(await page.evaluate(() => updaterTest.releases), 1);
  await page.evaluate(() => { updaterTest.installFails = false; });
  await page.getByRole('button', { name: '下载并安装', exact: true }).click();
  await page.getByText(/正在下载更新…\s*50%/).waitFor();
  assert.equal(await page.getByRole('button', { name: '检查更新', exact: true }).isDisabled(), true);
  await page.evaluate(() => updaterTest.finish());
  await page.waitForFunction(() => updaterTest.restarts === 1);
  assert.deepEqual(errors, []);
  console.log('updater UI: no update, network failure, busy-task guard, signature failure, progress and restart passed');
} catch (error) {
  console.error(await page.locator('.update-settings').innerText());
  throw error;
} finally {
  await browser.close();
  await server.close();
}
