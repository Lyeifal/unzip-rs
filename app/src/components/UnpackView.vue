<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import StyledSelect from "./StyledSelect.vue";
import TaskActivity from './TaskActivity.vue';
import type { SelectOption } from "../types/select";
import { useShellQueue, appendUniquePaths } from '../composables/useShellQueue';
import { waitForToolPathsSave } from '../composables/useToolPaths';
import { useTaskLog } from '../composables/useTaskLog';
import {
  defaultConfig,
  formatRule,
  type Config,
  type SummaryPayload,
} from "../types";

const { unpackPaths: selectedPaths, directRequests, directActive, quickActive } = useShellQueue();
const shellReady = ref(false);
let shellTimer: ReturnType<typeof setTimeout> | undefined;
function scheduleShellRun() {
  if (!shellReady.value || running.value || starting.value || quickActive.value || !directRequests.value.length || shellTimer) return;
  shellTimer = setTimeout(() => { shellTimer = undefined; void startShellRequest(); }, 300);
}
async function startShellRequest() {
  if (!shellReady.value || running.value || starting.value || quickActive.value) return;
  const request = directRequests.value.shift();
  if (!request) return;
  starting.value = true;
  running.value = true;
  const previousSummary = summary.value;
  const previousProgress = { done: done.value, total: total.value, current: current.value };
  summary.value = null;
  done.value = 0; total.value = 0; current.value = `正在准备右键解压：${request.paths.join('、')}`;
  try {
    await waitForToolPathsSave();
    const accepted = await invoke<boolean>('start_shell_run', { action: request.action, paths: [...request.paths] });
    if (!accepted) {
      directRequests.value.unshift(request);
      summary.value = previousSummary;
      done.value = previousProgress.done; total.value = previousProgress.total; current.value = previousProgress.current;
      running.value = false;
    } else {
      appendLog(`[右键解压] ${request.action === 'extract-here' ? '解压到当前目录' : '解压到同名文件夹'}，使用已保存的密码库及启用规则`, 'info');
    }
  } catch (error) {
    running.value = false;
    summary.value = previousSummary;
    done.value = previousProgress.done; total.value = previousProgress.total; current.value = previousProgress.current;
    appendLog(`[右键解压失败] ${error}`, 'error');
    void invoke('finish_shell_requests', { reveal: true }).catch(() => {});
  } finally { starting.value = false; scheduleShellRun(); }
}
type SourceMode = 'selected' | 'directory' | 'both';
const SOURCE_MODE_KEY = 'unpack_source_mode_v1';
function initialSourceMode(): SourceMode {
  try {
    const saved = localStorage.getItem(SOURCE_MODE_KEY);
    if (saved === 'selected' || saved === 'directory' || saved === 'both') return saved;
  } catch { /* localStorage unavailable: use current selection */ }
  return selectedPaths.value.length ? 'selected' : 'directory';
}
const sourceMode = ref<SourceMode>(initialSourceMode());
const sourceModeTouched = ref(false);
try { sourceModeTouched.value = localStorage.getItem(SOURCE_MODE_KEY) !== null; } catch { /* no storage */ }
function setSourceMode(mode: SourceMode, manual = true) {
  if (manual) sourceModeTouched.value = true;
  sourceMode.value = mode;
  try { localStorage.setItem(SOURCE_MODE_KEY, mode); } catch { /* still usable for this session */ }
}
watch(() => selectedPaths.value.length, (length, previous) => {
  if (!sourceModeTouched.value && previous === 0 && length > 0 && sourceMode.value === 'directory') setSourceMode('selected', false);
});
async function addSelectedFiles() {
  const paths = await invoke<string[]>('pick_files', { current: cfg.source_dir });
  selectedPaths.value = appendUniquePaths(selectedPaths.value, paths);
}
async function addSelectedDirectories() {
  const paths = await invoke<string[]>('pick_directories_multi', { current: cfg.source_dir });
  selectedPaths.value = appendUniquePaths(selectedPaths.value, paths);
}

