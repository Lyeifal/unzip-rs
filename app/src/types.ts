// 与 unzip_core::config::Config / PasswordRule 字段一一对应。
export interface PasswordRule {
  suffix: string;
  keyword: string;
  passwords: string[];
}

export interface Config {
  source_dir: string;
  output_dir: string;
  failed_dir: string;
  seven_zip: string;
  unrar: string;
  lz4: string;
  passwords: string[];
  password_rules: PasswordRule[];
  password_strategy: string; // list_order | recent_first
  recursive: boolean;
  product_exts: string[];
  max_depth: number;
}

// 后端 uz-done 事件载荷：ok/failed/skipped 为 [名称, 备注/原因] 二元组。
export interface SummaryPayload {
  ok: [string, string][];
  failed: [string, string][];
  skipped: [string, string][];
  warns: string[];
}

export function defaultConfig(): Config {
  return {
    source_dir: "",
    output_dir: "",
    failed_dir: "",
    seven_zip: String.raw`C:\Program Files\7-Zip\7z.exe`,
    unrar: String.raw`C:\Program Files\WinRAR\UnRAR.exe`,
    lz4: "",
    passwords: [],
    password_rules: [],
    password_strategy: "recent_first",
    recursive: true,
    product_exts: [".apk", ".xapk", ".apks", ".aab"],
    max_depth: 5,
  };
}

// 镜像 unzip_core::config::format_rule 的文案（逐字一致）。
export function formatRule(rule: PasswordRule): string {
  const suffix = rule.suffix.trim();
  const keyword = rule.keyword.trim();
  const cond = suffix
    ? `后缀为 ${suffix}`
    : keyword
      ? `文件名或路径含「${keyword}」`
      : "全部压缩包";
  const pws = rule.passwords.length ? rule.passwords.join("、") : "（无密码）";
  return `${cond}  →  优先尝试：${pws}`;
}

// ---------------------------------------------------------------------------
// 打包压缩（与 unzip_core::pack 一一对应）
// ---------------------------------------------------------------------------

export interface PackOptions {
  format: string; // "7z" | "zip"
  password_mode: string; // random_per_pack | random_uniform | manual
  uniform_password: string;
  volume_threshold_gb: number;
  volume_size_gb: number;
  name_start: string;
  name_step: number;
  mix_txt: boolean;
  txt_template: string; // 支持 {date} 占位符
  compression_level: number; // 0-9
  output_dir: string;
}

export function defaultPackOptions(): PackOptions {
  return {
    format: "7z",
    password_mode: "random_per_pack",
    uniform_password: "",
    volume_threshold_gb: 3,
    volume_size_gb: 2,
    name_start: "G001",
    name_step: 1,
    mix_txt: true,
    txt_template: "本资源于 {date} 打包整理，解压密码请查看发布页面。",
    compression_level: 5,
    output_dir: "",
  };
}

// 后端 pack-done 事件载荷。
export interface PackResultPayload {
  name: string;
  password: string | null;
  size_bytes: number;
  outputs: string[];
}

export interface PackSummaryPayload {
  ok: PackResultPayload[];
  failed: [string, string][];
  warns: string[];
}

// 镜像 unzip_core::pack::next_name：末尾数字 +step 递增，保留原宽度（G198→G199，Z999→Z1000）。
export function nextName(name: string, step: number): string {
  const m = name.match(/^(.*?)(\d+)$/);
  if (!m) return `${name}${step}`;
  const width = m[2].length;
  const next = (parseInt(m[2], 10) + step).toString();
  return m[1] + next.padStart(Math.max(width, next.length), "0");
}

// 镜像 unzip_core::pack::sanitize_name：Windows 文件名非法字符与控制字符 → _。
export function sanitizeName(name: string): string {
  return name.replace(/[\/:*?"<>|]/g, "_").trim();
}

export function fmtSize(n: number): string {
  const GB = 1024 ** 3;
  const MB = 1024 ** 2;
  if (n >= GB) return `${(n / GB).toFixed(1)} GB`;
  if (n >= MB) return `${(n / MB).toFixed(1)} MB`;
  return `${n} B`;
}
