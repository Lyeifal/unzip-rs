//! config.json 读写（schema 稳定，旧配置可直接导入）。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PasswordRule {
    #[serde(default)]
    pub suffix: String,
    #[serde(default)]
    pub keyword: String,
    #[serde(default)]
    pub passwords: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub source_dir: String,
    pub output_dir: String, // 空 = exe 旁「解压结果」
    pub failed_dir: String, // 空 = <输出目录>/解压失败
    pub seven_zip: String,
    pub unrar: String,
    pub lz4: String, // 可选回退；默认可留空走内置解码
    pub passwords: Vec<String>,
    pub password_rules: Vec<PasswordRule>, // 匹配的规则按序前置
    pub password_strategy: String,         // list_order | recent_first
    pub recursive: bool,
    pub product_exts: Vec<String>,
    pub max_depth: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            source_dir: String::new(),
            output_dir: String::new(),
            failed_dir: String::new(),
            seven_zip: r"C:\Program Files\7-Zip\7z.exe".to_string(),
            unrar: r"C:\Program Files\WinRAR\UnRAR.exe".to_string(),
            lz4: String::new(),
            passwords: Vec::new(),
            password_rules: Vec::new(),
            password_strategy: "recent_first".to_string(),
            recursive: true,
            product_exts: vec![
                ".apk".to_string(),
                ".xapk".to_string(),
                ".apks".to_string(),
                ".aab".to_string(),
            ],
            max_depth: 5,
        }
    }
}

/// config.json 位置：优先环境变量 UNZIP_CONFIG_PATH（GUI 模式指向应用数据目录），
/// 默认 exe 所在目录（便携，配置与 exe 同目录）。
pub fn config_path() -> PathBuf {
    if let Some(p) = std::env::var_os("UNZIP_CONFIG_PATH") {
        return PathBuf::from(p);
    }
    exe_dir().join("config.json")
}

pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::env::temp_dir())
}

/// 输出目录未配置时的默认位置：exe 旁「解压结果」。
pub fn default_output_dir() -> PathBuf {
    exe_dir().join("解压结果")
}

pub fn load_config() -> Config {
    load_config_from(&config_path())
}

/// 从指定路径读配置；缺文件/坏 JSON 都静默回退默认。
pub fn load_config_from(path: &Path) -> Config {
    let mut cfg = Config::default();
    if let Ok(text) = std::fs::read_to_string(path) {
        if let Ok(loaded) = serde_json::from_str::<Config>(&text) {
            cfg = loaded;
        }
    }
    cfg
}

/// 原子写（tmp + rename），失败返回 false。
pub fn save_config(cfg: &Config) -> bool {
    save_config_to(cfg, &config_path())
}

pub fn save_config_to(cfg: &Config, path: &Path) -> bool {
    let text = match serde_json::to_string_pretty(cfg) {
        Ok(t) => t,
        Err(_) => return false,
    };
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, text).is_err() {
        return false;
    }
    std::fs::rename(&tmp, path).is_ok()
}

/// 最近成功优先：把命中密码置顶并落盘。
pub fn promote_password(cfg: &mut Config, pw: &str) {
    if let Some(pos) = cfg.passwords.iter().position(|p| p == pw) {
        cfg.passwords.remove(pos);
    }
    cfg.passwords.insert(0, pw.to_string());
    save_config(cfg);
}

/// 规则的可读描述（GUI/CLI 展示用，措辞固定）。
pub fn format_rule(rule: &PasswordRule) -> String {
    let cond = if !rule.suffix.trim().is_empty() {
        format!("后缀为 {}", rule.suffix.trim())
    } else if !rule.keyword.trim().is_empty() {
        format!("文件名或路径含「{}」", rule.keyword.trim())
    } else {
        "全部压缩包".to_string()
    };
    let pws = if rule.passwords.is_empty() {
        "（无密码）".to_string()
    } else {
        rule.passwords.join("、")
    };
    format!("{}  →  优先尝试：{}", cond, pws)
}
