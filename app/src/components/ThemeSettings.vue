<script setup lang="ts">
import { ref, computed } from "vue";
import { useTheme } from "../composables/useTheme";

const emit = defineEmits<{
  close: [];
}>();

const theme = useTheme();

// 文件上传
const fileInput = ref<HTMLInputElement | null>(null);
const isDragging = ref(false);

// 选择文件
function selectFile() {
  fileInput.value?.click();
}

// 处理文件选择
async function handleFileSelect(event: Event) {
  const target = event.target as HTMLInputElement;
  const file = target.files?.[0];
  if (file) {
    await uploadFile(file);
  }
}

// 上传文件
async function uploadFile(file: File) {
  try {
    await theme.uploadWallpaper(file);
  } catch (error) {
    console.error("上传失败：", error);
  }
}

// 拖拽上传
function handleDragOver(event: DragEvent) {
  event.preventDefault();
  isDragging.value = true;
}

function handleDragLeave() {
  isDragging.value = false;
}

async function handleDrop(event: DragEvent) {
  event.preventDefault();
  isDragging.value = false;

  const file = event.dataTransfer?.files[0];
  if (file && file.type.startsWith("image/")) {
    await uploadFile(file);
  }
}

// 配色预览
const paletteColors = computed(() => {
  if (!theme.config.value.palette) return [];
  const p = theme.config.value.palette;
  return [
    { label: "主色", color: p.primary },
    { label: "辅助色", color: p.secondary },
    { label: "强调色", color: p.accent },
  ];
});
</script>

<template>
  <div class="settings-overlay" @click="emit('close')">
    <div class="settings-panel glass-card" @click.stop>
      <!-- 标题栏 -->
      <div class="panel-header">
        <h2 class="panel-title">⚙️ 主题设置</h2>
        <button class="close-btn" @click="emit('close')">✕</button>
      </div>

      <div class="panel-content">
        <!-- 壁纸上传区域 -->
        <section class="section">
          <h3 class="section-title">背景壁纸</h3>

          <!-- 当前壁纸预览 -->
          <div
            v-if="theme.config.value.enabled && theme.config.value.imageData"
            class="wallpaper-preview"
          >
            <img
              :src="theme.config.value.imageData"
              alt="当前壁纸"
              class="preview-image"
            />
            <button class="remove-btn btn-glass" @click="theme.clearWallpaper()">
              🗑️ 移除壁纸
            </button>
          </div>

          <!-- 上传区域 -->
          <div
            v-else
            class="upload-zone"
            :class="{ dragging: isDragging }"
            @click="selectFile"
            @dragover="handleDragOver"
            @dragleave="handleDragLeave"
            @drop="handleDrop"
          >
            <div class="upload-icon">📁</div>
            <p class="upload-text">点击或拖拽图片到此处</p>
            <p class="upload-hint">支持 JPG、PNG、WebP，限制 5MB</p>
          </div>

          <input
            ref="fileInput"
            type="file"
            accept="image/*"
            style="display: none"
            @change="handleFileSelect"
          />

          <!-- 加载状态 -->
          <div v-if="theme.isLoading.value" class="loading-indicator">
            <div class="spinner"></div>
            <span>处理中...</span>
          </div>

          <!-- 错误提示 -->
          <div v-if="theme.error.value" class="error-message">
            {{ theme.error.value }}
          </div>
        </section>

        <!-- 自动配色 -->
        <section
          v-if="theme.config.value.enabled"
          class="section"
        >
          <h3 class="section-title">自动配色</h3>

          <label class="checkbox-label">
            <input
              type="checkbox"
              :checked="theme.config.value.autoTheme"
              @change="
                theme.toggleAutoTheme(
                  ($event.target as HTMLInputElement).checked
                )
              "
            />
            <span>从壁纸自动提取颜色</span>
          </label>

          <!-- 配色预览 -->
          <div
            v-if="
              theme.config.value.autoTheme && theme.config.value.palette
            "
            class="palette-preview"
          >
            <p class="palette-title">提取的配色方案：</p>
            <div class="palette-grid">
              <div
                v-for="item in paletteColors"
                :key="item.label"
                class="palette-item"
              >
                <div
                  class="color-swatch"
                  :style="{ backgroundColor: item.color }"
                ></div>
                <div class="color-info">
                  <span class="color-label">{{ item.label }}</span>
                  <span class="color-hex">{{ item.color }}</span>
                </div>
              </div>
            </div>

            <button
              class="btn-glass reextract-btn"
              @click="theme.reextractColors()"
              :disabled="theme.isLoading.value"
            >
              🔄 重新提取
            </button>
          </div>
        </section>

        <!-- 高级选项 -->
        <section
          v-if="theme.config.value.enabled"
          class="section"
        >
          <h3 class="section-title">高级选项</h3>

          <!-- 模糊程度 -->
          <div class="slider-control">
            <label class="slider-label">
              <span>模糊程度</span>
              <span class="slider-value"
                >{{ theme.config.value.blur }}px</span
              >
            </label>
            <input
              type="range"
              min="0"
              max="20"
              step="1"
              :value="theme.config.value.blur"
              @input="
                theme.updateBlur(
                  parseInt(($event.target as HTMLInputElement).value)
                )
              "
              class="slider"
            />
          </div>

          <!-- 暗度遮罩 -->
          <div class="slider-control">
            <label class="slider-label">
              <span>暗度遮罩</span>
              <span class="slider-value"
                >{{ Math.round(theme.config.value.darkness * 100) }}%</span
              >
            </label>
            <input
              type="range"
              min="0"
              max="1"
              step="0.05"
              :value="theme.config.value.darkness"
              @input="
                theme.updateDarkness(
                  parseFloat(($event.target as HTMLInputElement).value)
                )
              "
              class="slider"
            />
          </div>
        </section>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-overlay {
  position: fixed;
  inset: 0;
  z-index: 1000;
  background: rgba(0, 0, 0, 0.6);
  backdrop-filter: blur(8px);
  display: flex;
  align-items: center;
  justify-content: center;
  animation: fadeIn 0.3s ease;
}

