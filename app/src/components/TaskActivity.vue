<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue';
const props = defineProps<{ running: boolean; current: string; done: number; total: number; kind: string }>();
const elapsed = ref(0);
let timer: ReturnType<typeof setInterval> | undefined;
watch(() => props.running, active => {
  if (timer) clearInterval(timer);
  timer = undefined;
  if (active) {
    const started = Date.now(); elapsed.value = 0;
    timer = setInterval(() => { elapsed.value = Math.floor((Date.now() - started) / 1000); }, 1000);
  }
}, { immediate: true });
onUnmounted(() => { if (timer) clearInterval(timer); });
const duration = computed(() => `${Math.floor(elapsed.value / 60)}分${elapsed.value % 60}秒`);
</script>

<template>
  <section v-if="running" class="task-activity glass-card" role="status" :aria-label="kind + '任务状态'">
    <strong>{{ kind }}任务进行中 · 已用 {{ duration }}</strong>
    <p>{{ current || '正在准备任务、扫描文件或匹配密码…' }}</p>
    <p>{{ total ? `已处理 ${done} / ${total} 项。` : '' }}单个大包处理期间会持续等待工具返回；这里显示任务状态与已用时间，不代表包内百分比。</p>
  </section>
</template>

<style scoped>
.task-activity { padding: 14px 18px; margin: 12px 0 18px; border-left: 3px solid var(--text-secondary); }
p { margin: 6px 0 0; overflow-wrap: anywhere; color: var(--text-secondary); font-size: 12px; line-height: 1.6; }
</style>
