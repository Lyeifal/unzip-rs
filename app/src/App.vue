<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, reactive, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  defaultConfig,
  formatRule,
  type Config,
  type SummaryPayload,
} from "./types";

const LEVELS = ["ok", "error", "warn", "skip", "info"] as const;

type DirKey = "source_dir" | "output_dir" | "failed_dir";
type FileKey = "seven_zip" | "unrar" | "lz4";

const dirRows: { key: DirKey; label: string; hint: string }[] = [
  { key: "source_dir", label: "压缩包所在目录", hint: "源目录（递归扫描，分卷可散落各处）" },
  { key: "output_dir", label: "解压文件所在目录", hint: "每个游戏/安装包一个独立文件夹" },
  { key: "failed_dir", label: "解压失败文件所在目录", hint: "失败的压缩包连同分卷移入此处" },
];

const fileRows: { key: FileKey; label: string }[] = [
  { key: "seven_zip", label: "7-Zip 路径" },
  { key: "unrar", label: "UnRAR 路径" },
  { key: "lz4", label: "lz4.exe 回退路径（可选，默认可留空）" },
];

const cfg = reactive<Config>(defaultConfig());
const running = ref(false);
const logs = ref<{ msg: string; level: string }[]>([]);
const logBox = ref<HTMLElement | null>(null);
const done = ref(0);
const total = ref(0);
const current = ref("");
const summary = ref<SummaryPayload | null>(null);

const pwSel = ref(-1);
const ruleSel = ref(-1);

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

// ---------- 日志 ----------

function appendLog(msg: string, level: string = "info") {
  logs.value.push({ msg, level: LEVELS.includes(level as (typeof LEVELS)[number]) ? level : "info" });
  nextTick(() => {
    const el = logBox.value;
    if (el) el.scrollTop = el.scrollHeight;
  });
}

// ---------- 目录/文件选择 ----------

async function browseDir(key: DirKey) {
  const picked = await invoke<string | null>("pick_directory", {
    current: cfg[key],
  });
  if (picked) cfg[key] = picked;
}

async function browseFile(key: FileKey) {
  const picked = await invoke<string | null>("pick_file", {
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
    rule = { suffix: sfx, keyword: "", passwords: pws };
  } else {
    const kw = ruleValue.value.trim();
    if (!kw) {
      ruleErr.value = "请填写关键词";
      return;
    }
    rule = { suffix: "", keyword: kw, passwords: pws };
  }
  if (ruleEditIdx.value >= 0) {
    cfg.password_rules[ruleEditIdx.value] = rule;
  } else {
    cfg.password_rules.push(rule);
  }
  ruleDlg.value = false;
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
  if (running.value) return;
  const src = cfg.source_dir.trim();
  if (!src) {
    appendLog("[警告] 请先选择有效的压缩包所在目录", "warn");
    return;
  }
  if (!cfg.output_dir.trim()) {
    appendLog("[警告] 请先选择解压文件所在目录", "warn");
    return;
  }
  cfg.recursive = true;
  try {
    await invoke("save_config", { cfg });
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
    });
  } catch (e) {
    running.value = false;
    appendLog(`[失败] ${e}`, "error");
  }
}

async function stop() {
  await invoke("cancel_run");
  appendLog("[警告] 正在停止…", "warn");
}

function onDone(s: SummaryPayload) {
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

// ---------- 生命周期 ----------

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
  );
});

onUnmounted(() => {
  for (const u of unlisteners) u();
  unlisteners = [];
});
</script>

