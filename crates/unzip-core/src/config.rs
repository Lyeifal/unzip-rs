//! config.json 读写（schema 稳定，旧配置可直接导入）。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// 新规则与旧配置（无 enabled 字段）都默认启用。
fn default_rule_enabled() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PasswordRule {
    #[serde(default)]
    pub suffix: String,
    #[serde(default)]
    pub keyword: String,
    #[serde(default)]
    pub passwords: Vec<String>,
    #[serde(default = "default_rule_enabled")]
    pub enabled: bool,
}

impl Default for PasswordRule {
    fn default() -> Self {
        PasswordRule {
            suffix: String::new(),
            keyword: String::new(),
            passwords: Vec::new(),
            enabled: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub source_dir: String,
    pub output_dir: String, // 空 = exe 旁 unpacked
    pub failed_dir: String, // 空 = <输出目录>/failed
    pub failed_dir_enabled: bool, // false = 失败源文件保留原位
    pub seven_zip: String,
    pub unrar: String,
    pub lz4: String, // 可选回退；默认可留空走内置解码
    pub passwords: Vec<String>,
    pub password_rules: Vec<PasswordRule>, // 匹配的规则按序前置
    pub password_strategy: String,         // list_order | recent_first
    pub hash_match: bool, // 优先使用内容 SHA-256 命中的记录密码
    pub recursive: bool,
    pub product_exts: Vec<String>,
    pub max_depth: u32,
    pub quick_pack_format: String, // Explorer quick pack: 7z | zip
    pub shell_show_progress: bool, // Show the application for direct Explorer tasks.
    pub shell_show_console: bool, // Opt-in persistent terminal logs for direct tasks.
    #[serde(skip)]
    pub retry_password_only: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            source_dir: String::new(),
            output_dir: String::new(),
            failed_dir: String::new(),
            failed_dir_enabled: true,
            seven_zip: r"C:\Program Files\7-Zip\7z.exe".to_string(),
            unrar: r"C:\Program Files\WinRAR\UnRAR.exe".to_string(),
            lz4: String::new(),
            passwords: Vec::new(),
            password_rules: Vec::new(),
            password_strategy: "recent_first".to_string(),
            hash_match: true,
            recursive: true,
            product_exts: vec![
                ".apk".to_string(),
                ".xapk".to_string(),
                ".apks".to_string(),
                ".aab".to_string(),
            ],
            max_depth: 5,
            quick_pack_format: "7z".into(),
            shell_show_progress: true,
            shell_show_console: false,
            retry_password_only: false,
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

/// 输出目录未配置时的默认位置：exe 旁 unpacked。
pub fn default_output_dir() -> PathBuf {
    exe_dir().join("unpacked")
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
    let _guard = CONFIG_WRITE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    save_config_to_unlocked(cfg, path)
}

static CONFIG_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Merge a change into the latest on-disk config under the same writer lock.
/// The update closure must not call another config-writing function.
pub fn update_config_to(path: &Path, update: impl FnOnce(&mut Config)) -> bool {
    let _guard = CONFIG_WRITE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut cfg = load_config_from(path);
    update(&mut cfg);
    save_config_to_unlocked(&cfg, path)
}

fn save_config_to_unlocked(cfg: &Config, path: &Path) -> bool {
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
    let path = config_path();
    persist_promoted_password(cfg, pw, &path);
}

fn persist_promoted_password(cfg: &Config, pw: &str, path: &Path) {
    update_config_to(path, |latest| {
        if !path.is_file() { *latest = cfg.clone(); }
        if latest.password_strategy != "recent_first" { return; }
        latest.passwords.retain(|value| value != pw);
        latest.passwords.insert(0, pw.to_string());
    });
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
    let disabled = if rule.enabled { "" } else { "（已禁用）" };
    format!("{}{}  →  优先尝试：{}", disabled, cond, pws)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_config_keeps_failed_directory_enabled_and_retry_mode_is_transient() {
        let old: Config = serde_json::from_str(r#"{"failed_dir":"C:/failed"}"#).unwrap();
        assert!(old.failed_dir_enabled);
        let disabled: Config = serde_json::from_str(r#"{"failed_dir_enabled":false}"#).unwrap();
        assert!(!disabled.failed_dir_enabled);
        let mut runtime = disabled;
        runtime.retry_password_only = true;
        let saved = serde_json::to_string(&runtime).unwrap();
        assert!(!saved.contains("retry_password_only"));
    }
    #[test]
    fn password_rule_without_enabled_field_deserializes_as_enabled() {
        // 旧 config.json 的规则没有 enabled 字段：必须反序列化为启用，行为与旧版一致。
        let old: PasswordRule =
            serde_json::from_str(r#"{"suffix":".zip","keyword":"","passwords":["pw1"]}"#).unwrap();
        assert_eq!(old.suffix, ".zip");
        assert_eq!(old.passwords, vec!["pw1".to_string()]);
        assert!(old.enabled, "缺 enabled 字段的旧规则应默认启用");
        // 新写入的配置带 enabled 字段，可往返。
        let text = serde_json::to_string(&old).unwrap();
        let round: PasswordRule = serde_json::from_str(&text).unwrap();
        assert!(round.enabled);
    }
    #[test]
    fn password_rule_explicitly_disabled_deserializes_as_disabled() {
        let rule: PasswordRule =
            serde_json::from_str(r#"{"suffix":".rar","enabled":false}"#).unwrap();
        assert!(!rule.enabled);
        assert_eq!(rule.passwords, Vec::<String>::new());
    }
    #[test]
    fn password_rule_default_is_enabled() {
        assert!(PasswordRule::default().enabled);
    }
    #[test]
    fn format_rule_marks_disabled_without_changing_passwords() {
        let enabled_rule = PasswordRule {
            suffix: ".zip".into(),
            keyword: String::new(),
            passwords: vec!["pw1".into(), "pw2".into()],
            enabled: true,
        };
        let disabled_rule = PasswordRule {
            enabled: false,
            ..enabled_rule.clone()
        };
        let enabled_text = format_rule(&enabled_rule);
        assert_eq!(enabled_text, "后缀为 .zip  →  优先尝试：pw1、pw2");
        let disabled_text = format_rule(&disabled_rule);
        assert!(
            disabled_text.contains("已禁用"),
            "禁用规则必须显示禁用状态：{disabled_text}"
        );
        assert!(
            disabled_text.ends_with("优先尝试：pw1、pw2"),
            "禁用规则的密码展示不得改变：{disabled_text}"
        );
    }
    #[test]
    fn password_promotion_preserves_settings_saved_after_task_started() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        let stale = Config { seven_zip: "old.exe".into(), passwords: vec!["hit".into(), "old".into()], ..Default::default() };
        assert!(save_config_to(&stale, &path));
        assert!(update_config_to(&path, |cfg| {
            cfg.seven_zip = "new.exe".into(); cfg.output_dir = "new output".into(); cfg.passwords = vec!["added later".into(), "hit".into()];
        }));
        persist_promoted_password(&stale, "hit", &path);
        let latest = load_config_from(&path);
        assert_eq!(latest.seven_zip, "new.exe");
        assert_eq!(latest.output_dir, "new output");
        assert_eq!(latest.passwords, vec!["hit", "added later"]);
        assert!(update_config_to(&path, |cfg| cfg.password_strategy = "list_order".into()));
        let before = std::fs::read(&path).unwrap();
        persist_promoted_password(&stale, "old", &path);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    #[test]
    fn simultaneous_config_updates_do_not_lose_changes_or_share_temp_writes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        assert!(save_config_to(&Config { passwords: vec![], ..Default::default() }, &path));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let threads: Vec<_> = (0..2).map(|worker| {
            let path = path.clone(); let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for index in 0..10 {
                    assert!(update_config_to(&path, |cfg| cfg.passwords.push(format!("{worker}-{index}"))));
                }
            })
        }).collect();
        for thread in threads { thread.join().unwrap(); }
        assert_eq!(load_config_from(&path).passwords.len(), 20);
    }
}
