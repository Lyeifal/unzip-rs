<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import GameCard from "./GameCard.vue";
import type { GameMapping, GameMetadata, PackMappingFile } from "../types";

// 状态
const outputDir = ref("");
const mappings = ref<GameMapping[]>([]);
const loading = ref(false);
const error = ref("");
const searchQuery = ref("");
const filterEngine = ref("");
const sortBy = ref<"name" | "size" | "ratio" | "date">("name");

// 加载 JSON 文件
async function loadMappingFile() {
  if (!outputDir.value.trim()) {
    error.value = "请先选择输出目录";
    return;
  }

  loading.value = true;
  error.value = "";

  try {
    const content = await invoke<string>("load_mapping_file", {
      dir: outputDir.value,
    });

    const data: PackMappingFile = JSON.parse(content);
    mappings.value = data.mappings;

    // 记住目录
    localStorage.setItem("library_output_dir", outputDir.value);
  } catch (e) {
    error.value = `加载失败：${e}`;
    mappings.value = [];
  } finally {
    loading.value = false;
  }
}

// 保存 JSON 文件
async function saveMappingFile() {
  if (!outputDir.value.trim() || mappings.value.length === 0) {
    return;
  }

  try {
    const data: PackMappingFile = {
      generated_at: new Date().toISOString().split("T")[0],
      total_count: mappings.value.length,
      mappings: mappings.value,
    };

    await invoke("save_mapping_file", {
      dir: outputDir.value,
      content: JSON.stringify(data, null, 2),
    });
  } catch (e) {
    console.error("保存失败：", e);
    error.value = `保存失败：${e}`;
  }
}

// 选择目录
async function browseDir() {
  try {
    const picked = await invoke<string | null>("pick_directory", {
      current: outputDir.value,
    });
    if (picked) {
      outputDir.value = picked;
      await loadMappingFile();
    }
  } catch (e) {
    error.value = `选择目录失败：${e}`;
  }
}

// 更新元数据
function updateMetadata(index: number, metadata: GameMetadata) {
  if (index >= 0 && index < mappings.value.length) {
    mappings.value[index].metadata = metadata;
    saveMappingFile();
  }
}

// 筛选和搜索
const filteredMappings = computed(() => {
  let result = mappings.value;

  // 搜索过滤
  if (searchQuery.value.trim()) {
    const query = searchQuery.value.toLowerCase();
    result = result.filter(
      (m) =>
        m.source_name.toLowerCase().includes(query) ||
        m.archive_name.toLowerCase().includes(query) ||
        (m.metadata?.version || "").toLowerCase().includes(query)
    );
  }

  // 引擎过滤
  if (filterEngine.value) {
    result = result.filter((m) => m.metadata?.engine === filterEngine.value);
  }

  // 排序
  result = [...result].sort((a, b) => {
    switch (sortBy.value) {
      case "name":
        return a.source_name.localeCompare(b.source_name);
      case "size":
        return b.source_size.bytes - a.source_size.bytes;
      case "ratio":
        return (
          parseFloat(a.compression_ratio) - parseFloat(b.compression_ratio)
        );
      case "date":
        return b.created_at.localeCompare(a.created_at);
      default:
        return 0;
    }
  });

  return result;
});

// 统计信息
const stats = computed(() => {
  const total = mappings.value.length;
  const totalSourceSize = mappings.value.reduce(
    (sum, m) => sum + m.source_size.bytes,
    0
  );
  const totalCompressedSize = mappings.value.reduce(
    (sum, m) => sum + m.compressed_size.bytes,
    0
  );
  const avgRatio =
    total > 0
      ? mappings.value.reduce(
          (sum, m) => sum + parseFloat(m.compression_ratio),
          0
        ) / total
      : 0;

  return {
    total,
    totalSourceSize: (totalSourceSize / (1024 ** 3)).toFixed(2),
    totalCompressedSize: (totalCompressedSize / (1024 ** 3)).toFixed(2),
    avgRatio: avgRatio.toFixed(1),
  };
});

