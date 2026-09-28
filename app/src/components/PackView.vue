<script setup lang="ts">
import "../styles.css";
import { computed, nextTick, onMounted, onUnmounted, reactive, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  defaultPackOptions,
  fmtSize,
  nextName,
  sanitizeName,
  type PackSummaryPayload,
} from "../types";

const LEVELS = ["ok", "error", "warn", "skip", "info"] as const;

const opts = reactive(defaultPackOptions());
const items = ref<string[]>([]);
const itemSel = ref(-1);
const running = ref(false);
const logs = ref<{ msg: string; level: string }[]>([]);
const logBox = ref<HTMLElement | null>(null);
const done = ref(0);
const total = ref(0);
const current = ref("");
const summary = ref<PackSummaryPayload | null>(null);
const copiedIdx = ref(-1);

let unlisteners: UnlistenFn[] = [];

// 包名预览：与 Rust next_name 同一算法（宽度保留、溢出进位）。
const namePreview = computed(() => {
  const step = opts.name_step > 0 ? opts.name_step : 1;
  let name = sanitizeName(opts.name_start.trim()) || "Pack";
  const names: string[] = [];
  for (let i = 0; i < items.value.length; i++) {
    names.push(name);
    name = nextName(name, step);
  }
  return names;
});

const validNameStart = computed(() => sanitizeName(opts.name_start.trim()).length > 0);

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
  const picked = await invoke<string | null>("pick_directory", {
    current: items.value.length ? items.value[items.value.length - 1] : "",
  });
  if (picked && !items.value.includes(picked)) items.value.push(picked);
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
  if (running.value) return;
  if (!items.value.length) {
    appendLog("[警告] 请先添加要打包的文件或目录", "warn");
    return;
  }
  if (!validNameStart.value) {
    appendLog("[警告] 请填写有效的起始包名", "warn");
    return;
  }
  if (!opts.output_dir.trim()) {
    appendLog("[警告] 请先选择打包输出目录", "warn");
    return;
  }
  if (opts.password_mode === "manual" && !opts.uniform_password.trim()) {
    appendLog("[警告] 统一密码模式：密码为空将按无密码打包", "warn");
  }
  try {
    logs.value = [];
    summary.value = null;
    done.value = 0;
    total.value = 0;
    current.value = "";
    running.value = true;
    await invoke("start_pack", { options: { ...opts }, files: [...items.value] });
  } catch (e) {
    running.value = false;
    appendLog(`[失败] ${e}`, "error");
  }
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

onUnmounted(() => {
  for (const u of unlisteners) u();
  unlisteners = [];
});
</script>

<template>
  <div class="page">
    <h1 class="title">打包压缩</h1>

    <!-- 待打包列表 -->
    <section class="group">
      <div class="group-title">待打包列表（每个条目单独打一个包，按顺序命名）</div>
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
            <span class="pack-name">{{ namePreview[i] }}.{{ opts.format }}</span>
          </li>
        </ul>
        <div class="btns">
          <button class="btn" @click="addFiles">添加文件</button>
          <button class="btn" @click="addDir">添加目录</button>
          <button class="btn" :disabled="itemSel < 0" @click="removeItem">移除</button>
          <button class="btn" :disabled="itemSel < 0" @click="moveItem(-1)">上移</button>
          <button class="btn" :disabled="itemSel < 0" @click="moveItem(1)">下移</button>
          <button class="btn" :disabled="!items.length" @click="clearItems">清空</button>
        </div>
      </div>
      <p class="hint">右侧为包名预览：打包时从起始名开始，末尾数字按步长逐个递增。</p>
    </section>

    <!-- 命名与输出 -->
    <section class="group">
      <div class="group-title">命名与输出</div>
      <div class="row">
        <label class="lbl">起始包名</label>
        <input v-model="opts.name_start" class="input" placeholder="如 G198、H113、HG19238" />
        <label class="lbl" style="min-width: 36px">步长</label>
        <input v-model.number="opts.name_step" type="number" min="1" class="input num" />
      </div>
      <div class="row">
        <label class="lbl">打包输出目录</label>
        <input v-model="opts.output_dir" class="input" placeholder="压缩包输出到此处" />
        <button class="btn" @click="browseOutDir">浏览…</button>
      </div>
      <div class="row">
        <label class="lbl">压缩格式</label>
        <select v-model="opts.format" class="input select">
          <option value="7z">7z（推荐：AES-256 + 文件名加密）</option>
          <option value="zip">zip（兼容性好，AES-256）</option>
        </select>
      </div>
      <div class="row">
        <label class="lbl">压缩级别</label>
        <select v-model.number="opts.compression_level" class="input select">
          <option :value="0">0 - 仅存储（最快）</option>
          <option :value="1">1</option>
          <option :value="3">3</option>
          <option :value="5">5 - 默认（均衡）</option>
          <option :value="7">7</option>
          <option :value="9">9 - 极限（最慢）</option>
        </select>
      </div>
    </section>

    <!-- 密码 -->
    <section class="group">
      <div class="group-title">压缩密码</div>
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
        <input v-model="opts.uniform_password" class="input" placeholder="留空 = 无密码打包" />
      </div>
      <p class="hint">
        随机密码为 16 位字母数字（剔除易混淆字符），打包完成后在结果表中显示，可复制。
      </p>
    </section>

    <!-- 分卷 -->
    <section class="group">
      <div class="group-title">分卷打包</div>
      <div class="row">
        <label class="lbl">分卷阈值（GB）</label>
        <input v-model.number="opts.volume_threshold_gb" type="number" min="0" step="0.5" class="input num" />
        <label class="lbl" style="min-width: 96px">每个分卷（GB）</label>
        <input v-model.number="opts.volume_size_gb" type="number" min="0.1" step="0.5" class="input num" />
      </div>
      <p class="hint">内容总大小超过阈值才分卷（压缩后大小不可预知，按源大小判断）。默认超过 3 GB 分卷为 2 GB/个。</p>
    </section>

    <!-- 混淆 txt -->
    <section class="group">
      <div class="group-title">混淆 txt（防网盘按 hash 比对封杀）</div>
      <div class="row">
        <label class="lbl">混入资源说明.txt</label>
        <label style="display: flex; align-items: center; gap: 6px">
          <input type="checkbox" v-model="opts.mix_txt" />
          开启（每个包自动附加随机后缀，同批包 hash 各不相同）
        </label>
      </div>
      <div v-if="opts.mix_txt" class="row">
        <label class="lbl">文案模板</label>
        <input v-model="opts.txt_template" class="input" placeholder="支持 {date} 占位符" />
      </div>
      <p class="hint">
        该 txt 写在压缩包根目录：下载者解压第一眼可见来源与日期；随机后缀保证即使同一天打包，每个包的内容也唯一。
      </p>
    </section>

    <!-- 运行 -->
    <div class="row run-row">
      <button class="btn accent" :disabled="running" @click="start">开始打包</button>
      <button class="btn danger" :disabled="!running" @click="stop">停止</button>
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
      <div class="group-title">运行日志</div>
      <div ref="logBox" class="log-view">
        <div v-for="(l, i) in logs" :key="i" class="log-line" :class="'lv-' + l.level">
          {{ l.msg }}
        </div>
      </div>
    </section>

    <!-- 打包结果表 -->
    <section v-if="summary" class="group">
      <div class="group-title">
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