.settings-panel {
  width: 90%;
  max-width: 600px;
  max-height: 90vh;
  overflow-y: auto;
  animation: scaleIn 0.3s ease;
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 24px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.1);
}

.panel-title {
  font-size: 24px;
  font-weight: 700;
  color: var(--text-primary);
  margin: 0;
}

.close-btn {
  background: rgba(255, 255, 255, 0.1);
  border: none;
  border-radius: 8px;
  width: 32px;
  height: 32px;
  color: var(--text-primary);
  font-size: 20px;
  cursor: pointer;
  transition: all 0.2s ease;
}

.close-btn:hover {
  background: rgba(255, 255, 255, 0.2);
  transform: rotate(90deg);
}

.panel-content {
  padding: 24px;
}

.section {
  margin-bottom: 32px;
}

.section:last-child {
  margin-bottom: 0;
}

.section-title {
  font-size: 16px;
  font-weight: 600;
  color: var(--primary);
  margin: 0 0 16px;
}

/* 壁纸预览 */
.wallpaper-preview {
  position: relative;
  border-radius: var(--radius-md);
  overflow: hidden;
  margin-bottom: 16px;
}

.preview-image {
  width: 100%;
  height: 200px;
  object-fit: cover;
  display: block;
}

.remove-btn {
  position: absolute;
  top: 12px;
  right: 12px;
  padding: 8px 16px;
  font-size: 13px;
}

/* 上传区域 */
.upload-zone {
  background: rgba(255, 255, 255, 0.03);
  border: 2px dashed rgba(255, 255, 255, 0.2);
  border-radius: var(--radius-md);
  padding: 48px 24px;
  text-align: center;
  cursor: pointer;
  transition: all 0.3s ease;
}