// 初始化
onMounted(() => {
  const saved = localStorage.getItem("library_output_dir");
  if (saved) {
    outputDir.value = saved;
    loadMappingFile();
  }
});
</script>

<template>
  <div class="page-content">
    <!-- 标题栏 -->
    <div class="library-header">
      <h2 class="library-title">📚 游戏库管理</h2>
      <p class="library-subtitle">可视化查看压缩成果，手动标记游戏版本和引擎信息</p>
    </div>

    <!-- 控制栏 -->
    <div class="glass-card control-bar">
      <div class="control-row">
        <div class="dir-control">
          <label class="control-label">📁 输出目录：</label>
          <input
            v-model="outputDir"
            class="input-glass dir-input"
            placeholder="选择包含 pack_mapping.json 的目录"
            readonly
          />
          <button class="btn-glass" @click="browseDir">浏览</button>
          <button
            class="btn-glass"
            :disabled="!outputDir || loading"
            @click="loadMappingFile"
          >
            🔄 刷新
          </button>
        </div>
      </div>

      <!-- 统计信息 -->
      <div v-if="mappings.length > 0" class="stats-row">
        <div class="stat-item">
          <span class="stat-label">总数：</span>
          <span class="stat-value">{{ stats.total }} 个游戏</span>
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
      <div v-if="mappings.length > 0" class="filter-row">
        <div class="search-box">
          <span class="search-icon">🔍</span>
          <input
            v-model="searchQuery"
            class="input-glass search-input"
            placeholder="搜索游戏名称、压缩包名或版本号..."
          />
        </div>

        <select v-model="filterEngine" class="select-glass">
          <option value="">全部引擎</option>
          <option value="Unity">Unity</option>
          <option value="Unreal">Unreal</option>
          <option value="RPG Maker">RPG Maker</option>
          <option value="Godot">Godot</option>
          <option value="自研">自研</option>
          <option value="其他">其他</option>
        </select>

        <select v-model="sortBy" class="select-glass">
          <option value="name">按名称排序</option>
          <option value="size">按大小排序</option>
          <option value="ratio">按压缩率排序</option>
          <option value="date">按日期排序</option>
        </select>
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
    <div v-if="!loading && !error && mappings.length === 0" class="empty-box glass-card">
      <div class="empty-icon">📦</div>
      <p class="empty-text">还没有游戏记录</p>
      <p class="empty-hint">
        请先使用"打包压缩"功能打包游戏，或选择包含 pack_mapping.json 的目录
      </p>
    </div>

    <!-- 游戏卡片网格 -->
    <div v-if="filteredMappings.length > 0" class="game-grid">
      <GameCard
        v-for="(mapping, index) in filteredMappings"
        :key="mapping.archive_name"
        :mapping="mapping"
        @update:metadata="updateMetadata(mappings.indexOf(mapping), $event)"
      />
    </div>

    <!-- 无搜索结果 -->
    <div
      v-if="!loading && mappings.length > 0 && filteredMappings.length === 0"
      class="empty-box glass-card"
    >
      <div class="empty-icon">🔍</div>
      <p class="empty-text">没有找到匹配的游戏</p>
      <p class="empty-hint">尝试调整搜索条件或筛选器</p>
    </div>
  </div>
</template>

<style scoped>
.page-content {
  display: flex;
  flex-direction: column;
  gap: 20px;
}

/* 标题栏 */
.library-header {
  text-align: center;
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
  padding: 20px;
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
  flex: 1;
  cursor: pointer;
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
  gap: 12px;
  align-items: center;
}

.search-box {
  flex: 1;
  position: relative;
  display: flex;
  align-items: center;
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
  grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
  gap: 20px;
}

@media (max-width: 960px) {
  .game-grid {
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 16px;
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