type DirKey = "source_dir" | "output_dir" | "failed_dir";

const dirRows: { key: DirKey; label: string; hint: string }[] = [
  { key: "source_dir", label: "压缩包所在目录", hint: "源目录（递归扫描，分卷可散落各处）" },
  { key: "output_dir", label: "解压输出目录", hint: "解压后的文件保存到这里，每个包一个独立文件夹" },
  { key: "failed_dir", label: "解压失败文件所在目录", hint: "失败的压缩包连同分卷移入此处" },
];

const passwordStrategyOptions: SelectOption[] = [
  { value: "recent_first", label: "最近成功优先（自动置顶，越用越准）" },
  { value: "list_order", label: "严格按列表顺序" },
];
const ruleKindOptions: SelectOption[] = [
  { value: "suffix", label: "按压缩包后缀（如 .lz4、.zip）" },
  { value: "keyword", label: "按文件名或路径关键词" },
];

const cfg = reactive<Config>(defaultConfig());
const running = ref(false);
const starting = ref(false);
const { logs, logBox, appendLog } = useTaskLog();
const done = ref(0);
const total = ref(0);
const current = ref("");
const summary = ref<SummaryPayload | null>(null);
const retrySlot = ref<number | null>(null);
const retryPassword = ref('');
const retryError = ref('');

const pwSel = ref(-1);
const ruleSel = ref(-1);
type SettingsPanel = 'passwords' | 'rules' | 'options';
const settingsPanel = ref<SettingsPanel | null>(null);
const settingsError = ref('');
const savingSettings = ref(false);
let settingsSnapshot: Config | null = null;
function openSettings(panel: SettingsPanel) {
  settingsSnapshot = JSON.parse(JSON.stringify(cfg)) as Config;
  settingsError.value = '';
  settingsPanel.value = panel;
}
function cancelSettings() {
  if (savingSettings.value) return;
  if (settingsSnapshot) Object.assign(cfg, settingsSnapshot);
  settingsSnapshot = null;
  settingsPanel.value = null;
  pwSel.value = -1;
  ruleSel.value = -1;
}
async function saveSettings() {
  if (!settingsPanel.value || savingSettings.value) return;
  savingSettings.value = true;
  settingsError.value = '';
  try {
    await waitForToolPathsSave();
    const saved = await invoke<boolean>('save_config', { cfg });
    if (!saved) throw new Error('配置保存失败，请检查配置目录写入权限或磁盘空间');
    settingsSnapshot = null;
    settingsPanel.value = null;
  } catch (error) { settingsError.value = String(error); }
  finally { savingSettings.value = false; }
}

// 产物后缀：界面以空格分隔文本编辑，存储为数组。
const productExtsText = computed({
  get: () => cfg.product_exts.join(" "),
  set: (v: string) => {
    cfg.product_exts = v.split(/\s+/).filter(Boolean);
  },
});

// 添加密码弹窗
const pwDlg = ref(false);
const pwInput = ref("");

// 密码策略规则弹窗
const ruleDlg = ref(false);
const ruleEditIdx = ref(-1);
const ruleKind = ref<"suffix" | "keyword">("suffix");
const ruleValue = ref("");
const rulePws = ref("");
const ruleErr = ref("");

let unlisteners: UnlistenFn[] = [];

// ---------- 目录/文件选择 ----------

async function browseDir(key: DirKey) {
  const picked = await invoke<string | null>("pick_directory", {
    current: cfg[key],
  });
  if (picked) cfg[key] = picked;
}

// ---------- 密码库 ----------

function pwAdd() {
  pwInput.value = "";
  pwDlg.value = true;
}

function pwAddOk() {
  const pw = pwInput.value;
  if (pw && !cfg.passwords.includes(pw)) {
    cfg.passwords.push(pw);
  }
  pwDlg.value = false;
}

