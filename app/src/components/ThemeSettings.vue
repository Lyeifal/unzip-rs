<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { useTheme, type ManualThemeColors } from "../composables/useTheme";
import { THEME_PRESETS, type ThemePresetId } from "../utils/themePresets";
import LibraryFieldsSettings from './LibraryFieldsSettings.vue';
import ShellIntegrationSettings from './ShellIntegrationSettings.vue';
import ToolPathsSettings from './ToolPathsSettings.vue';
import UpdateSettings from './UpdateSettings.vue';

const emit = defineEmits<{
  close: [];
}>();
const props = defineProps<{ initialSection?: string }>();
const panel = ref<HTMLElement | null>(null);
const previousFocus = document.activeElement as HTMLElement | null;
const previousOverflow = document.body.style.overflow;
function handleDialogKey(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    if (event.defaultPrevented) return;
    event.preventDefault();
    emit('close');
  } else if (event.key === 'Tab') {
    const focusable = [...(panel.value?.querySelectorAll<HTMLElement>('button, input, select, textarea, [tabindex]') ?? [])]
      .filter(element => !element.matches(':disabled, [tabindex="-1"]') && element.getClientRects().length > 0);
    const first = focusable[0], last = focusable[focusable.length - 1];
    if (event.shiftKey && (document.activeElement === first || !panel.value?.contains(document.activeElement))) {
      event.preventDefault(); last?.focus();
    } else if (!event.shiftKey && (document.activeElement === last || !panel.value?.contains(document.activeElement))) {
      event.preventDefault(); first?.focus();
    }
  }
}
onMounted(() => {
  document.body.style.overflow = 'hidden';
  panel.value?.querySelector<HTMLButtonElement>('.close-btn')?.focus({ preventScroll: true });
  if (props.initialSection) goToSection(props.initialSection, 'instant');
  document.addEventListener('keydown', handleDialogKey);
});
onUnmounted(() => {
  document.body.style.overflow = previousOverflow;
  document.removeEventListener('keydown', handleDialogKey);
  previousFocus?.focus({ preventScroll: true });
});

const theme = useTheme();
const activeSection = ref('updates');
const settingSections = [
  { id: 'updates', label: '应用更新' },
  { id: 'tools', label: '工具路径' },
  { id: 'library', label: '压缩包管理' },
  { id: 'shell', label: '系统集成' },
  { id: 'presets', label: '主题预设' },
  { id: 'wallpaper', label: '背景壁纸' },
  { id: 'manual', label: '手动配色' },
  { id: 'automatic', label: '自动配色' },
  { id: 'appearance', label: '界面外观' },
  { id: 'advanced', label: '高级选项' },
];
function goToSection(id: string, behavior: ScrollBehavior = 'smooth') {
  activeSection.value = id;
  document.getElementById(`settings-${id}`)?.scrollIntoView({ behavior, block: 'start' });
}

// 文件上传
const fileInput = ref<HTMLInputElement | null>(null);
const isDragging = ref(false);
const pickerError = ref("");

type ManualColorKey = keyof ManualThemeColors;
type EyeDropperCtor = new () => { open: () => Promise<{ sRGBHex: string }> };

const manualColorOptions: { key: ManualColorKey; label: string }[] = [
  { key: "primary", label: "主色" },
  { key: "secondary", label: "辅助色" },
  { key: "accent", label: "强调色" },
  { key: "background", label: "背景色" },
];

// 主题预设
const presets = THEME_PRESETS;

function isPresetActive(id: ThemePresetId): boolean {
  return (
    theme.config.value.themeMode === "preset" &&
    theme.config.value.presetId === id
  );
}

function isImageFile(file: File): boolean {
  return file.type.startsWith("image/") || /\.avif$/i.test(file.name);
}

function updateManualColor(key: ManualColorKey, event: Event) {
  const color = (event.target as HTMLInputElement).value;
  pickerError.value = "";
  theme.updateManualColor(key, color);
}

async function pickColor(key: ManualColorKey) {
  const EyeDropper = (window as Window & { EyeDropper?: EyeDropperCtor }).EyeDropper;
  if (!EyeDropper) {
    pickerError.value = "当前 WebView 暂不支持屏幕吸色，请用颜色块手动选择。";
    return;
  }

  pickerError.value = "";
  try {
    const { sRGBHex } = await new EyeDropper().open();
    theme.updateManualColor(key, sRGBHex);
  } catch (error) {
    if ((error as DOMException)?.name !== "AbortError") {
      pickerError.value = error instanceof Error ? error.message : "吸色失败";
    }
  }
}

