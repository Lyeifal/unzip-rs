<script setup lang="ts">
import { computed } from 'vue';
import { type GameMapping, type GameMetadata } from '../types';
import { useLibraryFields, type LibraryField } from '../composables/useLibraryFields';
import StyledSelect from './StyledSelect.vue';
const props = defineProps<{ mapping: GameMapping; disabled?: boolean }>();
const emit = defineEmits<{ 'update:metadata': [metadata: GameMetadata] }>();
const library = useLibraryFields();
const metadata = computed(() => props.mapping.metadata ?? {});
function saveNotes(event: Event) {
  emit('update:metadata', { ...metadata.value, notes: (event.target as HTMLTextAreaElement).value });
}
function setValue(id: string, value: string | number | boolean) {
  emit('update:metadata', { ...metadata.value, custom_fields: { ...metadata.value.custom_fields, [id]: typeof value === 'boolean' ? value : String(value) } });
}
function valueOf(id: string): string { return String(metadata.value.custom_fields?.[id] ?? ''); }
function optionsFor(field: LibraryField) {
  const current = valueOf(field.id);
  return [{ value: '', label: '未设置' }, ...[...new Set([...field.options, ...(current ? [current] : [])])].map(value => ({ value, label: value }))];
}
const ratioClass = computed(() => {
  const ratio = parseFloat(props.mapping.compression_ratio);
  return ratio <= 60 ? 'ratio-good' : ratio <= 80 ? 'ratio-ok' : 'ratio-poor';
});
const ratioColor = computed(() => {
  const ratio = parseFloat(props.mapping.compression_ratio);
  return ratio <= 60 ? 'var(--success)' : ratio <= 80 ? 'var(--warning)' : 'var(--error)';
});
const ratioWidth = computed(() => {
  const ratio = parseFloat(props.mapping.compression_ratio);
  return `${Number.isFinite(ratio) ? Math.max(0, Math.min(100, ratio)) : 0}%`;
});
</script>

<template>
  <div class="game-card glass-card card-enter">
    <!-- 图标区 -->
    <div class="card-icon">📦</div>

    <!-- 标题区 -->
    <div class="card-title" :title="mapping.source_name">
      {{ mapping.source_name }}
    </div>
    <div class="card-archive" :title="mapping.archive_name">{{ mapping.archive_name }}</div>
    <div v-if="mapping.output_dir" class="card-archive" :title="mapping.output_dir">{{ mapping.output_dir }}</div>
    <details v-if="mapping.output_hashes?.length" class="hash-details">
      <summary>SHA-256 · {{ mapping.output_hashes.length }} 个文件</summary>
      <div v-for="hash in mapping.output_hashes" :key="hash.file_name" class="hash-entry">
        <span>{{ hash.file_name }}</span>
        <code>{{ hash.sha256 }}</code>
      </div>
    </details>
    <div class="password-row">
      <span class="field-label">解压密码</span>
      <span v-if="!mapping.password" class="password-value">无密码</span>
      <span v-else-if="library.showPasswords.value" class="password-value">{{ mapping.password }}</span>
      <span v-else class="password-value" aria-label="密码已隐藏">••••••••</span>
    </div>

    <fieldset class="card-meta metadata-editor" :disabled="disabled">
      <label class="meta-field notes-field">
        <span class="field-label">备注</span>
        <textarea class="input-glass notes-input" :value="metadata.notes ?? ''" rows="2" placeholder="添加备注…" :aria-label="mapping.source_name + '备注'" @change="saveNotes" />
      </label>
      <div v-for="field in library.fields.value" :key="field.id" class="meta-field">
        <label v-if="field.type === 'checkbox'" class="checkbox-field">
          <input type="checkbox" :checked="metadata.custom_fields?.[field.id] === true" @change="setValue(field.id, ($event.target as HTMLInputElement).checked)" />
          <span class="checkbox-label">{{ field.label }}</span>
        </label>
        <template v-else>
          <span class="field-label">{{ field.label }}</span>
          <StyledSelect v-if="field.type === 'select'" class="engine-select" :disabled="disabled" :model-value="valueOf(field.id)" :options="optionsFor(field)" :ariaLabel="mapping.source_name + ' ' + field.label" @update:model-value="setValue(field.id, $event)" />
          <input v-else class="input-glass" :value="valueOf(field.id)" :aria-label="mapping.source_name + ' ' + field.label" @change="setValue(field.id, ($event.target as HTMLInputElement).value)" />
        </template>
      </div>
    </fieldset>

    <!-- 分隔线 -->
    <div class="card-divider"></div>

    <!-- 统计区 -->
    <div class="card-stats">
      <div class="stat-row">
        <span class="stat-label">压缩前</span>
        <span class="stat-value">{{ mapping.source_size.human }}</span>
      </div>
      <div class="stat-row">
        <span class="stat-label">压缩后</span>
        <span class="stat-value">{{ mapping.compressed_size.human }}</span>
      </div>
      <div class="stat-row">
        <span class="stat-label">压缩率</span>
        <span class="stat-value" :class="ratioClass">
          {{ mapping.compression_ratio }}
        </span>
      </div>

      <!-- 压缩率进度条 -->
      <div class="ratio-bar">
        <div
          class="ratio-fill"
          :style="{
            width: ratioWidth,
            background: ratioColor,
          }"
        ></div>
      </div>

      <!-- 附加信息 -->
      <div class="card-footer">
        <span v-if="mapping.is_volume" class="footer-tag">
          📦 {{ mapping.volume_count }} 个分卷
        </span>
        <span class="footer-date">{{ mapping.created_at }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.metadata-editor { border: 0; padding: 0; margin: 0; min-width: 0; }
.password-row { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; }
.password-value { font: 12px "Cascadia Mono", Consolas, monospace; color: var(--text-primary); overflow-wrap: anywhere; user-select: text; }
.hash-details { font-size: 11px; color: var(--text-secondary); }
.hash-details summary { cursor: pointer; }
.hash-entry { display: grid; gap: 4px; margin-top: 8px; }
.hash-entry code { overflow-wrap: anywhere; color: var(--text-primary); user-select: text; }
.metadata-editor:disabled { opacity: .65; }
.notes-field { grid-column: 1 / -1; }
.notes-input { width: 100%; resize: vertical; min-height: 65px; font: inherit; font-size: 13px; }

.game-card {
  padding: 14px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-width: 0;
  background-color: color-mix(in srgb, var(--surface) var(--card-opacity), transparent);
}

.game-card:hover {
  transform: none;
  box-shadow: 0 3px 12px var(--glass-shadow);
  border-color: #58685f;
}

.card-icon {
  font-size: 48px;
  text-align: center;
  opacity: 0.6;
  filter: drop-shadow(0 0 8px var(--primary-glow));
}

.card-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: center;
}