function pwDel() {
  if (pwSel.value >= 0 && pwSel.value < cfg.passwords.length) {
    cfg.passwords.splice(pwSel.value, 1);
    pwSel.value = -1;
  }
}

function pwMove(delta: number) {
  const i = pwSel.value;
  if (i < 0 || i >= cfg.passwords.length) return;
  const j = Math.max(0, Math.min(cfg.passwords.length - 1, i + delta));
  if (i === j) return;
  const t = cfg.passwords[i];
  cfg.passwords[i] = cfg.passwords[j];
  cfg.passwords[j] = t;
  pwSel.value = j;
}

// ---------- 密码策略规则 ----------

function ruleOpen(idx: number) {
  ruleEditIdx.value = idx;
  ruleErr.value = "";
  if (idx >= 0) {
    const r = cfg.password_rules[idx];
    if ((r.suffix || "").trim()) {
      ruleKind.value = "suffix";
      ruleValue.value = r.suffix.trim();
    } else {
      ruleKind.value = "keyword";
      ruleValue.value = (r.keyword || "").trim();
    }
    rulePws.value = (r.passwords || []).join(" ");
  } else {
    ruleKind.value = "suffix";
    ruleValue.value = "";
    rulePws.value = "";
  }
  ruleDlg.value = true;
}

function ruleOk() {
  const pws = rulePws.value
    .replace(/，/g, ",")
    .replace(/,/g, " ")
    .split(/\s+/)
    .filter(Boolean);
  if (!pws.length) {
    ruleErr.value = "请至少填写一个优先密码";
    return;
  }
  let rule;
  if (ruleKind.value === "suffix") {
    let sfx = ruleValue.value.trim();
    if (!sfx) {
      ruleErr.value = "请填写后缀（如 .lz4）";
      return;
    }
    if (!sfx.startsWith(".")) sfx = "." + sfx;
    rule = { suffix: sfx, keyword: "", passwords: pws, enabled: ruleEditIdx.value >= 0 ? cfg.password_rules[ruleEditIdx.value].enabled !== false : true };
  } else {
    const kw = ruleValue.value.trim();
    if (!kw) {
      ruleErr.value = "请填写关键词";
      return;
    }
    rule = { suffix: "", keyword: kw, passwords: pws, enabled: ruleEditIdx.value >= 0 ? cfg.password_rules[ruleEditIdx.value].enabled !== false : true };
  }
  if (ruleEditIdx.value >= 0) {
    cfg.password_rules[ruleEditIdx.value] = rule;
  } else {
    cfg.password_rules.push(rule);
  }
  ruleDlg.value = false;
}

function toggleRule(index: number) {
  const rule = cfg.password_rules[index];
  if (rule) rule.enabled = rule.enabled === false;
}

function ruleDel() {
  if (ruleSel.value >= 0 && ruleSel.value < cfg.password_rules.length) {
    cfg.password_rules.splice(ruleSel.value, 1);
    ruleSel.value = -1;
  }
}

function ruleMove(delta: number) {
  const i = ruleSel.value;
  if (i < 0 || i >= cfg.password_rules.length) return;
  const j = i + delta;
  if (j < 0 || j >= cfg.password_rules.length) return;
  const t = cfg.password_rules[i];
  cfg.password_rules[i] = cfg.password_rules[j];
  cfg.password_rules[j] = t;
  ruleSel.value = j;
}

// ---------- 运行控制 ----------

