<script setup lang="ts">
// 导入新样式
import "./styles/themes.css";
import "./styles/glassmorphism.css";
import "./styles/animations.css";

import { onMounted, onUnmounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTheme } from "./composables/useTheme";
import UnpackView from "./components/UnpackView.vue";
import PackView from "./components/PackView.vue";
import LibraryView from "./components/LibraryView.vue";
import ThemeSettings from "./components/ThemeSettings.vue";

// 当前标签页
type TabType = "unpack" | "pack" | "library";
const tab = ref<TabType>("unpack");

// 主题设置面板
const showSettings = ref(false);
const currentWindow = getCurrentWindow();
const isMaximized = ref(false);
const windowActionError = ref("");
let unlistenResize: (() => void) | undefined;
let scrollHideTimer: ReturnType<typeof setTimeout> | undefined;

function revealPageScrollbar() {
  document.documentElement.classList.add("is-scrolling");
  document.body.classList.add("is-scrolling");
  if (scrollHideTimer) clearTimeout(scrollHideTimer);
  scrollHideTimer = setTimeout(() => {
    document.documentElement.classList.remove("is-scrolling");
    document.body.classList.remove("is-scrolling");
  }, 850);
}

async function runWindowAction(action: () => Promise<void>) {
  try {
    await action();
    windowActionError.value = "";
    isMaximized.value = await currentWindow.isMaximized();
  } catch (error) {
    console.error("窗口操作失败", error);
    windowActionError.value = error instanceof Error ? error.message : "窗口操作失败";
  }
}

async function toggleMaximize() {
  await runWindowAction(() => currentWindow.toggleMaximize());
}

// 初始化主题
const theme = useTheme();
onMounted(async () => {
  theme.init();
  window.addEventListener("scroll", revealPageScrollbar, { passive: true });
  document.body.addEventListener("scroll", revealPageScrollbar, { passive: true });
  isMaximized.value = await currentWindow.isMaximized().catch(() => false);
  unlistenResize = await currentWindow.onResized(() => {
    currentWindow.isMaximized().then((value) => (isMaximized.value = value));
  });
});
onUnmounted(() => {
  unlistenResize?.();
  window.removeEventListener("scroll", revealPageScrollbar);
  document.body.removeEventListener("scroll", revealPageScrollbar);
  if (scrollHideTimer) clearTimeout(scrollHideTimer);
});
</script>

<template>
  <div class="app-container">
    <!-- 顶部标题栏 -->
    <div class="app-header">
      <h1 class="app-title" data-tauri-drag-region>
        <img class="title-icon" src="/app-icon.png" alt="" />
        自动解压工具
      </h1>
      <div class="header-drag-region" data-tauri-drag-region></div>
      <div class="header-actions">
        <span v-if="windowActionError" class="window-action-error" role="status">{{ windowActionError }}</span>
        <button
          class="settings-btn"
          title="主题设置"
          aria-label="主题设置"
          @click="showSettings = !showSettings"
        >
          <span>⚙</span>
        </button>
        <div class="window-controls">
          <button title="最小化" aria-label="最小化" @click.stop="runWindowAction(() => currentWindow.minimize())">
            <span class="minimize-icon" aria-hidden="true"></span>
          </button>
          <button :title="isMaximized ? '还原' : '最大化'" :aria-label="isMaximized ? '还原' : '最大化'" @click.stop="toggleMaximize">
            <span :class="['maximize-icon', { restored: isMaximized }]" aria-hidden="true"></span>
          </button>
          <button class="close-control" title="关闭" aria-label="关闭" @click.stop="runWindowAction(() => currentWindow.close())">
            <span class="close-icon" aria-hidden="true"></span>
          </button>
        </div>
      </div>
    </div>

    <!-- 标签页（毛玻璃风格） -->
    <div class="tabs-glass">
      <button
        class="tab-glass"
        :class="{ active: tab === 'unpack' }"
        @click="tab = 'unpack'"
      >
        📦 自动解压
      </button>
      <button
        class="tab-glass"
        :class="{ active: tab === 'pack' }"
        @click="tab = 'pack'"
      >
        🗜️ 打包压缩
      </button>
      <button
        class="tab-glass"
        :class="{ active: tab === 'library' }"
        @click="tab = 'library'"
      >
        📚 游戏库管理
      </button>
    </div>

    <!-- 页面内容（KeepAlive 保持状态） -->
    <KeepAlive>
      <UnpackView v-if="tab === 'unpack'" />
      <PackView v-else-if="tab === 'pack'" />
      <LibraryView v-else-if="tab === 'library'" />
    </KeepAlive>

    <!-- 主题设置面板 -->
    <ThemeSettings v-if="showSettings" @close="showSettings = false" />
  </div>
</template>

<style scoped>
.app-header {
  position: sticky;
  top: 0;
  z-index: 80;
  display: flex;
  align-items: center;
  justify-content: space-between;
  min-height: 46px;
  margin-bottom: 14px;
  padding: 0 4px;
  background: transparent;
  box-shadow: none;
  backdrop-filter: none;
  -webkit-backdrop-filter: none;
  border-bottom: 0;
  user-select: none;
}

.header-drag-region {
  align-self: stretch;
  flex: 1;
  min-width: 16px;
}

.app-title {
  font-size: 16px;
  font-weight: 650;
  color: var(--text-primary);
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0;
}

.title-icon {
  width: 24px;
  height: 24px;
  object-fit: contain;
  border-radius: 8px;
}

.settings-btn {
  display: grid;
  place-items: center;
  min-width: 34px;
  width: 34px;
  height: 30px;
  padding: 0;
  background: transparent;
  border: 1px solid transparent;
  border-radius: 5px;
  color: var(--text-secondary);
  font-size: 17px;
  cursor: pointer;
}

.settings-btn:hover {
  background: var(--surface-raised);
  border-color: var(--border);
  color: var(--text-primary);
}

.header-actions,
.window-controls {
  display: flex;
  align-items: center;
}

.header-actions {
  gap: 5px;
}

.window-controls {
  gap: 2px;
  padding-left: 5px;
  border-left: 1px solid var(--border-subtle);
}

.window-controls button {
  position: relative;
  z-index: 1;
  display: grid;
  place-items: center;
  width: 36px;
  height: 30px;
  padding: 0;
  background: transparent;
  border: 1px solid transparent;
  border-radius: 5px;
  color: var(--text-primary);
  cursor: pointer;
}

.window-controls button:hover {
  background: var(--surface-raised);
  border-color: var(--border);
  color: var(--text-primary);
}

.window-controls .close-control:hover {
  background: #8f4d49;
  border-color: #a46059;
  color: #fff;
}

.minimize-icon {
  width: 10px;
  height: 1px;
  background: currentColor;
}

.maximize-icon {
  width: 10px;
  height: 10px;
  border: 1px solid currentColor;
}

.maximize-icon.restored {
  position: relative;
  width: 9px;
  height: 9px;
  margin: 2px 0 0 2px;
  border-width: 1px;
}

.maximize-icon.restored::before {
  position: absolute;
  top: -4px;
  left: -4px;
  width: 7px;
  height: 7px;
  border: 1px solid currentColor;
  background: var(--bg-dark);
  content: "";
}

.close-icon,
.close-icon::after {
  width: 12px;
  height: 1px;
  background: currentColor;
}

.close-icon {
  transform: rotate(45deg);
}

.close-icon::after {
  display: block;
  transform: rotate(90deg);
  content: "";
}

.window-action-error {
  max-width: 240px;
  overflow: hidden;
  color: var(--error);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>