<template>
  <div class="root">
    <h1 class="title">自动解压工具</h1>

    <!-- 目录设置 -->
    <section class="group">
      <div class="group-title">目录设置</div>
      <div v-for="row in dirRows" :key="row.key" class="row">
        <label class="lbl">{{ row.label }}</label>
        <input v-model="cfg[row.key]" class="input" :placeholder="row.hint" />
        <button class="btn" @click="browseDir(row.key)">浏览…</button>
      </div>
    </section>

    <!-- 密码库 -->
    <section class="group">
      <div class="group-title">密码库（自上而下依次尝试）</div>
      <div class="split">
        <ul class="list" @click="pwSel = -1">
          <li
            v-for="(pw, i) in cfg.passwords"
            :key="i"
            :class="{ sel: i === pwSel }"
            @click.stop="pwSel = i"
          >
            {{ pw }}
          </li>
        </ul>
        <div class="btns">
          <button class="btn" @click="pwAdd">添加</button>
          <button class="btn" :disabled="pwSel < 0" @click="pwDel">删除</button>
          <button class="btn" :disabled="pwSel < 0" @click="pwMove(-1)">上移</button>
          <button class="btn" :disabled="pwSel < 0" @click="pwMove(1)">下移</button>
        </div>
      </div>
    </section>

    <!-- 密码策略规则 -->
    <section class="group">
      <div class="group-title">
        密码策略规则（命中的规则优先尝试，未命中走上方密码库顺序）
      </div>
      <div class="split">
        <ul class="list" @click="ruleSel = -1">
          <li
            v-for="(r, i) in cfg.password_rules"
            :key="i"
            :class="{ sel: i === ruleSel }"
            @click.stop="ruleSel = i"
          >
            {{ formatRule(r) }}
          </li>
        </ul>
        <div class="btns">
          <button class="btn" @click="ruleOpen(-1)">添加</button>
          <button class="btn" :disabled="ruleSel < 0" @click="ruleOpen(ruleSel)">编辑</button>
          <button class="btn" :disabled="ruleSel < 0" @click="ruleDel">删除</button>
          <button class="btn" :disabled="ruleSel < 0" @click="ruleMove(-1)">上移</button>
          <button class="btn" :disabled="ruleSel < 0" @click="ruleMove(1)">下移</button>
        </div>
      </div>
    </section>

    <!-- 密码排序规则 -->
    <div class="row">
      <label class="lbl">密码排序规则：</label>
      <select v-model="cfg.password_strategy" class="input select">
        <option value="recent_first">最近成功优先（自动置顶，越用越准）</option>
        <option value="list_order">严格按列表顺序</option>
      </select>
    </div>

    <!-- 高级设置 -->
    <details class="group adv">
      <summary class="group-title adv-title">高级设置</summary>
      <div v-for="row in fileRows" :key="row.key" class="row">
        <label class="lbl">{{ row.label }}</label>
        <input v-model="cfg[row.key]" class="input" />
        <button class="btn" @click="browseFile(row.key)">浏览…</button>
      </div>
      <div class="row">
        <label class="lbl">产物后缀（不解压，原样放入）</label>
        <input v-model="productExtsText" class="input" />
      </div>
    </details>

    <!-- 运行 -->
    <div class="row run-row">
      <button class="btn accent" :disabled="running" @click="start">开始解压</button>
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
        <div v-if="summary" class="summary">
          <div class="sum-head">
            成功 {{ summary.ok.length }}，失败 {{ summary.failed.length }}，跳过
            {{ summary.skipped.length }}，警告 {{ summary.warns.length }}
          </div>
          <div v-for="([name, reason], i) in summary.failed" :key="'f' + i" class="sum-line lv-error">
            失败：{{ name }} — {{ reason }}
          </div>
          <div v-for="(w, i) in summary.warns" :key="'w' + i" class="sum-line lv-warn">
            警告：{{ w }}
          </div>
          <button class="btn sum-close" @click="summary = null">关闭</button>
        </div>
      </div>
    </section>

    <!-- 添加密码弹窗 -->
    <div v-if="pwDlg" class="overlay" @click.self="pwDlg = false">
      <div class="dialog">
        <div class="dlg-title">添加密码</div>
        <div class="row">
          <label class="lbl">输入常用密码：</label>
          <input
            v-model="pwInput"
            class="input"
            autofocus
            @keyup.enter="pwAddOk"
          />
        </div>
        <div class="row dlg-btns">
          <button class="btn accent" @click="pwAddOk">确定</button>
          <button class="btn" @click="pwDlg = false">取消</button>
        </div>
      </div>
    </div>

    <!-- 密码策略规则弹窗 -->
    <div v-if="ruleDlg" class="overlay" @click.self="ruleDlg = false">
      <div class="dialog">
        <div class="dlg-title">密码策略规则</div>
        <div class="row">
          <label class="lbl">条件类型</label>
          <select v-model="ruleKind" class="input select">
            <option value="suffix">按压缩包后缀（如 .lz4、.zip）</option>
            <option value="keyword">按文件名或路径关键词</option>
          </select>
        </div>
        <div class="row">
          <label class="lbl">条件值</label>
          <input
            v-model="ruleValue"
            class="input"
            :placeholder="ruleKind === 'suffix' ? '.lz4（后缀）' : '如 游戏、汉化、整合'"
          />
        </div>
        <div class="row">
          <label class="lbl">优先密码</label>
          <input
            v-model="rulePws"
            class="input"
            placeholder="多个密码用逗号或空格分隔，按顺序优先尝试"
            @keyup.enter="ruleOk"
          />
        </div>
        <div v-if="ruleErr" class="dlg-err">{{ ruleErr }}</div>
        <div class="row dlg-btns">
          <button class="btn accent" @click="ruleOk">确定</button>
          <button class="btn" @click="ruleDlg = false">取消</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style>
* {
  box-sizing: border-box;
}

html,
body,
#app {
  margin: 0;
  padding: 0;
  height: 100%;
}