async function start() {
  if (running.value || starting.value || quickActive.value) return;
  const src = cfg.source_dir.trim();
  if ((sourceMode.value === 'directory' || sourceMode.value === 'both') && !src) {
    appendLog("[警告] 请先选择有效的压缩包所在目录", "warn");
    return;
  }
  if ((sourceMode.value === 'selected' || sourceMode.value === 'both') && !selectedPaths.value.length) {
    appendLog("[警告] 请先添加要解压的文件或目录", "warn");
    return;
  }
  starting.value = true;
  try {
    if (!cfg.output_dir.trim()) {
      appendLog("[提示] 请选择解压输出目录，用于保存解压后的文件", "info");
      await browseDir('output_dir');
      if (!cfg.output_dir.trim()) return;
    }
    running.value = true;
    await waitForToolPathsSave();
    const saved = await invoke<boolean>("save_config", { cfg });
    if (!saved) throw new Error('解压设置保存失败，请检查配置目录写入权限或磁盘空间');
    logs.value = [];
    summary.value = null;
    done.value = 0;
    total.value = 0;
    current.value = "";
    running.value = true;
    await invoke("start_run", {
      sourceDir: src,
      outputDir: cfg.output_dir.trim(),
      failedDir: cfg.failed_dir.trim(),
      files: sourceMode.value === 'directory' ? null : [...selectedPaths.value],
      sourceMode: sourceMode.value,
    });
  } catch (e) {
    running.value = false;
    appendLog(`[失败] ${e}`, "error");
  } finally { starting.value = false; }
}

async function stop() {
  directRequests.value = [];
  if (shellTimer) { clearTimeout(shellTimer); shellTimer = undefined; }
  await invoke("cancel_run");
  appendLog("[警告] 正在停止…", "warn");
}

function onDone(s: SummaryPayload) {
  if (s.failed.length || s.warns.length) void invoke('finish_shell_requests', { reveal: true }).catch(() => {});
  running.value = false;
  summary.value = s;
  appendLog(
    `[完成] 成功 ${s.ok.length}，失败 ${s.failed.length}，跳过 ${s.skipped.length}，警告 ${s.warns.length}`,
    s.failed.length ? "warn" : "ok",
  );
  for (const [name, reason] of s.failed) {
    appendLog(`失败：${name} — ${reason}`, "error");
  }
  for (const w of s.warns) {
    appendLog(`警告：${w}`, "warn");
  }
  nextTick(() => {
    const el = logBox.value;
    if (el) el.scrollTop = el.scrollHeight;
  });
}

function openRetry(slot: number) {
  retrySlot.value = slot;
  retryPassword.value = '';
  retryError.value = '';
}

async function submitRetry() {
  if (retrySlot.value === null || running.value || starting.value || quickActive.value) return;
  if (!retryPassword.value) { retryError.value = '请输入新密码'; return; }
  try {
    running.value = true;
    retryError.value = '';
    await invoke('retry_unpack_password', {
      slot: retrySlot.value,
      password: retryPassword.value,
      outputDir: cfg.output_dir.trim(),
    });
    retrySlot.value = null;
    retryPassword.value = '';
  } catch (error) {
    running.value = false;
    retryError.value = String(error);
  }
}

function onRetryDone(payload: { slot: number; failedIndex: number; result: SummaryPayload }) {
  running.value = false;
  const result = payload.result;
  const currentSummary = summary.value;
  if (!currentSummary) return;
  const failedIndex = currentSummary.retryable.find(item => item.slot === payload.slot)?.failed_index;
  if (failedIndex === undefined) return;
  if (result.ok.length) {
    currentSummary.ok.push(...result.ok);
    currentSummary.failed.splice(failedIndex, 1);
    currentSummary.retryable = currentSummary.retryable
      .filter(item => item.slot !== payload.slot)
      .map(item => ({ ...item, failed_index: item.failed_index > failedIndex ? item.failed_index - 1 : item.failed_index }));
    appendLog(`[成功] ${result.ok[0][0]} 使用新密码重试成功`, 'ok');
    invoke<Config>('load_config').then(loaded => { cfg.passwords = loaded.passwords; }).catch(() => {});
  } else {
    const failure = result.failed[0];
    if (failure) currentSummary.failed[failedIndex] = failure;
    if (failure && !result.retryable.length) currentSummary.retryable = currentSummary.retryable.filter(item => item.slot !== payload.slot);
    appendLog(failure ? `[失败] ${failure[0]} — ${failure[1]}` : '[警告] 重试已取消，可再次输入新密码', failure ? 'error' : 'warn');
  }
  currentSummary.warns.push(...result.warns);
}

