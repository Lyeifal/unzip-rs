<script setup lang="ts">
import "./styles.css";
import { ref } from "vue";
import UnpackView from "./components/UnpackView.vue";
import PackView from "./components/PackView.vue";

// 两页卡片切换；KeepAlive 保持各自状态（日志、列表、配置不丢）。
const tab = ref<"unpack" | "pack">("unpack");
</script>

<template>
  <div class="root">
    <div class="tabs">
      <button
        class="tab"
        :class="{ active: tab === 'unpack' }"
        @click="tab = 'unpack'"
      >
        自动解压
      </button>
      <button
        class="tab"
        :class="{ active: tab === 'pack' }"
        @click="tab = 'pack'"
      >
        打包压缩
      </button>
    </div>
    <KeepAlive>
      <UnpackView v-if="tab === 'unpack'" />
      <PackView v-else />
    </KeepAlive>
  </div>
</template>