.card-archive {
  font-size: 11px;
  color: var(--text-tertiary);
  font-family: "Cascadia Mono", Consolas, monospace;
  text-align: center;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 元数据编辑区 */
.card-meta {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px 12px;
}

.meta-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.field-label {
  font-size: 12px;
  color: var(--text-secondary);
  font-weight: 500;
}

/* 版本号显示/编辑 */
.version-display {
  padding: 8px 12px;
  background: var(--surface-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all 0.2s ease;
  min-height: 36px;
  display: flex;
  align-items: center;
}

.version-display:hover {
  background: var(--surface-raised);
  border-color: var(--primary);
}

.version-text {
  font-family: "Cascadia Mono", Consolas, monospace;
  font-size: 13px;
  color: var(--text-primary);
}

.version-placeholder {
  font-size: 12px;
  color: var(--text-tertiary);
  font-style: italic;
}

.version-edit {
  position: relative;
}

.version-input {
  width: 100%;
  padding: 8px 12px;
  font-size: 13px;
  font-family: "Cascadia Mono", Consolas, monospace;
}

/* 引擎选择器 */
.engine-select {
  width: 100%;
  flex: 0 0 100%;
  font-size: 13px;
  font-weight: 600;
}

:deep(.engine-select .select-control) {
  min-height: 34px;
  padding: 6px 30px 6px 10px;
  border-color: inherit;
  color: inherit;
}

/* APK 复选框 */
.checkbox-field {
  display: flex;
  grid-column: 1 / -1;
  flex-direction: row !important;
  align-items: center;
  gap: 8px !important;
  padding: 7px 9px;
  background: var(--surface-input);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background 0.2s ease;
}

.checkbox-field:hover {
  background: var(--surface-raised);
}

.checkbox-field input[type="checkbox"] {
  width: 16px;
  height: 16px;
  cursor: pointer;
}

.checkbox-label {
  font-size: 13px;
  color: var(--text-primary);
}

/* 分隔线 */
.card-divider {
  height: 1px;
  background: var(--border-subtle);
}

/* 统计区 */
.card-stats {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.stat-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 12px;
}

.stat-label {
  color: var(--text-secondary);
}

.stat-value {
  color: var(--text-primary);
  font-weight: 600;
  font-family: "Cascadia Mono", Consolas, monospace;
}

/* 压缩率颜色 */
.ratio-good {
  color: var(--success) !important;
}

.ratio-ok {
  color: var(--warning) !important;
}

.ratio-poor {
  color: var(--error) !important;
}

/* 压缩率进度条 */
.ratio-bar {
  height: 4px;
  background: rgba(255, 255, 255, 0.1);
  border-radius: 2px;
  overflow: hidden;
  margin-top: 4px;
}

.ratio-fill {
  height: 100%;
  border-radius: 2px;
  transition: width 0.3s ease;
  box-shadow: none;
  animation: none;
}

/* 卡片底部 */
.card-footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 11px;
  color: var(--text-tertiary);
  margin-top: 4px;
  padding-top: 8px;
  border-top: 1px solid rgba(255, 255, 255, 0.05);
}

.footer-tag {
  color: var(--primary);
}

.footer-date {
  font-family: "Cascadia Mono", Consolas, monospace;
}
</style>