// ---------- 生命周期 ----------
watch(() => directRequests.value.map(request => `${request.action}:${request.paths.length}`).join('|'), scheduleShellRun);
watch(quickActive, scheduleShellRun);
watch(() => running.value || starting.value, (active) => { directActive.value = active; scheduleShellRun(); }, { flush: 'sync' });

onMounted(async () => {
  try {
    const loaded = await invoke<Config>("load_config");
    Object.assign(cfg, loaded);
  } catch (e) {
    appendLog(`[失败] 配置加载失败：${e}`, "error");
  }
  unlisteners.push(
    await listen<{ msg: string; level: string }>("uz-log", (e) =>
      appendLog(e.payload.msg, e.payload.level),
    ),
    await listen<{ done: number; total: number; name: string }>(
      "uz-progress",
      (e) => {
        done.value = e.payload.done;
        total.value = e.payload.total;
        current.value = `(${e.payload.done}/${e.payload.total}) ${e.payload.name}`;
      },
    ),
    await listen<SummaryPayload>("uz-done", (e) => onDone(e.payload)),
    await listen<{ slot: number; failedIndex: number; result: SummaryPayload }>("uz-retry-done", (e) => onRetryDone(e.payload)),
  );
  shellReady.value = true;
  scheduleShellRun();
});

onUnmounted(() => {
  shellReady.value = false;
  if (shellTimer) clearTimeout(shellTimer);
  for (const u of unlisteners) u();
  unlisteners = [];
});
</script>

