<script setup lang="ts">
import { onMounted } from 'vue';
import { useAppUpdater } from '../composables/useAppUpdater';

const updater = useAppUpdater();
onMounted(() => { void updater.loadCurrentVersion(); });
</script>

<template>
  <section class="update-settings">
    <h3>应用更新</h3>
    <p class="update-hint">当前版本：{{ updater.currentVersion.value || '读取中…' }}。应用启动时会检查 GitHub Releases；下载和安装由你确认。</p>
    <p v-if="updater.available.value" class="update-available" role="status">
      可更新到 {{ updater.available.value.version }}
      <span v-if="updater.available.value.body" class="update-notes">{{ updater.available.value.body }}</span>
    </p>
    <div class="update-actions">
      <button class="btn-glass" :disabled="updater.checking.value || updater.installing.value" @click="updater.checkForUpdates()">
        {{ updater.checking.value ? '检查中…' : '检查更新' }}
      </button>
      <button v-if="updater.available.value" class="btn-glass btn-primary" :disabled="updater.checking.value || updater.installing.value" @click="updater.installUpdate()">
        {{ updater.installing.value ? '正在安装…' : '下载并安装' }}
      </button>
    </div>
    <p v-if="updater.status.value" role="status" class="update-hint">{{ updater.status.value }}<span v-if="updater.progress.value !== null"> {{ updater.progress.value }}%</span></p>
    <p v-if="updater.error.value" role="alert" class="update-error">{{ updater.error.value }}</p>
    <p class="update-hint">安装时应用会关闭，请先完成正在进行的压缩或解压任务。</p>
  </section>
</template>

<style scoped>
h3 { margin: 0 0 12px; font-size: 16px; }
.update-hint { color: var(--text-secondary); font-size: 12px; line-height: 1.6; }
.update-available { color: var(--text-primary); font-size: 14px; font-weight: 600; }
.update-notes { display: block; white-space: pre-wrap; margin-top: 8px; color: var(--text-secondary); font-size: 12px; font-weight: 400; }
.update-actions { display: flex; flex-wrap: wrap; gap: 8px; }
.update-error { color: var(--error); font-size: 12px; }
</style>
