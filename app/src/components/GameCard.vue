<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import { ENGINE_COLORS, type GameMapping, type GameMetadata } from "../types";
import StyledSelect from "./StyledSelect.vue";
import type { SelectOption } from "../types/select";

const props = defineProps<{
  mapping: GameMapping;
}>();

const engineOptions: SelectOption[] = [
  { value: "", label: "未标记" },
  { value: "Unity", label: "Unity" },
  { value: "Unreal", label: "Unreal" },
  { value: "RPG Maker", label: "RPG Maker" },
  { value: "Godot", label: "Godot" },
  { value: "自研", label: "自研" },
  { value: "其他", label: "其他" },
];

const emit = defineEmits<{
  "update:metadata": [metadata: GameMetadata];
}>();

// 初始化 metadata（兼容旧数据）
const metadata = reactive<GameMetadata>({
  version: props.mapping.metadata?.version || "",
  engine: props.mapping.metadata?.engine || "",
  is_apk: props.mapping.metadata?.is_apk || false,
  custom_tags: props.mapping.metadata?.custom_tags || [],
  notes: props.mapping.metadata?.notes || "",
});

// 编辑状态
const editingVersion = ref(false);
const versionInput = ref(metadata.version);

// 防抖保存
let saveTimer: number | null = null;
function saveMetadata() {
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    emit("update:metadata", { ...metadata });
  }, 500) as unknown as number;
}

// 版本号编辑
function startEditVersion() {
  editingVersion.value = true;
  versionInput.value = metadata.version;
  setTimeout(() => {
    const input = document.querySelector(".version-input") as HTMLInputElement;
    input?.focus();
  }, 50);
}

function saveVersion() {
  metadata.version = versionInput.value.trim();
  editingVersion.value = false;
  saveMetadata();
}

function cancelVersion() {
  editingVersion.value = false;
  versionInput.value = metadata.version;
}

// 引擎选择变更
function handleEngineChange() {
  saveMetadata();
}

// APK 复选框变更
function handleApkChange() {
  saveMetadata();
}

// 压缩率颜色
const ratioClass = computed(() => {
  const ratio = parseFloat(props.mapping.compression_ratio);
  if (ratio <= 60) return "ratio-good";
  if (ratio <= 80) return "ratio-ok";
  return "ratio-poor";
});

const ratioColor = computed(() => {
  const ratio = parseFloat(props.mapping.compression_ratio);
  if (ratio <= 60) return "var(--success)";
  if (ratio <= 80) return "var(--warning)";
  return "var(--error)";
});

// 引擎标签颜色
const engineColor = computed(() => {
  return ENGINE_COLORS[metadata.engine] || ENGINE_COLORS[""];
});
</script>

<template>
  <div class="game-card glass-card card-enter">
    <!-- 图标区 -->
    <div class="card-icon">🎮</div>

    <!-- 标题区 -->
    <div class="card-title" :title="mapping.source_name">
      {{ mapping.source_name }}
    </div>
    <div class="card-archive">{{ mapping.archive_name }}</div>

    <!-- 元数据编辑区 -->
    <div class="card-meta">
      <!-- 版本号 -->
      <div class="meta-field">
        <span class="field-label">版本</span>
        <div v-if="!editingVersion" class="version-display" @click="startEditVersion">
          <span v-if="metadata.version" class="version-text">{{ metadata.version }}</span>
          <span v-else class="version-placeholder">点击输入</span>
        </div>
        <div v-else class="version-edit">
          <input
            v-model="versionInput"
            type="text"
            class="version-input input-glass"
            placeholder="如 v1.2.3"
            @keyup.enter="saveVersion"
            @keyup.esc="cancelVersion"
            @blur="saveVersion"
          />
        </div>
      </div>

      <!-- 引擎选择 -->
      <div class="meta-field">
        <span class="field-label">引擎</span>
        <StyledSelect
          v-model="metadata.engine"
          class="engine-select select-glass"
          :options="engineOptions"
          aria-label="游戏引擎"
          :style="{ borderColor: engineColor, color: engineColor }"
          @update:model-value="handleEngineChange"
        />
      </div>

      <!-- APK 复选框 -->
      <label class="meta-field checkbox-field">
        <input
          v-model="metadata.is_apk"
          type="checkbox"
          @change="handleApkChange"
        />
        <span class="checkbox-label">📱 APK 安装包</span>
      </label>
    </div>

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
            width: mapping.compression_ratio,
            background: `linear-gradient(90deg, ${ratioColor}, ${ratioColor}dd)`,
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