<template>
  <div class="page-content">
    <h1 class="title">自动解压</h1>
    <TaskActivity :running="running" :current="current" :done="done" :total="total" kind="解压" />
    <p v-if="directRequests.length" class="selection-hint" role="status">右键解压等待中（{{ directRequests.length }} 批），当前任务结束后自动开始，使用已保存的解压和密码设置。</p>
    <p v-if="quickActive" class="selection-hint" role="status">打包任务进行中，完成后可开始解压。</p>
    <section class="glass-card section-card">
      <div class="section-title">解压来源</div>
      <div class="source-modes" role="radiogroup" aria-label="解压来源方式">
        <label><input type="radio" name="unpack-source-mode" value="selected" :checked="sourceMode === 'selected'" :disabled="running" @change="setSourceMode('selected')" /> 仅选中文件</label>
        <label><input type="radio" name="unpack-source-mode" value="directory" :checked="sourceMode === 'directory'" :disabled="running" @change="setSourceMode('directory')" /> 仅扫描源目录</label>
        <label><input type="radio" name="unpack-source-mode" value="both" :checked="sourceMode === 'both'" :disabled="running" @change="setSourceMode('both')" /> 两者一起</label>
      </div>
      <p class="selection-hint">{{ sourceMode === 'both' ? '本次会合并选中的文件或文件夹与下方源目录，同一路径只处理一次。分卷请选齐同组文件。' : sourceMode === 'selected' ? '本次只处理下列选中项，不扫描其他源目录。分卷请选齐同组文件。' : '本次只扫描下方源目录；选中列表会保留，切换来源方式后仍可使用。' }}</p>
      <ul v-if="selectedPaths.length" class="list selected-files">
        <li v-for="(path, index) in selectedPaths" :key="path" class="item selected-file">
          <span :title="path">{{ path }}</span>
          <button class="btn-glass" :disabled="running" :aria-label="'移除 ' + path" @click="selectedPaths.splice(index, 1)">移除</button>
        </li>
      </ul>
      <div class="selection-actions"><button class="btn-glass" :disabled="running" @click="addSelectedFiles">添加文件</button><button class="btn-glass" :disabled="running" @click="addSelectedDirectories">添加文件夹</button><button v-if="selectedPaths.length" class="btn-glass" :disabled="running" @click="selectedPaths = []">清空列表</button></div>
    </section>

    <!-- 目录设置 -->
    <section class="glass-card section-card">
      <div class="section-title">目录设置</div>
      <div v-for="row in dirRows" :key="row.key" class="row">
        <label class="lbl" :for="`unpack-${row.key}`">{{ row.label }}</label>
        <label v-if="row.key === 'failed_dir'"><input v-model="cfg.failed_dir_enabled" type="checkbox" :disabled="running || starting" /> 启用解压失败目录</label>
        <input :id="`unpack-${row.key}`" v-model="cfg[row.key]" class="input-glass" :placeholder="row.hint" :disabled="running || starting || (row.key === 'source_dir' && sourceMode === 'selected') || (row.key === 'failed_dir' && !cfg.failed_dir_enabled)" />
        <button class="btn-glass" :disabled="running || starting || (row.key === 'source_dir' && sourceMode === 'selected') || (row.key === 'failed_dir' && !cfg.failed_dir_enabled)" @click="browseDir(row.key)">浏览…</button>
      </div>
      <p class="selection-hint">{{ cfg.failed_dir_enabled ? '失败的压缩包及分卷会移入失败目录；未填写路径时使用输出目录下的 failed 文件夹。' : '失败目录已停用：失败的压缩包及分卷保留在原位置，已选的失败目录路径会保留。' }}</p>
    </section>

    <section class="glass-card section-card">
      <div class="section-title">解压配置</div>
      <div class="settings-buttons">
        <button class="btn-glass" :disabled="running" @click="openSettings('passwords')">密码库 <span>· {{ cfg.passwords.length }} 条</span></button>
        <button class="btn-glass" :disabled="running" @click="openSettings('rules')">密码策略规则 <span>· {{ cfg.password_rules.filter(rule => rule.enabled !== false).length }}/{{ cfg.password_rules.length }} 启用</span></button>
        <button class="btn-glass" :disabled="running" @click="openSettings('options')">解压选项</button>
      </div>
    </section>

    <!-- 运行 -->
    <div class="row run-row">
      <button class="btn-glass btn-primary" :disabled="running || starting || quickActive" @click="start">开始解压</button>
      <button class="btn-glass btn-danger" :disabled="!running" @click="stop">停止</button>
      <span class="current">{{ current }}</span>
    </div>

    <div class="bar">
      <div
        class="bar-inner"
        :style="{ width: total > 0 ? Math.min(100, (done / total) * 100) + '%' : '0%' }"
      ></div>
    </div>

    <!-- 运行日志 -->
    <section class="group log-group">
      <div class="section-title">运行日志</div>
      <div ref="logBox" class="log-view">
        <div v-for="(l, i) in logs" :key="i" class="log-line" :class="'lv-' + l.level">
          {{ l.msg }}
        </div>
        <div v-if="summary" class="summary">
          <div class="sum-head">
            成功 {{ summary.ok.length }}，失败 {{ summary.failed.length }}，跳过
            {{ summary.skipped.length }}，警告 {{ summary.warns.length }}
          </div>
          <div v-for="([name, reason], i) in summary.failed" :key="'f' + i" class="sum-line lv-error">
            失败：{{ name }} — {{ reason }}
            <button v-if="summary.retryable?.some(item => item.failed_index === i)" class="btn-glass retry-btn" :disabled="running || starting || quickActive" @click="openRetry(summary.retryable.find(item => item.failed_index === i)!.slot)">添加新密码并重试</button>
          </div>
          <div v-for="(w, i) in summary.warns" :key="'w' + i" class="sum-line lv-warn">
            警告：{{ w }}
          </div>
          <button class="btn sum-close" @click="summary = null">关闭</button>
        </div>
      </div>
    </section>

    <div v-if="retrySlot !== null" class="overlay" @click.self="retrySlot = null">
      <div class="dialog" role="dialog" aria-modal="true" aria-label="添加新密码并重试">
        <div class="dlg-title">添加新密码并重试</div>
        <p class="selection-hint">本次只对这个密码不匹配的压缩包尝试新密码；成功后加入密码库。</p>
        <input v-model="retryPassword" class="input-glass" type="text" aria-label="新密码" autofocus @keyup.enter="submitRetry" />
        <div v-if="retryError" class="dlg-err" role="alert">{{ retryError }}</div>
        <div class="row dlg-btns"><button class="btn-glass btn-primary" :disabled="running || starting || quickActive" @click="submitRetry">重试</button><button class="btn-glass" @click="retrySlot = null">取消</button></div>
      </div>
    </div>

    <!-- 三组解压配置弹窗 -->
    <div v-if="settingsPanel" class="overlay" @click.self="cancelSettings">
      <div class="dialog settings-dialog" role="dialog" aria-modal="true" :aria-label="settingsPanel === 'passwords' ? '密码库配置' : settingsPanel === 'rules' ? '密码策略规则配置' : '解压选项配置'">
        <div class="dlg-title">{{ settingsPanel === 'passwords' ? '密码库' : settingsPanel === 'rules' ? '密码策略规则' : '解压选项' }}</div>

        <template v-if="settingsPanel === 'passwords'">
          <p class="selection-hint">全局密码按下面的排序规则尝试；命中后可自动置顶。</p>
          <div class="split">
            <ul class="list modal-list" @click="pwSel = -1">
              <li v-for="(pw, i) in cfg.passwords" :key="i" :class="{ sel: i === pwSel }" @click.stop="pwSel = i">{{ pw }}</li>
            </ul>
            <div class="btns">
              <button class="btn-glass" @click="pwAdd">添加</button>
              <button class="btn-glass" :disabled="pwSel < 0" @click="pwDel">删除</button>
              <button class="btn-glass" :disabled="pwSel < 0" @click="pwMove(-1)">上移</button>
              <button class="btn-glass" :disabled="pwSel < 0" @click="pwMove(1)">下移</button>
            </div>
          </div>
          <div class="row">
            <label class="lbl">密码排序规则</label>
            <StyledSelect v-model="cfg.password_strategy" class="select" :options="passwordStrategyOptions" ariaLabel="密码排序规则" />
          </div>
        </template>

        <template v-else-if="settingsPanel === 'rules'">
          <p class="selection-hint">命中的启用规则按列表顺序优先尝试；停用规则保留但不参与。未命中时继续使用密码库。</p>
          <div class="row">
            <label><input v-model="cfg.hash_match" type="checkbox" /> 按 SHA-256 匹配记录密码</label>
            <span class="current">命中后最先尝试；未命中或密码无效时继续使用规则和密码库</span>
          </div>
          <div class="split">
            <ul class="list modal-list" @click="ruleSel = -1">
              <li v-for="(rule, i) in cfg.password_rules" :key="i" class="rule-list-item" :class="{ sel: i === ruleSel, disabled: rule.enabled === false }" @click.stop="ruleSel = i">
                <span>{{ formatRule(rule) }}</span>
                <button class="btn-glass rule-toggle" :aria-label="(rule.enabled === false ? '启用规则 ' : '停用规则 ') + (i + 1)" @click.stop="toggleRule(i)">{{ rule.enabled === false ? '启用' : '停用' }}</button>
              </li>
            </ul>
            <div class="btns">
              <button class="btn-glass" @click="ruleOpen(-1)">添加</button>
              <button class="btn-glass" :disabled="ruleSel < 0" @click="ruleOpen(ruleSel)">编辑</button>
              <button class="btn-glass" :disabled="ruleSel < 0" @click="ruleDel">删除</button>
              <button class="btn-glass" :disabled="ruleSel < 0" @click="ruleMove(-1)">上移</button>
              <button class="btn-glass" :disabled="ruleSel < 0" @click="ruleMove(1)">下移</button>
            </div>
          </div>
        </template>

        <template v-else>
          <div class="row">
            <label><input v-model="cfg.recursive" type="checkbox" /> 递归扫描源目录</label>
          </div>
          <div class="row">
            <label class="lbl">产物后缀（不解压，原样放入）</label>
            <input v-model="productExtsText" class="input-glass" aria-label="产物后缀" placeholder=".apk .xapk .apks .aab" />
          </div>
        </template>

        <div v-if="settingsError" class="dlg-err" role="alert">{{ settingsError }}</div>
        <div class="row dlg-btns">
          <button class="btn-glass btn-primary" :disabled="savingSettings" @click="saveSettings">{{ savingSettings ? '保存中…' : '保存并关闭' }}</button>
          <button class="btn-glass" :disabled="savingSettings" @click="cancelSettings">取消</button>
        </div>
      </div>
    </div>

    <!-- 添加密码弹窗 -->
    <div v-if="pwDlg" class="overlay" @click.self="pwDlg = false">
      <div class="dialog">
        <div class="dlg-title">添加密码</div>
        <div class="row">
          <label class="lbl">输入常用密码：</label>
          <input
            v-model="pwInput"
            class="input-glass"
            autofocus
            @keyup.enter="pwAddOk"
          />
        </div>
        <div class="row dlg-btns">
          <button class="btn-glass btn-primary" @click="pwAddOk">确定</button>
          <button class="btn-glass" @click="pwDlg = false">取消</button>
        </div>
      </div>
    </div>

    <!-- 密码策略规则弹窗 -->
    <div v-if="ruleDlg" class="overlay" @click.self="ruleDlg = false">
      <div class="dialog">
        <div class="dlg-title">密码策略规则</div>
        <div class="row">
          <label class="lbl">条件类型</label>
          <StyledSelect
            v-model="ruleKind"
            class="select"
            :options="ruleKindOptions"
            ariaLabel="匹配规则类型"
          />
        </div>
        <div class="row">
          <label class="lbl">条件值</label>
          <input
            v-model="ruleValue"
            class="input-glass"
            :placeholder="ruleKind === 'suffix' ? '.lz4（后缀）' : '如 游戏、汉化、整合'"
          />
        </div>
        <div class="row">
          <label class="lbl">优先密码</label>
          <input
            v-model="rulePws"
            class="input-glass"
            placeholder="多个密码用逗号或空格分隔，按顺序优先尝试"
            @keyup.enter="ruleOk"
          />
        </div>
        <div v-if="ruleErr" class="dlg-err">{{ ruleErr }}</div>
        <div class="row dlg-btns">
          <button class="btn-glass btn-primary" @click="ruleOk">确定</button>
          <button class="btn-glass" @click="ruleDlg = false">取消</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.selection-hint { color: var(--text-secondary); font-size: 12px; margin: 0 0 12px; line-height: 1.6; }