function updateCardTransparency(event: Event) {
  const transparency = Number((event.target as HTMLInputElement).value) / 100;
  theme.updateCardOpacity(1 - transparency);
}

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
  target.value = "";
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
  if (file && isImageFile(file)) {
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
    <div ref="panel" class="settings-panel glass-card" role="dialog" aria-modal="true" aria-labelledby="settings-title" @click.stop>
      <!-- 标题栏 -->
      <div class="panel-header">
        <h2 id="settings-title" class="panel-title">⚙️ 应用设置</h2>
        <button class="close-btn" aria-label="关闭应用设置" @click="emit('close')">✕</button>
      </div>

      <div class="settings-body">
        <nav class="settings-sidebar" aria-label="设置导航">
          <button v-for="item in settingSections" v-show="theme.config.value.enabled || (item.id !== 'automatic' && item.id !== 'advanced')" :key="item.id" type="button" class="sidebar-link" :class="{ active: activeSection === item.id }" :aria-current="activeSection === item.id ? 'location' : undefined" @click="goToSection(item.id)">{{ item.label }}</button>
        </nav>
      <div class="panel-content">
        <div id="settings-updates" class="section"><UpdateSettings /></div>
        <div id="settings-tools" class="section"><ToolPathsSettings /></div>
        <div id="settings-library" class="section"><LibraryFieldsSettings /></div>
        <div id="settings-shell" class="section"><ShellIntegrationSettings /></div>
        <!-- 主题预设 -->
        <section id="settings-presets" class="section">
          <h3 class="section-title">主题预设</h3>
          <p class="section-hint">配色预设可与壁纸和卡片透明度搭配使用。</p>
          <div class="preset-gallery">
            <button
              v-for="preset in presets"
              :key="preset.id"
              class="preset-card"
              :class="{ active: isPresetActive(preset.id) }"
              :aria-pressed="isPresetActive(preset.id)"
              type="button"
              @click="theme.selectPreset(preset.id)"
            >
              <span class="preset-swatches">
                <span
                  v-for="(color, index) in preset.swatches"
                  :key="index"
                  class="preset-swatch"
                  :style="{ backgroundColor: color }"
                ></span>
              </span>
              <span class="preset-name">{{ preset.label }}</span>
              <span class="preset-desc">{{ preset.description }}</span>
            </button>
          </div>
        </section>

        <!-- 壁纸上传区域 -->
        <section id="settings-wallpaper" class="section">
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
            <button class="replace-btn btn-glass" @click="selectFile">
              🖼️ 更换壁纸
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
            <p class="upload-hint">支持 JPG、PNG、WebP、AVIF，限制 5MB</p>
          </div>

          <input
            ref="fileInput"
            type="file"
            accept="image/*,.avif"
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

        <section id="settings-manual" class="section">
          <h3 class="section-title">手动配色</h3>
          <p class="section-hint">可以直接选颜色，也可以吸取屏幕上预览图的颜色。</p>
          <p v-if="theme.config.value.themeMode !== 'manual'" class="section-hint mode-hint">
            修改任意颜色将自动切换到手动配色模式。
          </p>
          <div class="manual-color-grid">
            <div v-for="item in manualColorOptions" :key="item.key" class="manual-color-item">
              <label :for="`manual-color-${item.key}`">{{ item.label }}</label>
              <div class="manual-color-control">
                <input
                  :id="`manual-color-${item.key}`"
                  type="color"
                  :value="theme.config.value.manualColors[item.key]"
                  @input="updateManualColor(item.key, $event)"
                />
                <code>{{ theme.config.value.manualColors[item.key] }}</code>
                <button class="btn-glass eyedropper-btn" @click="pickColor(item.key)">吸取</button>
              </div>
            </div>
          </div>
          <p v-if="pickerError" class="error-message picker-error">{{ pickerError }}</p>
        </section>

        <!-- 自动配色 -->
        <section
          v-if="theme.config.value.enabled"
          id="settings-automatic"
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
          <div v-if="theme.config.value.autoTheme" class="palette-preview">
            <template v-if="theme.config.value.palette">
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
            </template>
            <p v-else class="palette-title">
              尚未从壁纸提取到配色，可点击重试。
            </p>

            <button
              class="btn-glass reextract-btn"
              @click="theme.reextractColors()"
              :disabled="theme.isLoading.value"
            >
              🔄 重新提取
            </button>
          </div>
        </section>

        <section id="settings-appearance" class="section appearance-section">
          <h3 class="section-title">界面外观</h3>
          <label class="checkbox-label">
            <input type="checkbox" :checked="theme.config.value.titlebarBackground" @change="theme.updateTitlebarBackground(($event.target as HTMLInputElement).checked)" />
            <span>显示标题栏底色</span>
          </label>
          <p class="section-hint titlebar-hint">为软件名称和窗口按钮所在栏添加主题底色；关闭时保持透明。</p>
          <div class="slider-control">
            <label class="slider-label">
              <span>卡片透明度</span>
              <span class="slider-value">{{ Math.round((1 - theme.config.value.cardOpacity) * 100) }}%</span>
            </label>
            <input
              type="range"
              min="0"
              max="45"
              step="5"
              :value="Math.round((1 - theme.config.value.cardOpacity) * 100 / 5) * 5"
              @input="updateCardTransparency"
              class="slider"
            />
            <p class="section-hint">只调整卡片表面，文字保持清晰。</p>
          </div>
        </section>

        <!-- 高级选项 -->
        <section
          v-if="theme.config.value.enabled"
          id="settings-advanced"
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
  max-width: 900px;
  height: 90vh;
  max-height: 90vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  animation: scaleIn 0.3s ease;
}
.settings-body { display: grid; grid-template-columns: 170px minmax(0, 1fr); flex: 1; min-height: 0; }
.settings-sidebar { display: flex; flex-direction: column; gap: 4px; padding: 16px 10px; border-right: 1px solid var(--border-subtle); overflow-y: auto; }
.sidebar-link { width: 100%; padding: 10px 12px; border: 0; border-radius: var(--radius-md); background: transparent; color: var(--text-secondary); text-align: left; cursor: pointer; }
.sidebar-link:hover, .sidebar-link.active { background: var(--surface-input); color: var(--text-primary); }
.sidebar-link.active { box-shadow: inset 3px 0 var(--primary); }

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 24px;
  border-bottom: 1px solid var(--border-subtle);
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
  min-height: 0;
  overflow-y: auto;
}
.panel-content > .section { scroll-margin-top: 16px; }

