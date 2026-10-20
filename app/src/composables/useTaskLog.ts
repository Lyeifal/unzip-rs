import { nextTick, ref } from 'vue';

const LEVELS = new Set(['ok', 'error', 'warn', 'skip', 'info']);
const MAX_LINES = 5000;

export function useTaskLog() {
  const logs = ref<{ msg: string; level: string }[]>([]);
  const logBox = ref<HTMLElement | null>(null);
  let scrollPending = false;

  function appendLog(msg: string, level = 'info') {
    const element = logBox.value;
    const follow = !element || element.scrollHeight - element.scrollTop - element.clientHeight < 48;
    logs.value.push({ msg, level: LEVELS.has(level) ? level : 'info' });
    if (logs.value.length > MAX_LINES) logs.value.splice(0, logs.value.length - MAX_LINES);
    if (!follow || scrollPending) return;
    scrollPending = true;
    void nextTick(() => {
      scrollPending = false;
      if (logBox.value) logBox.value.scrollTop = logBox.value.scrollHeight;
    });
  }

  return { logs, logBox, appendLog };
}