body {
  background-color: #1b1f26;
  color: #e6e9ef;
  font-family: "Microsoft YaHei UI", "Segoe UI", sans-serif;
  font-size: 13px;
}

.root {
  max-width: 960px;
  margin: 0 auto;
  padding: 12px 14px 16px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.title {
  font-size: 20px;
  font-weight: bold;
  color: #e6e9ef;
  margin: 0;
}

.group {
  border: 1px solid #2d333d;
  border-radius: 8px;
  margin-top: 6px;
  padding: 10px 12px 12px;
  background-color: #22262f;
}

.group-title {
  color: #8ab4f8;
  font-weight: bold;
  margin: -2px 0 8px;
}

.row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 6px 0;
}

.lbl {
  flex: 0 0 auto;
  min-width: 120px;
}

.input {
  flex: 1;
  background-color: #161a20;
  border: 1px solid #333a45;
  border-radius: 5px;
  padding: 6px 8px;
  color: #e6e9ef;
  font-family: inherit;
  font-size: 13px;
  outline: none;
}

.input:focus {
  border-color: #3b8eed;
}

.select {
  flex: 1;
  appearance: auto;
}

.btn {
  background-color: #2c3440;
  border: 1px solid #3a4350;
  border-radius: 6px;
  color: #e6e9ef;
  padding: 6px 16px;
  cursor: pointer;
  font-family: inherit;
  font-size: 13px;
}

.btn:hover {
  background-color: #364052;
}

.btn:active {
  background-color: #2a3547;
}

.btn:disabled {
  color: #6b7380;
  background-color: #242932;
  cursor: default;
}

.btn.accent {
  background-color: #2f81f7;
  border-color: #2f81f7;
  color: #fff;
  font-weight: bold;
  padding: 9px 28px;
}

.btn.accent:hover {
  background-color: #3d8bfd;
}

.btn.danger {
  background-color: #7a2e2e;
  border-color: #a03e3e;
  color: #ffd7d7;
}

.btn.danger:hover {
  background-color: #933;
}

.split {
  display: flex;
  gap: 8px;
  align-items: stretch;
}

.list {
  flex: 1;
  margin: 0;
  padding: 0;
  list-style: none;
  background-color: #161a20;
  border: 1px solid #333a45;
  border-radius: 5px;
  max-height: 150px;
  overflow-y: auto;
}

.list li {
  padding: 4px 8px;
  cursor: default;
  user-select: none;
}

.list li.sel {
  background-color: #2f4b73;
}

.btns {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.btns .btn {
  padding: 4px 14px;
}

.adv summary {
  cursor: pointer;
}

.run-row {
  margin-top: 10px;
}

.current {
  flex: 1;
  color: #8ab4f8;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.bar {
  border: 1px solid #333a45;
  border-radius: 6px;
  background-color: #161a20;
  height: 18px;
  overflow: hidden;
}

.bar-inner {
  height: 100%;
  background-color: #2f81f7;
  border-radius: 5px;
  transition: width 0.15s;
}

.log-group {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 180px;
}

.log-view {
  flex: 1;
  background-color: #14171d;
  border: 1px solid #2d333d;
  border-radius: 6px;
  font-family: "Cascadia Mono", Consolas, monospace;
  font-size: 12px;
  padding: 6px 8px;
  overflow-y: auto;
  min-height: 160px;
  max-height: 320px;
}

.log-line {
  white-space: pre-wrap;
  word-break: break-all;
}

.lv-ok {
  color: #5ad18b;
}

.lv-error {
  color: #ff6b6b;
}

.lv-warn {
  color: #ffc857;
}

.lv-skip {
  color: #9aa4b2;
}

.lv-info {
  color: #c9d1d9;
}

.summary {
  margin-top: 8px;
  border: 1px solid #3a4350;
  border-radius: 6px;
  padding: 8px 10px;
  background-color: #1b1f26;
}

.sum-head {
  color: #8ab4f8;
  font-weight: bold;
  margin-bottom: 4px;
}

.sum-line {
  padding: 1px 0;
}

.sum-close {
  margin-top: 6px;
  padding: 3px 12px;
  font-size: 12px;
}

.overlay {
  position: fixed;
  inset: 0;
  background-color: rgba(0, 0, 0, 0.55);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.dialog {
  background-color: #22262f;
  border: 1px solid #3a4350;
  border-radius: 8px;
  padding: 14px 16px;
  width: 460px;
  max-width: 92vw;
}

.dlg-title {
  color: #8ab4f8;
  font-weight: bold;
  font-size: 14px;
  margin-bottom: 8px;
}

.dlg-btns {
  justify-content: flex-end;
  margin-top: 12px;
}

.dlg-err {
  color: #ff6b6b;
  margin: 4px 0;
}
</style>
