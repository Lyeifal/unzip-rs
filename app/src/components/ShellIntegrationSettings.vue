<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import StyledSelect from './StyledSelect.vue';
const enabled = ref(false), busy = ref(true), error = ref('');
const quickFormat = ref('7z'), savedFormat = ref('7z'), formatBusy = ref(true), formatError = ref('');
const formatOptions = [{ value: '7z', label: '7z（默认）' }, { value: 'zip', label: 'zip（兼容性好）' }];
const display = ref({ showProgress: true, showConsole: false });
const displayBusy = ref(true), displayError = ref('');
onMounted(async () => {
  await Promise.all([
    (async () => {
      try { const value = await invoke<typeof display.value>('get_shell_display'); if (value) display.value = value; }
      catch(e) { displayError.value = String(e); }
      finally { displayBusy.value = false; }
    })(),
    (async () => {
      try { enabled.value = await invoke<boolean>('get_shell_integration'); }
      catch(e) { error.value = String(e); }
      finally { busy.value = false; }
    })(),
    (async () => {
      try {
        const format = await invoke<string>('get_quick_pack_format');
        savedFormat.value = quickFormat.value = format === 'zip' ? 'zip' : '7z';
      } catch(e) { formatError.value = String(e); }
      finally { formatBusy.value = false; }
    })(),
  ]);
});
async function saveDisplay(key: 'showProgress' | 'showConsole', event: Event) {
  const target = event.target as HTMLInputElement;
  const previous = { ...display.value };
  display.value = { ...previous, [key]: target.checked };
  displayBusy.value = true; displayError.value = '';
  try { await invoke('set_shell_display', { ...display.value }); }
  catch(e) { display.value = previous; displayError.value = String(e); }
  finally { target.checked = display.value[key]; displayBusy.value = false; }
}
async function saveFormat(value: string | number) {
  if (formatBusy.value) return;
  quickFormat.value = String(value); formatBusy.value = true; formatError.value = '';
  try {
    const format = await invoke<string>('set_quick_pack_format', { format: quickFormat.value });
    savedFormat.value = quickFormat.value = format;
  } catch(e) { quickFormat.value = savedFormat.value; formatError.value = String(e); }
  finally { formatBusy.value = false; }
}
async function toggle(event: Event) {
  const next = (event.target as HTMLInputElement).checked;
  busy.value = true; error.value = '';
  try { enabled.value = await invoke<boolean>('set_shell_integration', { enabled: next }); }
  catch(e) { error.value = String(e); }
  finally { (event.target as HTMLInputElement).checked = enabled.value; busy.value = false; }
}
</script>

<template>
  <section class="shell-settings">
    <h3>资源管理器右键菜单</h3>
    <label class="toggle-option"><input type="checkbox" :checked="enabled" :disabled="busy" @change="toggle" /><span>添加 unzip-rs 右键子菜单</span></label>
    <div class="quick-format"><span>快捷压缩默认格式</span><StyledSelect :modelValue="quickFormat" :options="formatOptions" ariaLabel="快捷压缩默认格式" :disabled="formatBusy" @update:modelValue="saveFormat" /></div>
    <p>“添加到同名压缩包”直接在源文件旁创建压缩包，例如 文件.txt → 文件.{{ quickFormat }}、资料文件夹 → 资料文件夹.{{ quickFormat }}。文件去掉原扩展名，文件夹保留完整名称。多选时每项单独打包；不加密码、不混入说明文件、不自动分卷。同名包已存在时自动递增避让，不修改旧包。格式更改后自动保存，下一次快捷压缩生效。</p>
    <div class="display-options">
      <label class="toggle-option"><input type="checkbox" :checked="display.showProgress" :disabled="displayBusy" @change="saveDisplay('showProgress', $event)" /><span>右键任务显示进度与日志窗口</span></label>
      <label class="toggle-option"><input type="checkbox" :checked="display.showConsole" :disabled="displayBusy" @change="saveDisplay('showConsole', $event)" /><span>右键任务显示终端日志</span></label>
    </div>
    <p>默认显示应用中的任务状态、已用时间和日志，完成后保留结果；开启终端日志时也显示应用，方便停止和退出。两个开关均关闭时静默执行，成功后退出后台实例，失败或有警告仍显示应用。终端模式完成后保留窗口，退出应用时关闭；停止任务请使用应用中的停止按钮。更改自动保存，下一次右键任务生效，不需要重新注册菜单。</p>
    <p v-if="displayError" role="alert" class="shell-error">{{ displayError }}</p>
    <p v-if="formatError" role="alert" class="shell-error">{{ formatError }}</p>
    <p>右键选中文件或文件夹 → unzip-rs →“解压到当前目录”直接解压到压缩包所在目录；“解压到同名文件夹”为每个包新建同名文件夹。已有同名内容自动递增命名，不覆盖。</p>
    <p>直接解压自动使用已保存的密码库、启用的密码策略和 SHA-256 匹配设置；失败目录开关也沿用解压设置。应用有任务时会排队等待，无需再次点击开始。分卷请选齐同组文件。</p>
    <p>子菜单也保留“添加到压缩列表…”和“添加到解压列表…”，方便在应用中调整参数后开始。启用子菜单会移除本应用旧版的顶层入口。</p>
    <p>Windows 11 中入口位于“显示更多选项”；一次最多选择 100 项。启用后移动程序，请在新位置重新启用。</p>
    <p v-if="error" role="alert" class="shell-error">{{ error }}</p>
  </section>
</template>

<style scoped>
h3 { margin: 0 0 12px; font-size: 16px; }
p { font-size: 12px; color: var(--text-secondary); line-height: 1.6; }
.toggle-option { display: flex; gap: 8px; align-items: center; }
.display-options { display: grid; gap: 12px; margin-top: 16px; }
.quick-format { display: flex; align-items: center; gap: 12px; margin-top: 14px; }
.quick-format :deep(.styled-select) { min-width: 180px; }
.shell-error { color: var(--error); }
</style>
