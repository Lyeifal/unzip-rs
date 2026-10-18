use serde::{Deserialize, Serialize};
use std::path::Path;
use unzip_core::config::{load_config_from, update_config_to, Config};

pub fn find_bundled_tools_dir(resource_root: &Path) -> Option<std::path::PathBuf> {
    [resource_root.join("resources/bundled"), resource_root.join("bundled"), resource_root.to_path_buf()]
        .into_iter()
        .find(|dir| dir.join("7z.exe").is_file() && dir.join("7z.dll").is_file() && dir.join("UnRAR.exe").is_file())
}

pub fn initialize_bundled_tool_paths(config_path: &Path, bundled_dir: &Path) -> bool {
    if config_path.exists() {
        let valid = std::fs::read_to_string(config_path)
            .ok()
            .and_then(|text| serde_json::from_str::<Config>(&text).ok())
            .is_some();
        if !valid { return true; } // 不覆盖无法读取的旧配置，留给用户恢复。
    }
    let defaults = Config::default();
    let current = load_config_from(config_path);
    let seed_seven_zip = !config_path.exists() || current.seven_zip == defaults.seven_zip;
    let seed_unrar = !config_path.exists() || current.unrar == defaults.unrar;
    if !seed_seven_zip && !seed_unrar { return true; }
    update_config_to(config_path, |cfg| {
        if seed_seven_zip && cfg.seven_zip == defaults.seven_zip {
            cfg.seven_zip = bundled_dir.join("7z.exe").to_string_lossy().into_owned();
        }
        if seed_unrar && cfg.unrar == defaults.unrar {
            cfg.unrar = bundled_dir.join("UnRAR.exe").to_string_lossy().into_owned();
        }
        // lz4 保持空值，由 Rust 内置解码器优先处理。
    })
}

