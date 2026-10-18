// 与 unzip_core::config::Config / PasswordRule 字段一一对应。
export interface PasswordRule {
  suffix: string;
  keyword: string;
  passwords: string[];
  enabled?: boolean; // 旧配置缺省时视为启用
}

export interface Config {
  source_dir: string;
  output_dir: string;
  failed_dir: string;
  failed_dir_enabled: boolean;
  seven_zip: string;
  unrar: string;
  lz4: string;
  passwords: string[];
  password_rules: PasswordRule[];
  password_strategy: string; // list_order | recent_first
  hash_match: boolean;
  recursive: boolean;
  product_exts: string[];
  max_depth: number;
  quick_pack_format: string;
  shell_show_progress: boolean;
  shell_show_console: boolean;
}

// 后端 uz-done 事件载荷：ok/failed/skipped 为 [名称, 备注/原因] 二元组。
export interface SummaryPayload {
  ok: [string, string][];
  failed: [string, string][];
  skipped: [string, string][];
  warns: string[];
  retryable: { slot: number; failed_index: number; name: string }[];
}

export function defaultConfig(): Config {
  return {
    source_dir: "",
    output_dir: "",
    failed_dir: "",
    failed_dir_enabled: true,
    seven_zip: String.raw`C:\Program Files\7-Zip\7z.exe`,
    unrar: String.raw`C:\Program Files\WinRAR\UnRAR.exe`,
    lz4: "",
    passwords: [],
    password_rules: [],
    password_strategy: "recent_first",
    hash_match: true,
    recursive: true,
    product_exts: [".apk", ".xapk", ".apks", ".aab"],
    max_depth: 5,
    quick_pack_format: '7z',
    shell_show_progress: true,
    shell_show_console: false,
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
  return `${rule.enabled === false ? '（已禁用）' : ''}${cond}  →  优先尝试：${pws}`;
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
  name_step: number;
  name_mode: string; // 桌面端固定 parts；保留后端协议字段
  name_prefix: string;
  name_suffix: string;
  prefix_enabled: boolean;
  suffix_enabled: boolean;
  number_start: string; // ASCII digits, width preserved
  auto_number: boolean;
  mix_txt: boolean;
  txt_template: string; // 支持 {date} 占位符
  compression_level: number; // 0-9
  output_dir: string;
}

export function defaultPackOptions(): PackOptions {
  // 尝试从 localStorage 加载上次的配置
  const saved = localStorage.getItem("pack_options");
  if (saved) {
    try {
      const parsed = JSON.parse(saved);
      const oldName = typeof parsed.name_start === 'string' ? parsed.name_start.trim() : 'G001';
      const oldParts = /^(.*?)(\d+)$/.exec(oldName);
      const legacy = parsed.name_mode === 'legacy' || (!parsed.name_mode && typeof parsed.name_start === 'string');
      const migratedPrefix = oldParts?.[1] ?? oldName;
      const migratedNumber = oldParts?.[2] ?? '001';
      // 合并默认值，确保新增字段有默认值
      return {
        format: parsed.format || "7z",
        password_mode: parsed.password_mode || "random_per_pack",
        uniform_password: parsed.uniform_password || "",
        volume_threshold_gb: parsed.volume_threshold_gb ?? 3,
        volume_size_gb: parsed.volume_size_gb ?? 2,
        name_step: parsed.name_step ?? 1,
        name_mode: 'parts',
        name_prefix: legacy ? migratedPrefix : (parsed.name_prefix ?? oldParts?.[1] ?? 'G'),
        name_suffix: legacy ? '' : (parsed.name_suffix ?? ''),
        prefix_enabled: legacy ? !!migratedPrefix : (parsed.prefix_enabled ?? !!oldParts?.[1]),
        suffix_enabled: legacy ? false : (parsed.suffix_enabled ?? false),
        number_start: legacy ? migratedNumber : (parsed.number_start ?? oldParts?.[2] ?? '001'),
        auto_number: parsed.auto_number ?? true,
        mix_txt: parsed.mix_txt ?? true,
        txt_template: parsed.txt_template || "本资源于 {date} 打包整理，解压密码请查看发布页面。",
        compression_level: parsed.compression_level ?? 5,
        output_dir: parsed.output_dir || "",
      };
    } catch {
      // 解析失败，返回默认值
    }
  }

  return {
    format: "7z",
    password_mode: "random_per_pack",
    uniform_password: "",
    volume_threshold_gb: 3,
    volume_size_gb: 2,
    name_step: 1,
    name_mode: 'parts',
    name_prefix: 'G',
    name_suffix: '',
    prefix_enabled: true,
    suffix_enabled: false,
    number_start: '001',
    auto_number: true,
    mix_txt: true,
    txt_template: "本资源于 {date} 打包整理，解压密码请查看发布页面。",
    compression_level: 5,
    output_dir: "",
  };
}

// 保存打包配置到 localStorage
export function savePackOptions(opts: PackOptions): void {
  try {
    localStorage.setItem("pack_options", JSON.stringify(opts));
  } catch (e) {
    console.error("保存打包配置失败：", e);
  }
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
  return name.replace(/[\/:*?"<>|\x00-\x1f]/g, "_").trim();
}

export function fmtSize(n: number): string {
  const GB = 1024 ** 3;
  const MB = 1024 ** 2;
  if (n >= GB) return `${(n / GB).toFixed(1)} GB`;
  if (n >= MB) return `${(n / MB).toFixed(1)} MB`;
  return `${n} B`;
}

// ---------------------------------------------------------------------------
// 压缩包管理（AppData 集中 SQLite，兼容旧目录 JSON 映射）
// ---------------------------------------------------------------------------

export type EngineType =
  | "Unity"
  | "Unreal"
  | "RPG Maker"
  | "Godot"
  | "自研"
  | "其他"
  | "";  // 未标记

export interface GameMetadata {
  version?: string;             // 兼容旧版；显示内容迁移到备注
  engine?: string;              // 兼容旧版引擎标记
  is_apk?: boolean;             // 兼容旧版 APK 标记
  custom_tags?: string[];       // 自定义标签（可选）
  notes?: string;               // 备注（可选）
  custom_fields?: Record<string, string | boolean>;
  schema_version?: number;
  [key: string]: unknown;
}

export interface GameMapping {
  record_id?: string;
  archive_path?: string;
  output_dir?: string;
  output_hashes?: { file_name: string; size_bytes: number; sha256: string }[];
  source_name: string;
  archive_name: string;
  password: string | null;
  source_size: {
    bytes: number;
    mb: string;
    gb: string;
    human: string;
  };
  compressed_size: {
    bytes: number;
    mb: string;
    gb: string;
    human: string;
  };
  compression_ratio: string;
  is_volume: boolean;
  volume_count: number;
  created_at: string;
  metadata?: GameMetadata;  // 用户手动标记的元数据
}

export interface PackMappingFile {
  generated_at: string;
  total_count: number;
  mappings: GameMapping[];
}

export interface ArchiveLibraryPage {
  items: GameMapping[];
  page: number;
  page_size: number;
  total: number;
  stats: {
    total: number;
    source_bytes: number;
    compressed_bytes: number;
    avg_ratio: number;
  };
  filter_options: Record<string, string[]>;
}

// 引擎标签颜色映射
export const ENGINE_COLORS: Record<EngineType, string> = {
  "Unity": "var(--engine-unity)",
  "Unreal": "var(--engine-unreal)",
  "RPG Maker": "var(--engine-rpgmaker)",
  "Godot": "var(--engine-godot)",
  "自研": "var(--engine-custom)",
  "其他": "var(--engine-other)",
  "": "var(--text-secondary)",
};
