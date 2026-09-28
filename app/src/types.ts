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