pub fn save_unpack_config(path: &Path, cfg: unzip_core::config::Config) -> bool {
    // Tool paths are edited separately, so a cached unpack page cannot undo them.
    update_config_to(path, |latest| {
        let (seven_zip, unrar, lz4) = (
            latest.seven_zip.clone(),
            latest.unrar.clone(),
            latest.lz4.clone(),
        );
        let quick_pack_format = latest.quick_pack_format.clone();
        let shell_display = (latest.shell_show_progress, latest.shell_show_console);
        *latest = cfg;
        latest.seven_zip = seven_zip;
        latest.unrar = unrar;
        latest.lz4 = lz4;
        latest.quick_pack_format = quick_pack_format;
        (latest.shell_show_progress, latest.shell_show_console) = shell_display;
    })
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ToolPaths {
    pub seven_zip: String,
    pub unrar: String,
    pub lz4: String,
}

pub fn save_paths(config_path: &Path, mut paths: ToolPaths) -> Result<ToolPaths, String> {
    for (label, value) in [
        ("7-Zip", &mut paths.seven_zip),
        ("UnRAR", &mut paths.unrar),
        ("lz4", &mut paths.lz4),
    ] {
        *value = value.trim().trim_matches('"').trim().to_string();
        if !value.is_empty() && !Path::new(value).is_file() {
            return Err(format!("{label} 路径不存在或不是文件，请重新选择"));
        }
    }
    let saved = update_config_to(config_path, |cfg| {
        cfg.seven_zip = paths.seven_zip.clone();
        cfg.unrar = paths.unrar.clone();
        cfg.lz4 = paths.lz4.clone();
    });
    if !saved {
        return Err("工具路径保存失败，请检查配置目录的写入权限或磁盘空间".into());
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use unzip_core::config::{load_config_from, save_config_to, Config};
    #[test]
    fn bundled_paths_seed_defaults_and_keep_custom_choices() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let bundled = root.join("resources/bundled");
        std::fs::create_dir_all(&bundled).unwrap();
        for name in ["7z.exe", "7z.dll", "UnRAR.exe"] {
            std::fs::write(bundled.join(name), b"fixture").unwrap();
        }
        assert_eq!(find_bundled_tools_dir(root), Some(bundled.clone()));
        let config = root.join("config.json");
        assert!(initialize_bundled_tool_paths(&config, &bundled));
        let loaded = load_config_from(&config);
        assert_eq!(loaded.seven_zip, bundled.join("7z.exe").to_string_lossy());
        assert_eq!(loaded.unrar, bundled.join("UnRAR.exe").to_string_lossy());
        assert!(loaded.lz4.is_empty());
        let mut custom = Config::default();
        custom.seven_zip = "C:/custom/7z.exe".into();
        custom.passwords.push("keep".into());
        assert!(save_config_to(&custom, &config));
        assert!(initialize_bundled_tool_paths(&config, &bundled));
        assert_eq!(load_config_from(&config).seven_zip, custom.seven_zip);
        assert_eq!(load_config_from(&config).unrar, bundled.join("UnRAR.exe").to_string_lossy());
        assert_eq!(load_config_from(&config).passwords, ["keep"]);
        std::fs::write(&config, "{invalid json").unwrap();
        assert!(initialize_bundled_tool_paths(&config, &bundled));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "{invalid json");
    }
    #[test]
    fn saves_tools_without_overwriting_unrelated_settings_and_can_clear() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.json");
        let exe = temp.path().join("工具 空格.exe");
        std::fs::write(&exe, b"fixture, not executed").unwrap();
        assert!(save_config_to(
            &Config {
                passwords: vec!["keep".into()],
                output_dir: "keep output".into(),
                ..Default::default()
            },
            &config
        ));
        let paths = ToolPaths {
            seven_zip: format!(" \"{}\" ", exe.display()),
            unrar: String::new(),
            lz4: String::new(),
        };
        let saved = save_paths(&config, paths).unwrap();
        let loaded = load_config_from(&config);
        assert_eq!(loaded.seven_zip, exe.to_string_lossy());
        assert_eq!(loaded.passwords, vec!["keep"]);
        assert_eq!(loaded.output_dir, "keep output");
        save_paths(
            &config,
            ToolPaths {
                seven_zip: String::new(),
                ..saved
            },
        )
        .unwrap();
        assert!(load_config_from(&config).seven_zip.is_empty());
    }
    #[test]
    fn invalid_path_does_not_change_saved_config_and_write_failure_is_reported() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.json");
        assert!(save_config_to(&Config::default(), &config));
        let before = std::fs::read(&config).unwrap();
        let invalid = ToolPaths {
            seven_zip: temp.path().to_string_lossy().into_owned(),
            unrar: String::new(),
            lz4: String::new(),
        };
        assert!(save_paths(&config, invalid).is_err());
        assert_eq!(std::fs::read(&config).unwrap(), before);
        let empty = ToolPaths {
            seven_zip: String::new(),
            unrar: String::new(),
            lz4: String::new(),
        };
        assert!(save_paths(&temp.path().join("missing/config.json"), empty).is_err());
    }
    #[test]
    fn stale_unpack_page_cannot_overwrite_new_shared_tool_paths() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.json");
        let exe = temp.path().join("new-tools.exe");
        std::fs::write(&exe, b"not executed").unwrap();
        let mut stale = Config::default();
        assert!(save_config_to(&stale, &config));
        let new_path = exe.to_string_lossy().into_owned();
        save_paths(
            &config,
            ToolPaths {
                seven_zip: new_path.clone(),
                unrar: new_path.clone(),
                lz4: String::new(),
            },
        )
        .unwrap();
        stale.output_dir = "new unpack output".into();
        stale.passwords = vec!["new password".into()];
        assert!(save_unpack_config(&config, stale));
        let loaded = load_config_from(&config);
        assert_eq!(loaded.seven_zip, new_path);
        assert_eq!(loaded.unrar, new_path);
        assert_eq!(loaded.output_dir, "new unpack output");
        assert_eq!(loaded.passwords, vec!["new password"]);
    }
}
