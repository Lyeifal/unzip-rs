<script setup lang="ts">
import { computed, onActivated, onDeactivated, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import GameCard from './GameCard.vue';
import StyledSelect from './StyledSelect.vue';
import { useLibraryFields } from '../composables/useLibraryFields';
import type { ArchiveLibraryPage, GameMapping, GameMetadata } from '../types';
defineEmits<{ 'open-settings': [] }>();
const library = useLibraryFields();
const storagePath = ref('');
const mappings = ref<GameMapping[]>([]);
const loading = ref(false), error = ref(''), searchQuery = ref('');
const saving = ref(0);
const notice = ref('');
const migrationWarning = ref('');
const filterValues = ref<Record<string, string>>({});
const availableOptions = ref<Record<string, string[]>>({});
const sortBy = ref<'name' | 'size' | 'ratio' | 'date'>('name');
const page = ref(1);
const pageSize = 24;
const matchingTotal = ref(0);
const globalStats = ref({ total: 0, source_bytes: 0, compressed_bytes: 0, avg_ratio: 0 });
const sortOptions = [{value:'name',label:'按名称排序'},{value:'size',label:'按大小排序'},{value:'ratio',label:'按压缩率排序'},{value:'date',label:'按日期排序'}];
let loadToken = 0;
let writeQueue = Promise.resolve();
let searchTimer: ReturnType<typeof setTimeout> | undefined;
const selectFields = computed(() => library.fields.value.filter(f => f.type === 'select'));
const totalPages = computed(() => Math.max(1, Math.ceil(matchingTotal.value / pageSize)));
const rangeStart = computed(() => matchingTotal.value ? (page.value - 1) * pageSize + 1 : 0);
const rangeEnd = computed(() => Math.min(page.value * pageSize, matchingTotal.value));

async function loadPage(resetPage = false) {
  if (resetPage) page.value = 1;
  const token = ++loadToken;
  loading.value = true; error.value = '';
  await writeQueue;
  if (token !== loadToken) return;
  try {
    const payload = await invoke<{ path: string; result: ArchiveLibraryPage }>('query_archive_library', {
      page: page.value,
      pageSize,
      search: searchQuery.value.trim(),
      sort: sortBy.value,
      filters: Object.fromEntries(Object.entries(filterValues.value).filter(([, value]) => !!value)),
      optionFields: selectFields.value.map(field => field.id),
    });
    if (token !== loadToken) return;
    const result = payload.result;
    if (!Array.isArray(result.items) || !Number.isFinite(result.total) || !result.stats || !Number.isFinite(result.stats.total)) {
      throw new Error('分页记录格式无效');
    }
    if (!result.items.every(m => m && typeof m.source_name === 'string' && typeof m.archive_name === 'string' && typeof m.created_at === 'string' && typeof m.compression_ratio === 'string' && Number.isFinite(m.source_size?.bytes) && Number.isFinite(m.compressed_size?.bytes))) {
      throw new Error('分页记录中的条目缺少名称、大小或日期信息');
    }
    const lastPage = Math.max(1, Math.ceil(result.total / pageSize));
    if (result.page > lastPage) {
      page.value = lastPage;
      void loadPage();
      return;
    }
    storagePath.value = payload.path;
    matchingTotal.value = result.total;
    globalStats.value = result.stats;
    availableOptions.value = result.filter_options ?? {};
    page.value = result.page;
    mappings.value = result.items.map(m => ({ ...m, metadata: library.normalizeMetadata(m.metadata) }));
  } catch(e) {
    if (token === loadToken) error.value = '加载失败：' + e;
  } finally { if (token === loadToken) loading.value = false; }
}
function updateMetadata(id: string | undefined, metadata: GameMetadata) {
  if (loading.value) return;
  if (!id) { error.value = '记录缺少 ID，请重新导入旧 JSON'; return; }
  const index = mappings.value.findIndex(item => item.record_id === id);
  if (index < 0) return;
  mappings.value[index] = { ...mappings.value[index], metadata };
  const snapshot = JSON.parse(JSON.stringify(metadata)) as GameMetadata;
  saving.value++;
  writeQueue = writeQueue.then(async () => {
    try { await invoke('update_archive_metadata', { recordId: id, metadata: snapshot }); error.value = ''; }
    catch(e) { error.value = '保存失败：' + e; }
    finally { saving.value--; queueMicrotask(() => void loadPage()); }
  });
}
async function importLegacy() {
  loading.value = true; error.value = ''; notice.value = '';
  try {
    await writeQueue;
    const count = await invoke<number | null>('import_archive_library', { path: null });
    if (count !== null) { await loadPage(true); notice.value = `已导入 ${count} 条新记录，原文件保持不变。`; }
  } catch(e) { error.value = '导入失败：' + e; }
  finally { loading.value = false; }
}
async function exportBackup() {
  loading.value = true; error.value = ''; notice.value = '';
  try {
    await writeQueue;
    const path = await invoke<string | null>('export_archive_library');
    if (path) notice.value = '备份已导出：' + path;
  } catch(e) { error.value = '导出失败：' + e; }
  finally { loading.value = false; }
}
function filterOptions(id: string, label: string, options: string[]) {
  const existing = availableOptions.value[id] ?? [];
  return [{ value:'',label:'全部' + label }, ...[...new Set([...options, ...existing])].map(value => ({value,label:value}))];
}
const stats = computed(() => ({
  total: globalStats.value.total,
  totalSourceSize: (globalStats.value.source_bytes / 1024**3).toFixed(2),
  totalCompressedSize: (globalStats.value.compressed_bytes / 1024**3).toFixed(2),
  avgRatio: globalStats.value.avg_ratio.toFixed(1),
}));
watch(searchQuery, () => {
  if (searchTimer) clearTimeout(searchTimer);
  searchTimer = setTimeout(() => void loadPage(true), 250);
});
watch(sortBy, () => void loadPage(true));
watch(filterValues, () => void loadPage(true), { deep: true });
function goToPage(next: number) {
  if (next < 1 || next > totalPages.value || next === page.value) return;
  page.value = next;
  void loadPage();
}
let migration: Promise<void> | undefined;
async function migrateKnownDirectories() {
  try {
    const dirs = new Set([localStorage.getItem('library_output_dir'), JSON.parse(localStorage.getItem('pack_options') ?? '{}').output_dir].filter((p): p is string => typeof p === 'string' && !!p.trim()));
    for (const dir of dirs) {
      try { await invoke('import_archive_library', { path: dir.replace(/[\\/]$/, '') + '/pack_mapping.json' }); }
      catch(e) { migrationWarning.value = '部分旧记录未能自动导入，可手动选择 JSON 重试：' + e; }
    }
  } catch(e) { migrationWarning.value = '旧目录设置读取失败，可手动导入 JSON：' + e; }
}
onActivated(async () => {
  loading.value = true;
  await (migration ??= migrateKnownDirectories());
  await loadPage();
});
onDeactivated(() => {
  loadToken++;
  if (searchTimer) clearTimeout(searchTimer);
});
</script>

<template>
  <div class="page-content">
    <!-- 标题栏 -->
    <div class="library-header">
      <h2 class="library-title">📚 压缩包管理</h2>
      <p class="library-subtitle">查看打包记录，用备注和自定义字段整理各类文件</p>
      <button class="btn-glass fields-settings-btn" @click="$emit('open-settings')">管理字段</button>
      <span v-if="saving" role="status">正在保存…</span>
    </div>

    <!-- 控制栏 -->
    <div class="glass-card control-bar">
      <div class="control-row">
        <div class="dir-control">
          <label class="control-label">📁 记录位置：</label>
          <input
            :value="storagePath"
            class="input-glass dir-input"
            placeholder="AppData 应用专用目录"
            readonly
          />
          <button class="btn-glass" :disabled="loading || saving > 0" @click="importLegacy">导入旧 JSON</button>
          <button class="btn-glass" :disabled="loading || saving > 0" @click="exportBackup">导出备份</button>
          <button
            class="btn-glass"
            :disabled="loading || saving > 0"
            @click="loadPage()"
          >
            🔄 刷新
          </button>
        </div>
      </div>
      <p v-if="notice" class="library-message" role="status">{{ notice }}</p>
      <p v-if="migrationWarning" class="library-message" role="alert">{{ migrationWarning }}</p>

      <!-- 统计信息 -->
      <div v-if="stats.total > 0" class="stats-row">
        <div class="stat-item">
          <span class="stat-label">总数：</span>
          <span class="stat-value">{{ stats.total }} 个条目</span>
        </div>
        <div class="stat-item">
          <span class="stat-label">压缩前：</span>
          <span class="stat-value">{{ stats.totalSourceSize }} GB</span>
        </div>
        <div class="stat-item">
          <span class="stat-label">压缩后：</span>
          <span class="stat-value">{{ stats.totalCompressedSize }} GB</span>
        </div>
        <div class="stat-item">
          <span class="stat-label">平均压缩率：</span>
          <span class="stat-value">{{ stats.avgRatio }}%</span>
        </div>
      </div>

      <!-- 搜索和筛选 -->
      <div v-if="stats.total > 0" class="filter-row">
        <div class="search-box">
          <span class="search-icon">🔍</span>
          <input
            v-model="searchQuery"
            class="input-glass search-input"
            placeholder="搜索文件名称、压缩包名、备注或词条…"
          />
        </div>
        <StyledSelect v-for="field in selectFields" :key="field.id" :model-value="filterValues[field.id] ?? ''" @update:model-value="filterValues[field.id] = String($event)" class="select-glass" :options="filterOptions(field.id,field.label,field.options)" :ariaLabel="'筛选' + field.label" />

        <StyledSelect v-model="sortBy" class="select-glass" :options="sortOptions" ariaLabel="条目排序方式" />
      </div>
    </div>

    <!-- 加载状态 -->
    <div v-if="loading" class="loading-box glass-card">
      <div class="spinner"></div>
      <span>加载中...</span>
    </div>

    <!-- 错误提示 -->
    <div v-if="error" class="error-box glass-card">
      <span class="error-icon">⚠️</span>
      <span>{{ error }}</span>
    </div>

    <!-- 空状态 -->
    <div v-if="!loading && !error && stats.total === 0" class="empty-box glass-card">
      <div class="empty-icon">📦</div>
      <p class="empty-text">还没有打包记录</p>
      <p class="empty-hint">
        请先使用“打包压缩”功能，或导入以前的 pack_mapping.json
      </p>
    </div>

    <!-- 游戏卡片网格 -->
    <div v-if="mappings.length > 0" class="game-grid">
      <GameCard
        v-for="mapping in mappings"
        :key="mapping.record_id ?? mapping.archive_path ?? mapping.archive_name"
        :mapping="mapping"
        :disabled="loading"
        @update:metadata="updateMetadata(mapping.record_id, $event)"
      />
    </div>

    <div v-if="!error && matchingTotal > 0" class="pagination-bar glass-card" role="navigation" aria-label="压缩记录分页">
      <span>显示 {{ rangeStart }}–{{ rangeEnd }} 条，共 {{ matchingTotal }} 条匹配记录</span>
      <div class="pagination-actions">
        <button class="btn-glass" :disabled="loading || page <= 1" @click="goToPage(page - 1)">上一页</button>
        <span>第 {{ page }} / {{ totalPages }} 页</span>
        <button class="btn-glass" :disabled="loading || page >= totalPages" @click="goToPage(page + 1)">下一页</button>
      </div>
    </div>

    <!-- 无搜索结果 -->
    <div
      v-if="!loading && stats.total > 0 && matchingTotal === 0"
      class="empty-box glass-card"
    >
      <div class="empty-icon">🔍</div>
      <p class="empty-text">没有找到匹配的条目</p>
      <p class="empty-hint">尝试调整搜索条件或筛选器</p>
    </div>
  </div>
</template>

<style scoped>
.fields-settings-btn { margin-top: 12px; }
.library-message { margin: 0; color: var(--text-secondary); font-size: 12px; overflow-wrap: anywhere; }
.page-content {
  display: flex;
  flex-direction: column;
  gap: 14px;
  width: 100%;
  min-width: 0;
}

/* 标题栏 */
.library-header {
  text-align: left;
  margin-bottom: 8px;
}

.library-title {
  font-size: 28px;
  font-weight: 700;
  color: var(--text-primary);
  margin: 0 0 8px;
}

.library-subtitle {
  font-size: 14px;
  color: var(--text-secondary);
  margin: 0;
}

/* 控制栏 */
.control-bar {
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.control-row {
  display: flex;
  gap: 12px;
}

.dir-control {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px;
  flex: 1;
}

.control-label {
  font-size: 14px;
  color: var(--text-primary);
  font-weight: 500;
  white-space: nowrap;
}

.dir-input {
  flex: 1 1 220px;
  min-width: 0;
}

/* 统计信息 */
.stats-row {
  display: flex;
  justify-content: space-around;
  padding: 16px;
  background: rgba(255, 255, 255, 0.03);
  border-radius: var(--radius-md);
  flex-wrap: wrap;
  gap: 16px;
}

.stat-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
}

.stat-label {
  font-size: 12px;
  color: var(--text-secondary);
}

.stat-value {
  font-size: 18px;
  font-weight: 700;
  color: var(--primary);
  font-family: "Cascadia Mono", Consolas, monospace;
}

/* 筛选行 */
.filter-row {
  display: flex;
  min-width: 0;
  flex-wrap: wrap;
  gap: 12px;
  align-items: center;
}

.search-box {
  flex: 1;
  position: relative;
  display: flex;
  align-items: center;
  min-width: 240px;
}

.filter-row > .select-glass {
  flex: 0 1 200px;
  min-width: 175px;
}

.search-icon {
  position: absolute;
  left: 12px;
  font-size: 16px;
  opacity: 0.6;
  pointer-events: none;
}

.search-input {
  flex: 1;
  padding-left: 40px !important;
}

/* 状态盒子 */
.loading-box,
.error-box,
.empty-box {
  padding: 48px 24px;
  text-align: center;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
}

.loading-box {
  display: flex;
  flex-direction: row;
  justify-content: center;
  gap: 12px;
}

.spinner {
  width: 24px;
  height: 24px;
  border: 3px solid rgba(255, 255, 255, 0.2);
  border-top-color: var(--primary);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

.error-box {
  background: rgba(255, 55, 95, 0.1);
  border-color: var(--error);
}

.error-icon {
  font-size: 48px;
}

.empty-icon {
  font-size: 64px;
  opacity: 0.5;
}

.empty-text {
  font-size: 18px;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0;
}

.empty-hint {
  font-size: 14px;
  color: var(--text-tertiary);
  margin: 0;
}

/* 游戏卡片网格 */
.game-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 14px;
}

.pagination-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 12px;
  padding: 12px 16px;
  color: var(--text-secondary);
  font-size: 13px;
}

.pagination-actions {
  display: flex;
  align-items: center;
  gap: 12px;
}

@media (max-width: 960px) {
  .game-grid {
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 12px;
  }
}

@media (max-width: 600px) {
  .game-grid {
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: 12px;
  }

  .stats-row {
    flex-direction: column;
    align-items: stretch;
  }

  .stat-item {
    flex-direction: row;
    justify-content: space-between;
  }
}
</style>