.section {
  margin-bottom: 32px;
}

.section:last-child {
  margin-bottom: 0;
}

.section-hint {
  margin: -7px 0 12px;
  color: var(--text-tertiary);
  font-size: 12px;
}
.titlebar-hint { margin-top: 8px; }

.manual-color-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}

.manual-color-item {
  min-width: 0;
  padding: 10px;
  background: color-mix(in srgb, var(--surface-input) 75%, transparent);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
}

.manual-color-item > label {
  display: block;
  margin-bottom: 7px;
  color: var(--text-secondary);
  font-size: 12px;
}

.manual-color-control {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
}

.manual-color-control input[type="color"] {
  flex: 0 0 34px;
  width: 34px;
  height: 32px;
  padding: 2px;
  background: var(--surface-raised);
  border: 1px solid var(--border);
  border-radius: 6px;
  cursor: pointer;
}

.manual-color-control code {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  color: var(--text-primary);
  font-size: 11px;
  text-overflow: ellipsis;
}

.eyedropper-btn {
  min-height: 30px;
  padding: 4px 8px;
  font-size: 12px;
}

.picker-error {
  margin-top: 10px;
}

.section-title {
  font-size: 16px;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0 0 16px;
}

.mode-hint {
  color: var(--warning);
}

/* 主题预设选择 */
.preset-gallery {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
  gap: 10px;
}

.preset-card {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 8px;
  padding: 10px;
  background: rgba(255, 255, 255, 0.03);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  cursor: pointer;
  text-align: left;
  transition: border-color 0.2s ease, background 0.2s ease, box-shadow 0.2s ease;
}

.preset-card:hover {
  background: rgba(255, 255, 255, 0.06);
  border-color: var(--border);
}

.preset-card.active {
  border-color: var(--primary);
  box-shadow: 0 0 0 1px var(--primary), 0 0 12px var(--primary-glow);
}

.preset-swatches {
  display: flex;
  gap: 4px;
}

.preset-swatch {
  flex: 1;
  height: 22px;
  border-radius: var(--radius-sm);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.15), 0 1px 3px rgba(0, 0, 0, 0.25);
}

.preset-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-primary);
}

.preset-desc {
  font-size: 11px;
  line-height: 1.4;
  color: var(--text-tertiary);
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

.replace-btn {
  position: absolute;
  bottom: 12px;
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
  background: var(--primary);
  border-radius: 50%;
  cursor: pointer;
  box-shadow: 0 1px 3px var(--glass-shadow);
  transition: transform 0.2s ease;
}

.slider::-webkit-slider-thumb:hover {
  transform: scale(1.2);
}

.slider::-moz-range-thumb {
  width: 18px;
  height: 18px;
  background: var(--primary);
  border-radius: 50%;
  cursor: pointer;
  border: none;
  box-shadow: 0 1px 3px var(--glass-shadow);
  transition: transform 0.2s ease;
}

.slider::-moz-range-thumb:hover {
  transform: scale(1.2);
}

@media (max-width: 520px) {
  .settings-body { grid-template-columns: 1fr; grid-template-rows: auto minmax(0, 1fr); }
  .settings-sidebar { flex-direction: row; overflow-x: auto; overflow-y: hidden; padding: 8px; border-right: 0; border-bottom: 1px solid var(--border-subtle); }
  .sidebar-link { width: auto; flex: 0 0 auto; white-space: nowrap; }
  .manual-color-grid {
    grid-template-columns: 1fr;
  }
}
</style>