.upload-zone:hover,
.upload-zone.dragging {
  background: rgba(255, 255, 255, 0.08);
  border-color: var(--primary);
  box-shadow: 0 0 20px var(--primary-glow);
}

.upload-icon {
  font-size: 48px;
  margin-bottom: 16px;
  opacity: 0.6;
}

.upload-text {
  font-size: 16px;
  color: var(--text-primary);
  margin: 0 0 8px;
}

.upload-hint {
  font-size: 13px;
  color: var(--text-tertiary);
  margin: 0;
}

/* 加载指示器 */
.loading-indicator {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
  padding: 16px;
  background: rgba(255, 255, 255, 0.05);
  border-radius: var(--radius-md);
  margin-top: 16px;
}

.spinner {
  width: 20px;
  height: 20px;
  border: 2px solid rgba(255, 255, 255, 0.2);
  border-top-color: var(--primary);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

/* 错误提示 */
.error-message {
  padding: 12px;
  background: rgba(255, 55, 95, 0.1);
  border: 1px solid var(--error);
  border-radius: var(--radius-md);
  color: var(--error);
  margin-top: 16px;
  font-size: 13px;
}

/* 复选框 */
.checkbox-label {
  display: flex;
  align-items: center;
  gap: 10px;
  cursor: pointer;
  padding: 12px;
  background: rgba(255, 255, 255, 0.03);
  border-radius: var(--radius-md);
  transition: background 0.2s ease;
}

.checkbox-label:hover {
  background: rgba(255, 255, 255, 0.06);
}

.checkbox-label input[type="checkbox"] {
  width: 18px;
  height: 18px;
  cursor: pointer;
}

/* 配色预览 */
.palette-preview {
  margin-top: 16px;
  padding: 16px;
  background: rgba(255, 255, 255, 0.03);
  border-radius: var(--radius-md);
}

.palette-title {
  font-size: 13px;
  color: var(--text-secondary);
  margin: 0 0 12px;
}

.palette-grid {
  display: grid;
  gap: 12px;
  margin-bottom: 16px;
}

.palette-item {
  display: flex;
  align-items: center;
  gap: 12px;
}

.color-swatch {
  width: 48px;
  height: 48px;
  border-radius: var(--radius-sm);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3), inset 0 1px 0 rgba(255, 255, 255, 0.2);
}

.color-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.color-label {
  font-size: 14px;
  color: var(--text-primary);
}

.color-hex {
  font-size: 12px;
  font-family: "Cascadia Mono", Consolas, monospace;
  color: var(--text-tertiary);
}

.reextract-btn {
  width: 100%;
  padding: 10px;
}

/* 滑块控制 */
.slider-control {
  margin-bottom: 20px;
}

.slider-control:last-child {
  margin-bottom: 0;
}

.slider-label {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 8px;
  font-size: 14px;
  color: var(--text-primary);
}

.slider-value {
  font-family: "Cascadia Mono", Consolas, monospace;
  color: var(--primary);
  font-weight: 600;
}

.slider {
  width: 100%;
  height: 6px;
  background: rgba(255, 255, 255, 0.1);
  border-radius: 3px;
  outline: none;
  cursor: pointer;
  -webkit-appearance: none;
}

.slider::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 18px;
  height: 18px;
  background: linear-gradient(135deg, var(--primary), var(--primary-dark));
  border-radius: 50%;
  cursor: pointer;
  box-shadow: 0 2px 8px var(--primary-glow);
  transition: transform 0.2s ease;
}

.slider::-webkit-slider-thumb:hover {
  transform: scale(1.2);
}

.slider::-moz-range-thumb {
  width: 18px;
  height: 18px;
  background: linear-gradient(135deg, var(--primary), var(--primary-dark));
  border-radius: 50%;
  cursor: pointer;
  border: none;
  box-shadow: 0 2px 8px var(--primary-glow);
  transition: transform 0.2s ease;
}

.slider::-moz-range-thumb:hover {
  transform: scale(1.2);
}
</style>