.selected-files { max-height: 180px; }
.selected-file { display: flex; gap: 8px; align-items: center; justify-content: space-between; }
.selected-file span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.selected-file button { flex: 0 0 auto; padding: 4px 8px; }
.selection-actions { display: flex; gap: 8px; margin-top: 10px; }
.source-modes { display: flex; flex-wrap: wrap; gap: 10px; margin-bottom: 10px; }
.source-modes label { display: inline-flex; align-items: center; gap: 5px; padding: 7px 10px; border: 1px solid var(--border); border-radius: var(--radius-md); color: var(--text-primary); }
.settings-buttons { display: flex; flex-wrap: wrap; gap: 10px; }
.settings-buttons .btn-glass { min-width: 145px; text-align: left; }
.settings-buttons span { color: var(--text-secondary); font-size: 12px; }
.settings-dialog { width: min(680px, 94vw); max-height: 86vh; overflow-y: auto; }
.retry-btn { margin-left: 10px; padding: 4px 8px; }
.modal-list { max-height: 270px; min-height: 100px; overflow-y: auto; }
.rule-list-item { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.rule-list-item span { min-width: 0; overflow-wrap: anywhere; }
.rule-list-item.disabled span { opacity: .55; }
.rule-toggle { flex: 0 0 auto; padding: 4px 8px; }
</style>
