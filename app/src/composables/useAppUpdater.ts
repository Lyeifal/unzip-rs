import { ref, shallowRef } from 'vue';
import { getVersion } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';
import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

const available = shallowRef<Update | null>(null);
const currentVersion = ref('');
const checking = ref(false);
const installing = ref(false);
const progress = ref<number | null>(null);
const status = ref('');
const error = ref('');

async function loadCurrentVersion() {
  if (currentVersion.value) return;
  try { currentVersion.value = await getVersion(); } catch { /* 浏览器预览没有 Tauri 后端 */ }
}

async function checkForUpdates(silent = false) {
  if (checking.value || installing.value) return;
  checking.value = true;
  error.value = '';
  if (!silent) status.value = '正在检查更新…';
  try {
    await loadCurrentVersion();
    const found = await check({ timeout: 20000 });
    const old = available.value;
    available.value = found;
    if (old && old !== found) await old.close().catch(() => {});
    status.value = found ? `发现新版本 ${found.version}` : '当前已是最新版本';
  } catch (cause) {
    if (!silent) error.value = `检查更新失败：${String(cause)}`;
    status.value = '';
  } finally {
    checking.value = false;
  }
}

async function installUpdate() {
  const update = available.value;
  if (!update || checking.value || installing.value) return;
  installing.value = true;
  error.value = '';
  progress.value = null;
  status.value = '正在下载更新…';
  let received = 0;
  let total: number | undefined;
  let acquired = false;
  try {
    await invoke('begin_update_install');
    acquired = true;
    await update.downloadAndInstall(event => {
      if (event.event === 'Started') total = event.data.contentLength;
      if (event.event === 'Progress') received += event.data.chunkLength;
      if (total && total > 0) progress.value = Math.min(100, Math.round(received * 100 / total));
      if (event.event === 'Finished') status.value = '下载完成，正在安装…';
    }, { timeout: 120000 });
    // Windows 安装器通常会退出并重启应用；其余桌面系统需要主动重启。
    await relaunch();
  } catch (cause) {
    if (acquired) await invoke('end_update_install').catch(() => {});
    error.value = `安装更新失败：${String(cause)}`;
    status.value = '';
    installing.value = false;
    progress.value = null;
  }
}

export function useAppUpdater() {
  return { available, currentVersion, checking, installing, progress, status, error,
    loadCurrentVersion, checkForUpdates, installUpdate };
}
