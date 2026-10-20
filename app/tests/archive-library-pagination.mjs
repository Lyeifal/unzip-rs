// Run with node app/tests/archive-library-pagination.mjs. PLAYWRIGHT_MODULE can
// point to an existing Playwright ESM installation; no user data is touched.
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
    localStorage.setItem('pack_options', JSON.stringify({ output_dir: '' }));
    window.libraryTest = { queries: [], updates: [], delayed: null, release: null, failSave: false, delaySave: false, releaseSave: null, rows: Array.from({ length: 50 }, (_, i) => {
      const n = String(i + 1).padStart(2, '0');
      return { record_id: `d:/out/item-${n}.zip`, source_name: `item ${n}`, archive_name: `item-${n}.zip`, password: null,
        source_size: { bytes: i + 1, mb: '0.00', gb: '0.000', human: `${i + 1} B` },
        compressed_size: { bytes: i + 1, mb: '0.00', gb: '0.000', human: `${i + 1} B` },
        compression_ratio: '50.0%', is_volume: false, volume_count: 1, created_at: '2026-10-16',
        metadata: { notes: '', custom_fields: { category: i === 49 ? 'old choice' : '文档' }, schema_version: 1 } };
    }) };
  });
  await page.route('**/src/main.ts*', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: `
import {mockIPC,mockWindows} from '/node_modules/@tauri-apps/api/mocks.js';
mockWindows('main');
mockIPC(async(cmd,args)=>{
 const t=window.libraryTest;
 if(cmd==='take_shell_requests') return [];
 if(cmd==='query_archive_library') {
   t.queries.push({...args});
   let rows=t.rows.filter(row => (!args.search || [row.source_name,row.archive_name,row.metadata.notes,...Object.values(row.metadata.custom_fields)].join(' ').toLowerCase().includes(args.search.toLowerCase())) && Object.entries(args.filters).every(([key,value]) => row.metadata.custom_fields[key]===value));
   if(args.sort==='size') rows=[...rows].sort((a,b)=>b.source_size.bytes-a.source_size.bytes);
   else if(args.sort==='date') rows=[...rows].sort((a,b)=>b.created_at.localeCompare(a.created_at)||a.record_id.localeCompare(b.record_id));
   else rows=[...rows].sort((a,b)=>a.source_name.localeCompare(b.source_name));
   const result={path:'C:/Mock/pack_mapping.sqlite3',result:{items:rows.slice((args.page-1)*args.pageSize,args.page*args.pageSize),page:args.page,page_size:args.pageSize,total:rows.length,stats:{total:t.rows.length,source_bytes:1275,compressed_bytes:1275,avg_ratio:50},filter_options:{category:['old choice','文档']}}};
   if(t.delayed===args.page){t.delayed=null;await new Promise(resolve=>t.release=resolve);}
   return result;
 }
 if(cmd==='update_archive_metadata') {t.updates.push(structuredClone(args));if(t.delaySave){t.delaySave=false;await new Promise(resolve=>t.releaseSave=resolve);}if(t.failSave)throw new Error('模拟磁盘写入失败');const row=t.rows.find(row=>row.record_id===args.recordId);row.metadata=structuredClone(args.metadata);return;}
 if(cmd==='import_archive_library') return null;
 if(cmd==='get_shell_integration'||cmd==='plugin:window|is_maximized') return false;
 return null;
}, {shouldMockEvents:true});
` + await response.text() });
  });
  await page.goto(server.resolvedUrls.local[0]);
  await page.getByRole('button', { name: '📚 压缩包管理', exact: true }).click();
  await page.getByText('显示 1–24 条，共 50 条匹配记录').waitFor();
  assert.equal(await page.locator('.game-card').count(), 24);
  assert.equal(await page.getByText('总数：').count(), 1);

  await page.getByRole('button', { name: '下一页' }).click();
  await page.getByText('显示 25–48 条，共 50 条匹配记录').waitFor();
  assert.equal(await page.locator('.game-card').count(), 24);
  await page.getByRole('button', { name: '下一页' }).click();
  await page.getByText('显示 49–50 条，共 50 条匹配记录').waitFor();
  assert.equal(await page.locator('.game-card').count(), 2);

  await page.getByPlaceholder('搜索文件名称、压缩包名、备注或词条…').fill('item 50');
  await page.getByText('显示 1–1 条，共 1 条匹配记录').waitFor();
  assert.equal(await page.locator('.game-card').count(), 1);
  assert.equal((await page.evaluate(() => libraryTest.queries.at(-1))).page, 1);
  assert.equal(await page.getByText('50 个条目').count(), 1);

  await page.getByPlaceholder('搜索文件名称、压缩包名、备注或词条…').fill('');
  await page.getByText('显示 1–24 条，共 50 条匹配记录').waitFor();
  await page.evaluate(() => libraryTest.delayed = 2);
  await page.getByRole('button', { name: '下一页' }).click();
  await page.waitForFunction(() => !!window.libraryTest.release);
  await page.getByPlaceholder('搜索文件名称、压缩包名、备注或词条…').fill('item 49');
  await page.getByText('显示 1–1 条，共 1 条匹配记录').waitFor();
  await page.evaluate(() => libraryTest.release());
  await page.waitForTimeout(100);
  assert.equal(await page.locator('.game-card').count(), 1, 'late page response must not replace current search');
  assert.equal(await page.getByText('显示 1–1 条，共 1 条匹配记录').count(), 1);

  await page.getByPlaceholder('搜索文件名称、压缩包名、备注或词条…').fill('');
  await page.getByText('显示 1–24 条，共 50 条匹配记录').waitFor();
  await page.getByRole('combobox', { name: '筛选分类' }).click();
  await page.getByRole('option', { name: 'old choice' }).click();
  await page.getByText('显示 1–1 条，共 1 条匹配记录').waitFor();
  assert.equal(await page.locator('.game-card').count(), 1, 'an option from another page must filter the whole library');
  assert.equal(await page.locator('.game-card .card-title').innerText(), 'item 50');
  await page.getByRole('combobox', { name: '筛选分类' }).click();
  await page.getByRole('option', { name: '全部分类' }).click();
  await page.getByText('显示 1–24 条，共 50 条匹配记录').waitFor();
  await page.getByRole('combobox', { name: '条目排序方式' }).click();
  await page.getByRole('option', { name: '按大小排序' }).click();
  await page.getByText('显示 1–24 条，共 50 条匹配记录').waitFor();
  await page.waitForFunction(() => window.libraryTest.queries.at(-1)?.sort === 'size');
  assert.equal(await page.locator('.game-card .card-title').first().innerText(), 'item 50');

  // Finishing one save must not reset a different field currently being edited.
  await page.evaluate(() => libraryTest.delaySave = true);
  const firstNote = page.locator('.game-card textarea').nth(0);
  const secondNote = page.locator('.game-card textarea').nth(1);
  await firstNote.fill('first committed note');
  await secondNote.click();
  await page.waitForFunction(() => !!libraryTest.releaseSave);
  await secondNote.fill('second unfinished draft');
  await page.evaluate(() => libraryTest.releaseSave());
  await page.waitForFunction(() => libraryTest.rows[49].metadata.notes === 'first committed note');
  await page.waitForTimeout(150);
  assert.equal(await secondNote.inputValue(), 'second unfinished draft');
  assert.equal(await secondNote.evaluate(element => document.activeElement === element), true);
  await page.getByPlaceholder('搜索文件名称、压缩包名、备注或词条…').click();
  await page.waitForFunction(() => libraryTest.rows[48].metadata.notes === 'second unfinished draft');
  await page.getByText('加载中...').waitFor({ state: 'hidden' });

  // Keep a failed edit visible through reloads, with an explicit retry action.
  await page.evaluate(() => libraryTest.failSave = true);
  await firstNote.fill('retry this note');
  await page.getByPlaceholder('搜索文件名称、压缩包名、备注或词条…').click();
  await page.getByRole('button', { name: '重试保存' }).waitFor();
  assert.equal(await firstNote.inputValue(), 'retry this note');
  await page.getByRole('button', { name: '🔄 刷新', exact: true }).click();
  await page.getByText('加载中...').waitFor({ state: 'hidden' });
  assert.equal(await firstNote.inputValue(), 'retry this note');
  assert.equal(await page.getByRole('button', { name: '重试保存' }).count(), 1);
  await page.evaluate(() => libraryTest.failSave = false);
  await page.getByRole('button', { name: '重试保存' }).click();
  await page.getByRole('button', { name: '重试保存' }).waitFor({ state: 'hidden' });
  await page.waitForFunction(() => libraryTest.rows[49].metadata.notes === 'retry this note');
  await page.getByText('加载中...').waitFor({ state: 'hidden' });

  // Keyboard navigation keeps the active option visible and restores control focus.
  const filter = page.getByRole('combobox', { name: '筛选分类' });
  await filter.focus();
  await filter.press('Enter');
  await filter.press('End');
  const option = page.locator('.select-option.active');
  assert.equal(await option.innerText(), 'old choice');
  assert.equal(await option.evaluate(element => {
    const bounds = element.getBoundingClientRect(), menu = element.parentElement.getBoundingClientRect();
    return bounds.top >= menu.top && bounds.bottom <= menu.bottom;
  }), true);
  await filter.press('Enter');
  assert.equal(await filter.getAttribute('aria-expanded'), 'false');
  assert.equal(await filter.evaluate(element => document.activeElement === element), true);
  await page.getByText('加载中...').waitFor({ state: 'hidden' });

  // Removing an active filter field must remove that filter from the query too.
  await page.getByRole('button', { name: '管理字段', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: '⚙️ 应用设置' });
  await dialog.waitFor();
  assert.equal(await dialog.locator('.sidebar-link.active').innerText(), '压缩包管理');
  await dialog.getByRole('button', { name: '移除字段 分类', exact: true }).click();
  await page.keyboard.press('Escape');
  await dialog.waitFor({ state: 'hidden' });
  await page.waitForFunction(() => !('category' in libraryTest.queries.at(-1).filters));
  assert.equal(await page.getByRole('combobox', { name: '筛选分类' }).count(), 0);
  await page.getByText('显示 1–24 条，共 50 条匹配记录').waitFor();
  for (const [width, height] of [[760, 620], [1280, 900]]) {
    await page.setViewportSize({ width, height });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, 'library must fit window width');
    await page.getByRole('button', { name: '管理字段', exact: true }).click();
    await dialog.waitFor();
    if (width === 1280) {
      await dialog.getByRole('navigation', { name: '设置导航' }).getByRole('button', { name: '主题预设', exact: true }).click();
      await dialog.locator('.preset-card').filter({ hasText: '墨黑' }).click();
      await dialog.getByRole('navigation', { name: '设置导航' }).getByRole('button', { name: '压缩包管理', exact: true }).click();
    }
    await page.waitForTimeout(500);
    assert.equal(await dialog.evaluate(element => element.scrollWidth <= element.clientWidth), true, 'settings must fit window width');
    if (process.env.UI_SCREENSHOT_DIR) await page.screenshot({ path: path.join(process.env.UI_SCREENSHOT_DIR, `settings-${width}.png`) });
    await page.keyboard.press('Escape');
    await dialog.waitFor({ state: 'hidden' });
    await page.waitForTimeout(500);
    if (process.env.UI_SCREENSHOT_DIR) await page.screenshot({ path: path.join(process.env.UI_SCREENSHOT_DIR, `library-${width}.png`) });
    await page.getByRole('button', { name: '🗜️ 打包压缩', exact: true }).click();
    await page.waitForTimeout(500);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, 'pack view must fit window width');
    if (process.env.UI_SCREENSHOT_DIR) await page.screenshot({ path: path.join(process.env.UI_SCREENSHOT_DIR, `pack-${width}.png`) });
    await page.getByRole('button', { name: '📚 压缩包管理', exact: true }).click();
    await page.getByText('加载中...').waitFor({ state: 'hidden' });
  }
  assert.deepEqual(errors, []);
  console.log('archive library UI: pagination, stale responses, continuous editing, failed-save retry, keyboard selection, settings navigation and removed filters passed');
} finally {
  await browser.close();
  await server.close();
}
