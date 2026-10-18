<script setup lang="ts">
import { computed, nextTick, onActivated, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import StyledSelect from "./StyledSelect.vue";
import type { SelectOption } from "../types/select";
import { useShellQueue } from '../composables/useShellQueue';
import { waitForToolPathsSave } from '../composables/useToolPaths';
import {
  defaultPackOptions,
  fmtSize,
  sanitizeName,
  savePackOptions,
  type PackSummaryPayload,
} from "../types";

const LEVELS = ["ok", "error", "warn", "skip", "info"] as const;

const formatOptions: SelectOption[] = [
  { value: "7z", label: "7z（推荐：AES-256 + 文件名加密）" },
  { value: "zip", label: "zip（兼容性好，AES-256）" },
];
const compressionLevelOptions: SelectOption[] = [
  { value: 0, label: "0 - 仅存储（最快）" },
  { value: 1, label: "1" },
  { value: 3, label: "3" },
  { value: 5, label: "5 - 默认（均衡）" },
  { value: 7, label: "7" },
  { value: 9, label: "9 - 极限（最慢）" },
];

const opts = reactive(defaultPackOptions());
const items = useShellQueue().packPaths;
const itemSel = ref(-1);
const running = ref(false);
const starting = ref(false);
const logs = ref<{ msg: string; level: string }[]>([]);
const logBox = ref<HTMLElement | null>(null);
const done = ref(0);
const total = ref(0);
const current = ref("");
const summary = ref<PackSummaryPayload | null>(null);
const copiedIdx = ref(-1);
const namePreview = ref<string[]>([]);
const previewError = ref('');
const previewLoading = ref(false);
const newPrefix = ref('');
const newSuffix = ref('');
const affixError = ref('');
const AFFIX_KEY = 'pack_name_affixes_v1';
function savedAffixes(): { prefixes: string[]; suffixes: string[] } {
  try {
    const data = JSON.parse(localStorage.getItem(AFFIX_KEY) ?? '{}');
    const values = (source: unknown): string[] => Array.isArray(source)
      ? [...new Set(source.filter((value): value is string => typeof value === 'string' && !!value.trim() && value.trim().length <= 80 && sanitizeName(value.trim()) === value.trim()).map(value => value.trim()))]
      : [];
    return { prefixes: values(data.prefixes), suffixes: values(data.suffixes) };
  } catch { return { prefixes: [], suffixes: [] }; }
}
const saved = savedAffixes();
const prefixes = ref([...new Set(['G', 'H', 'HG', ...saved.prefixes, opts.name_prefix].filter(Boolean))]);
const suffixes = ref([...new Set([...saved.suffixes, opts.name_suffix].filter(Boolean))]);
const prefixOptions = computed<SelectOption[]>(() => prefixes.value.map(value => ({ value, label: value })));
const suffixOptions = computed<SelectOption[]>(() => suffixes.value.map(value => ({ value, label: value })));

let unlisteners: UnlistenFn[] = [];
let previewTimer: ReturnType<typeof setTimeout> | undefined;
let previewToken = 0;

const validNameStart = computed(() => /^[0-9]{1,18}$/.test(opts.number_start.trim())
    && Number.isInteger(opts.name_step) && opts.name_step > 0
    && (!opts.prefix_enabled || (!!opts.name_prefix && opts.name_prefix.length <= 80 && sanitizeName(opts.name_prefix) === opts.name_prefix))
    && (!opts.suffix_enabled || (!!opts.name_suffix && opts.name_suffix.length <= 80 && sanitizeName(opts.name_suffix) === opts.name_suffix)));

function persistAffixes() {
  try { localStorage.setItem(AFFIX_KEY, JSON.stringify({ prefixes: prefixes.value, suffixes: suffixes.value })); }
  catch { affixError.value = '前后缀列表保存失败，请检查本机存储空间。'; }
}
function addAffix(kind: 'prefix' | 'suffix') {
  const value = (kind === 'prefix' ? newPrefix.value : newSuffix.value).trim();
  if (!value || value.length > 80 || sanitizeName(value) !== value) {
    affixError.value = '请填写 1–80 个字符的有效前缀或后缀，不能含文件名非法字符。';
    return;
  }
  const values = kind === 'prefix' ? prefixes : suffixes;
  if (!values.value.includes(value)) values.value.push(value);
  if (kind === 'prefix') { opts.name_prefix = value; opts.prefix_enabled = true; newPrefix.value = ''; }
  else { opts.name_suffix = value; opts.suffix_enabled = true; newSuffix.value = ''; }
  affixError.value = '';
  persistAffixes();
}
async function refreshPreview() {
  const token = ++previewToken;
  if (!validNameStart.value) { namePreview.value = []; previewError.value = ''; previewLoading.value = false; return; }
  previewLoading.value = true;
  try {
    const names = await invoke<string[]>('preview_pack_names', { options: { ...opts }, count: Math.max(1, items.value.length) });
    if (!Array.isArray(names) || !names.every(name => typeof name === 'string')) throw new Error('预览结果格式无效');
    if (token === previewToken) { namePreview.value = names; previewError.value = ''; }
  } catch (error) {
    if (token === previewToken) { namePreview.value = []; previewError.value = String(error); }
  } finally { if (token === previewToken) previewLoading.value = false; }
}
function schedulePreview() {
  if (previewTimer) clearTimeout(previewTimer);
  previewToken++;
  previewTimer = setTimeout(() => void refreshPreview(), 180);
}
watch(() => [opts.name_prefix, opts.name_suffix, opts.prefix_enabled,
  opts.suffix_enabled, opts.number_start, opts.name_step, opts.auto_number, opts.format, items.value.length], schedulePreview, { immediate: true });

// ---------- 日志 ----------

function appendLog(msg: string, level: string = "info") {
  logs.value.push({ msg, level: LEVELS.includes(level as (typeof LEVELS)[number]) ? level : "info" });
  nextTick(() => {
    const el = logBox.value;
    if (el) el.scrollTop = el.scrollHeight;
  });
}

// ---------- 待打包列表 ----------

async function addFiles() {
  const picked = await invoke<string[]>("pick_files", { current: "" });
  for (const p of picked) {
    if (!items.value.includes(p)) items.value.push(p);
  }
}

async function addDir() {
  // 一次可多选目录；以列表最后一个条目所在的目录作为对话框起点。
  const last = items.value[items.value.length - 1];
  const startDir = last ? last.replace(/[/\\][^/\\]*$/, "") : "";
  const picked = await invoke<string[]>("pick_directories_multi", { current: startDir });
  for (const p of picked) {
    if (!items.value.includes(p)) items.value.push(p);
  }
}

function removeItem() {
  if (itemSel.value >= 0 && itemSel.value < items.value.length) {
    items.value.splice(itemSel.value, 1);
    itemSel.value = -1;
  }
}

function clearItems() {
  items.value = [];
  itemSel.value = -1;
}

function moveItem(delta: number) {
  const i = itemSel.value;
  if (i < 0 || i >= items.value.length) return;
  const j = i + delta;
  if (j < 0 || j >= items.value.length) return;
  const t = items.value[i];
  items.value[i] = items.value[j];
  items.value[j] = t;
  itemSel.value = j;
}

async function browseOutDir() {
  const picked = await invoke<string | null>("pick_directory", {
    current: opts.output_dir,
  });
  if (picked) opts.output_dir = picked;
}

// ---------- 运行控制 ----------

async function start() {
  if (running.value || starting.value) return;
  if (!items.value.length) {
    appendLog("[警告] 请先添加要打包的文件或目录", "warn");
    return;
  }
  if (!validNameStart.value) {
    appendLog("[警告] 请填写有效的数字起点、步长及已启用的前后缀", "warn");
    return;
  }
  if (opts.password_mode === "manual" && !opts.uniform_password.trim()) {
    appendLog("[警告] 统一密码模式：密码为空将按无密码打包", "warn");
  }
  starting.value = true;
  try {
    if (!opts.output_dir.trim()) {
      appendLog("[提示] 请选择打包输出目录", "info");
      await browseOutDir();
      if (!opts.output_dir.trim()) return;
    }
    logs.value = [];
    summary.value = null;
    done.value = 0;
    total.value = 0;
    current.value = "";
    running.value = true;
    await waitForToolPathsSave();
    await invoke("start_pack", { options: { ...opts }, files: [...items.value] });
  } catch (e) {
    running.value = false;
    appendLog(`[失败] ${e}`, "error");
  } finally { starting.value = false; }
}

async function stop() {
  await invoke("cancel_run");
  appendLog("[警告] 正在停止…", "warn");
}

function onDone(s: PackSummaryPayload) {
  running.value = false;
  summary.value = s;
  appendLog(
    `[完成] 打包结束：成功 ${s.ok.length}，失败 ${s.failed.length}，警告 ${s.warns.length}`,
    s.failed.length ? "warn" : "ok",
  );
  for (const w of s.warns) {
    appendLog(`警告：${w}`, "warn");
  }

  // 保存成功的配置（记住目录、密码、模板等）
  if (s.ok.length > 0) {
    savePackOptions(opts);
  }
  void refreshPreview();

  nextTick(() => {
    const el = logBox.value;
    if (el) el.scrollTop = el.scrollHeight;
  });
}

// ---------- 结果表 ----------

async function copyPassword(text: string, idx: number) {
  try {
    await navigator.clipboard.writeText(text);
    copiedIdx.value = idx;
    setTimeout(() => {
      if (copiedIdx.value === idx) copiedIdx.value = -1;
    }, 1500);
  } catch {
    appendLog("[警告] 复制失败，请手动选择密码复制", "warn");
  }
}

// ---------- 生命周期 ----------

onMounted(async () => {
  unlisteners.push(
    await listen<{ msg: string; level: string }>("pack-log", (e) =>
      appendLog(e.payload.msg, e.payload.level),
    ),
    await listen<{ done: number; total: number; name: string }>(
      "pack-progress",
      (e) => {
        done.value = e.payload.done;
        total.value = e.payload.total;
        current.value = `(${e.payload.done}/${e.payload.total}) ${e.payload.name}`;
      },
    ),
    await listen<PackSummaryPayload>("pack-done", (e) => onDone(e.payload)),
  );
});

onActivated(() => void refreshPreview());

onUnmounted(() => {
  if (previewTimer) clearTimeout(previewTimer);
  previewToken++;
  for (const u of unlisteners) u();
  unlisteners = [];
});
</script>

<template>
  <div class="page-content">
    <h1 class="title">打包压缩</h1>

    <!-- 待打包列表 -->
    <section class="glass-card section-card">
      <div class="section-title">待打包列表（每个条目单独打一个包，按顺序命名）</div>
      <div class="split">
        <ul class="list item-list" @click="itemSel = -1">
          <li
            v-for="(path, i) in items"
            :key="path"
            class="item"
            :class="{ sel: i === itemSel }"
            @click.stop="itemSel = i"
          >
            <span class="src" :title="path">{{ path }}</span>
            <span class="pack-name">{{ namePreview[i] ? `${namePreview[i]}.${opts.format}` : '待计算' }}</span>
          </li>
        </ul>
        <div class="btns">
          <button class="btn-glass" @click="addFiles">添加文件</button>
          <button class="btn-glass" @click="addDir">添加目录</button>
          <button class="btn-glass" :disabled="itemSel < 0" @click="removeItem">移除</button>
          <button class="btn-glass" :disabled="itemSel < 0" @click="moveItem(-1)">上移</button>
          <button class="btn-glass" :disabled="itemSel < 0" @click="moveItem(1)">下移</button>
          <button class="btn-glass" :disabled="!items.length" @click="clearItems">清空</button>
        </div>
      </div>
      <p class="hint">右侧为包名预览；真正开始打包时会再次检查压缩包管理记录，避免使用过期序号。</p>
    </section>

    <!-- 命名与输出 -->
    <section class="glass-card section-card">
      <div class="section-title">命名与输出</div>
        <div class="row">
          <label class="lbl">前缀</label>
          <label class="toggle-label"><input v-model="opts.prefix_enabled" type="checkbox" /> 启用前缀</label>
          <StyledSelect v-model="opts.name_prefix" class="select" :disabled="!opts.prefix_enabled || !prefixOptions.length" :options="prefixOptions" ariaLabel="选择包名前缀" placeholder="先添加前缀" />
        </div>
        <div class="row">
          <label class="lbl">添加新前缀</label>
          <input v-model="newPrefix" class="input-glass affix-input" placeholder="例如 G、HG、合集_" @keydown.enter.prevent="addAffix('prefix')" />
          <button class="btn-glass" type="button" @click="addAffix('prefix')">添加前缀</button>
        </div>
        <div class="row">
          <label class="lbl">后缀</label>
          <label class="toggle-label"><input v-model="opts.suffix_enabled" type="checkbox" /> 启用后缀</label>
          <StyledSelect v-model="opts.name_suffix" class="select" :disabled="!opts.suffix_enabled || !suffixOptions.length" :options="suffixOptions" ariaLabel="选择包名后缀" placeholder="先添加后缀" />
        </div>
        <div class="row">
          <label class="lbl">添加新后缀</label>
          <input v-model="newSuffix" class="input-glass affix-input" placeholder="例如 _正式版、-备份" @keydown.enter.prevent="addAffix('suffix')" />
          <button class="btn-glass" type="button" @click="addAffix('suffix')">添加后缀</button>
        </div>
        <div class="row">
          <label class="lbl">数字起点</label>
          <input v-model="opts.number_start" class="input-glass number-start-input" inputmode="numeric" pattern="[0-9]*" placeholder="001" aria-label="数字起点" />
          <label class="lbl" style="min-width: 36px">步长</label>
          <input v-model.number="opts.name_step" type="number" min="1" step="1" class="input num" aria-label="包名步长" />
        </div>
        <div class="row">
          <label class="lbl">自动续号</label>
          <label class="toggle-label"><input v-model="opts.auto_number" type="checkbox" aria-label="自动续号" /> 匹配压缩包管理中的前后缀，从最大序号之后开始</label>
        </div>
        <p v-if="affixError" class="naming-error" role="alert">{{ affixError }}</p>
        <p v-if="previewError" class="naming-error" role="alert">序号预览失败：{{ previewError }}</p>
        <p class="hint">{{ previewLoading ? '正在匹配已有记录…' : `下一包预览：${namePreview[0] ?? '请完成命名设置'}.${opts.format}` }}。关闭前缀或后缀后，该部分不参与命名或记录匹配。</p>
      <div class="row">
        <label class="lbl">打包输出目录</label>
        <input v-model="opts.output_dir" class="input-glass" placeholder="压缩包输出到此处" />
        <button class="btn-glass" @click="browseOutDir">浏览…</button>
      </div>
      <div class="row">
        <label class="lbl">压缩格式</label>
        <StyledSelect v-model="opts.format" class="select" :options="formatOptions" ariaLabel="压缩格式" />
      </div>
      <div class="row">
        <label class="lbl">压缩级别</label>
        <StyledSelect
          v-model="opts.compression_level"
          class="select"
          :options="compressionLevelOptions"
          ariaLabel="压缩级别"
        />
      </div>
    </section>

    <!-- 密码 -->
    <section class="glass-card section-card">
      <div class="section-title">压缩密码</div>
      <div class="radio-row">
        <label>
          <input type="radio" value="random_per_pack" v-model="opts.password_mode" />
          每包随机（推荐）
        </label>
        <label>
          <input type="radio" value="random_uniform" v-model="opts.password_mode" />
          统一随机
        </label>
        <label>
          <input type="radio" value="manual" v-model="opts.password_mode" />
          统一手填
        </label>
      </div>
      <div v-if="opts.password_mode === 'manual'" class="row">
        <label class="lbl">统一密码</label>
        <input v-model="opts.uniform_password" class="input-glass" placeholder="留空 = 无密码打包" />
      </div>
      <p class="hint">
        随机密码为 16 位字母数字（剔除易混淆字符），打包完成后在结果表中显示，可复制。
      </p>
    </section>

    <!-- 分卷 -->
    <section class="glass-card section-card">
      <div class="section-title">分卷打包</div>
      <div class="row">
        <label class="lbl">分卷阈值（GB）</label>
        <input v-model.number="opts.volume_threshold_gb" type="number" min="0" step="0.5" class="input num" />
        <label class="lbl" style="min-width: 96px">每个分卷（GB）</label>
        <input v-model.number="opts.volume_size_gb" type="number" min="0.1" step="0.5" class="input num" />
      </div>
      <p class="hint">内容总大小超过阈值才分卷（压缩后大小不可预知，按源大小判断）。默认超过 3 GB 分卷为 2 GB/个。</p>
    </section>

    <!-- 混淆 txt -->
    <section class="glass-card section-card">
      <div class="section-title">混淆 txt（防网盘按 hash 比对封杀）</div>
      <div class="row">
        <label class="lbl">混入资源说明.txt</label>
        <label style="display: flex; align-items: center; gap: 6px">
          <input type="checkbox" v-model="opts.mix_txt" />
          开启（每个包自动附加随机后缀，同批包 hash 各不相同）
        </label>
      </div>
      <div v-if="opts.mix_txt" class="row">
        <label class="lbl">文案模板</label>
        <input v-model="opts.txt_template" class="input-glass" placeholder="支持 {date} 占位符" />
      </div>
      <p class="hint">
        该 txt 写在压缩包根目录：下载者解压第一眼可见来源与日期；随机后缀保证即使同一天打包，每个包的内容也唯一。
      </p>
    </section>

    <!-- 运行 -->
    <div class="row run-row">
      <button class="btn-glass btn-primary" :disabled="running || starting" @click="start">开始打包</button>
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
      </div>
    </section>

    <!-- 打包结果表 -->
    <section v-if="summary" class="glass-card section-card">
      <div class="section-title">
        打包结果：成功 {{ summary.ok.length }}，失败 {{ summary.failed.length }}，警告
        {{ summary.warns.length }}
      </div>
      <table v-if="summary.ok.length" class="result-table">
        <thead>
          <tr>
            <th>包名</th>
            <th>密码</th>
            <th>源大小</th>
            <th>分卷数</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(r, i) in summary.ok" :key="i">
            <td class="mono">{{ r.name }}.{{ opts.format }}</td>
            <td class="mono">
              {{ r.password ?? "（无密码）" }}
              <button v-if="r.password" class="btn copy-btn" @click="copyPassword(r.password!, i)">
                {{ copiedIdx === i ? "已复制" : "复制" }}
              </button>
            </td>
            <td>{{ fmtSize(r.size_bytes) }}</td>
            <td>{{ r.outputs.length > 1 ? r.outputs.length + " 个" : "不分卷" }}</td>
          </tr>
        </tbody>
      </table>
      <div v-for="([name, reason], i) in summary.failed" :key="'f' + i" class="sum-line lv-error">
        失败：{{ name }} — {{ reason }}
      </div>
      <div v-for="(w, i) in summary.warns" :key="'w' + i" class="sum-line lv-warn">
        警告：{{ w }}
      </div>
      <button class="btn sum-close" @click="summary = null">关闭</button>
    </section>
  </div>
</template>

<style scoped>
.toggle-label { display: inline-flex; align-items: center; gap: 7px; color: var(--text-primary); }
.affix-input { flex: 1 1 180px; min-width: 150px; }
.number-start-input { width: 120px; max-width: 100%; }
.naming-error { color: var(--error); margin: 0; font-size: 13px; }
</style>
