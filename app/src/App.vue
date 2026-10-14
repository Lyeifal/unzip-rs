<script setup lang="ts">
// 导入新样式
import "./styles/themes.css";
import "./styles/glassmorphism.css";
import "./styles/animations.css";

import { onMounted, ref } from "vue";
import { useTheme } from "./composables/useTheme";
import UnpackView from "./components/UnpackView.vue";
import PackView from "./components/PackView.vue";
import ThemeSettings from "./components/ThemeSettings.vue";

// TODO: 等实现后导入
// import LibraryView from "./components/LibraryView.vue";

// 当前标签页
type TabType = "unpack" | "pack" | "library";
const tab = ref<TabType>("unpack");

// 主题设置面板
const showSettings = ref(false);

// 初始化主题
const theme = useTheme();
onMounted(() => {
  theme.init();
});
</script>

<template>
  <div class="app-container">
    <!-- 顶部标题栏 -->
    <div class="app-header">
      <h1 class="app-title">
        <span class="title-icon">🎮</span>
        自动解压工具
      </h1>
      <button class="btn-glass settings-btn" @click="showSettings = !showSettings">
        <span>⚙️</span>
      </button>
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
      <div v-else class="glass-card" style="padding: 40px; text-align: center">
        <h2>🚧 游戏库管理</h2>
        <p style="margin-top: 16px; color: var(--text-secondary)">
          即将推出...
        </p>
      </div>
    </KeepAlive>

    <!-- 主题设置面板 -->
    <ThemeSettings v-if="showSettings" @close="showSettings = false" />
  </div>
</template>

<style scoped>
.app-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 24px;
}

.app-title {
  font-size: 28px;
  font-weight: 700;
  color: var(--text-primary);
  display: flex;
  align-items: center;
  gap: 12px;
  margin: 0;
}

.title-icon {
  font-size: 32px;
  filter: drop-shadow(0 0 10px var(--primary-glow));
}

.settings-btn {
  padding: 10px 16px;
  font-size: 20px;
}
</style>

