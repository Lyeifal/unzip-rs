<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { saveToolPaths, waitForToolPathsSave, type ToolPaths } from '../composables/useToolPaths';
import type { Config } from '../types';

const rows: { key: keyof ToolPaths; label: string; hint: string }[] = [
  { key: 'seven_zip', label: '7-Zip 路径', hint: '压缩、解压共用；留空使用内置 7-Zip' },
  { key: 'unrar', label: 'UnRAR 路径', hint: 'RAR 解压回退；留空使用内置 UnRAR' },
  { key: 'lz4', label: 'lz4.exe 回退路径', hint: '内置 Rust lz4 解码器已可用；仅在需要外部回退时填写' },
];
const draft = reactive<ToolPaths>({ seven_zip: '', unrar: '', lz4: '' });
const saved = ref<ToolPaths>({ ...draft });
const loaded = ref(false), saving = ref(false), error = ref(''), message = ref('');
const dirty = computed(() => rows.some(row => draft[row.key] !== saved.value[row.key]));

async function load() {
  loaded.value = false; error.value = '';
  try {
    await waitForToolPathsSave();
    const cfg = await invoke<Config>('load_config');
    for (const row of rows) draft[row.key] = cfg[row.key];
    saved.value = { ...draft };
    loaded.value = true;
  } catch(e) { error.value = `工具路径加载失败：${e}`; }
}
async function browse(key: keyof ToolPaths) {
  error.value = ''; message.value = '';
  try {
    const path = await invoke<string | null>('pick_file', { current: draft[key] });
    if (path) draft[key] = path;
  } catch(e) { error.value = `选择文件失败：${e}`; }
}
async function save() {
  if (!loaded.value || saving.value) return;
  saving.value = true; error.value = ''; message.value = '';
  try {
    const result = await saveToolPaths(draft);
    Object.assign(draft, result);
    saved.value = { ...result };
    message.value = '已保存，下一次压缩或解压生效，无需重启。';
  } catch(e) { error.value = `保存失败：${e}`; }
  finally { saving.value = false; }
}
onMounted(load);
</script>

<template>
  <section class="tool-paths-settings">
    <h3>压缩与解压工具</h3>
    <p class="tool-hint">两个页面共用这些路径。修改后点击保存；正在进行的任务继续使用启动时的工具。</p>
    <fieldset :disabled="!loaded || saving" class="tool-fields">
      <div v-for="row in rows" :key="row.key" class="tool-row">
        <label :for="`tool-path-${row.key}`">{{ row.label }}</label>
        <div class="tool-control">
          <input :id="`tool-path-${row.key}`" v-model="draft[row.key]" class="input-glass" :placeholder="row.hint" spellcheck="false" @input="message = ''; error = ''" />
          <button class="btn-glass" :aria-label="`浏览${row.label}`" @click="browse(row.key)">浏览…</button>
        </div>
        <span class="tool-hint">{{ row.hint }}</span>
      </div>
    </fieldset>
    <div class="tool-save-row">
      <button class="btn-glass btn-primary" :disabled="!loaded || saving || !dirty" @click="save">{{ saving ? '保存中…' : '保存工具路径' }}</button>
      <span v-if="dirty && !saving" class="tool-hint">有未保存的修改</span>
      <button v-if="!loaded && error" class="btn-glass" @click="load">重新加载</button>
    </div>
    <p v-if="message" role="status" class="tool-success">{{ message }}</p>
    <p v-if="error" role="alert" class="tool-error">{{ error }}</p>
  </section>
</template>

<style scoped>
h3 { margin: 0 0 12px; font-size: 16px; }
.tool-hint { font-size: 12px; color: var(--text-secondary); line-height: 1.6; }
.tool-fields { padding: 0; margin: 14px 0; border: 0; min-width: 0; display: grid; gap: 14px; }
.tool-row { display: grid; gap: 6px; font-size: 13px; }
.tool-control, .tool-save-row { display: flex; gap: 8px; align-items: center; }
.tool-control input { flex: 1; min-width: 0; }
.tool-control button { flex: 0 0 auto; }
.tool-save-row { flex-wrap: wrap; }
.tool-error { color: var(--error); font-size: 12px; }
.tool-success { color: var(--success); font-size: 12px; }
</style>
