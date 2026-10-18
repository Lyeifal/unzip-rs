<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
const enabled = ref(false), busy = ref(true), error = ref('');
onMounted(async () => {
  try { enabled.value = await invoke<boolean>('get_shell_integration'); }
  catch(e) { error.value = String(e); }
  finally { busy.value = false; }
});
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
    <label class="toggle-option"><input type="checkbox" :checked="enabled" :disabled="busy" @change="toggle" /><span>添加“压缩”和“解压”入口</span></label>
    <p>在文件夹中框选或多选文件，右键选择“添加到压缩列表”或“添加到解压列表”，到应用中确认参数后开始。文件夹也可直接添加。</p>
    <p>Windows 11 中入口位于“显示更多选项”；一次最多选择 100 项。启用后移动程序，请在新位置重新启用。</p>
    <p v-if="error" role="alert" class="shell-error">{{ error }}</p>
  </section>
</template>

<style scoped>
h3 { margin: 0 0 12px; font-size: 16px; }
p { font-size: 12px; color: var(--text-secondary); line-height: 1.6; }
.toggle-option { display: flex; gap: 8px; align-items: center; }
.shell-error { color: var(--error); }
</style>
