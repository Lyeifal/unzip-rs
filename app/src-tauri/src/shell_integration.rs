//! Explorer submenu: queue files or request extraction with the saved password rules.
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ShellRequest {
    pub action: String,
    pub paths: Vec<String>,
}

#[derive(Default)]
pub struct ShellState(pub Mutex<Vec<ShellRequest>>, pub AtomicBool);

pub fn is_direct_request(args: &[String]) -> bool {
    matches!(args.get(1).map(String::as_str), Some("--shell-extract-here" | "--shell-extract-folder" | "--shell-pack-same-name"))
}

pub fn show_main(app: &AppHandle) {
    app.state::<ShellState>().1.store(false, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn parse_request(args: &[String], cwd: &Path) -> Option<ShellRequest> {
    let action = match args.get(1).map(String::as_str) {
        Some("--shell-pack") => "pack",
        Some("--shell-pack-same-name") => "pack-same-name",
        Some("--shell-unpack") => "unpack",
        Some("--shell-extract-here") => "extract-here",
        Some("--shell-extract-folder") => "extract-folder",
        _ => return None,
    };
    let mut paths = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw in args.iter().skip(2).filter(|s| s.as_str() != "--") {
        let path = PathBuf::from(raw);
        let path = if path.is_absolute() {
            path
        } else {
            cwd.join(path)
        };
        if !path.is_file() && !path.is_dir() {
            continue;
        }
        let path = path
            .canonicalize()
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        // Windows canonicalize uses extended-length syntax; keep the normal shell spelling.
        #[cfg(windows)]
        let path = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{}", unc)
        } else {
            path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
        };
        let key = if cfg!(windows) {
            path.to_lowercase()
        } else {
            path.clone()
        };
        if seen.insert(key) {
            paths.push(path);
        }
    }
    (!paths.is_empty()).then(|| ShellRequest {
        action: action.into(),
        paths,
    })
}

pub fn enqueue(app: &AppHandle, args: &[String], cwd: &Path) {
    if let Some(request) = parse_request(args, cwd) {
        if is_direct_request(args) {
            let cfg = unzip_core::config::load_config();
            crate::task_console::set_visible(cfg.shell_show_console);
            crate::task_console::log(&format!("[已接收] {}：{} 项，正在准备任务…", request.action, request.paths.len()));
            if cfg.shell_show_progress || cfg.shell_show_console { show_main(app); }
        }
        let state = app.state::<ShellState>();
        // Preserve requests until the frontend explicitly drains them (including startup).
        state
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(request);
        let _ = app.emit("shell-request-pending", ());
    }
}

#[tauri::command]
pub fn take_shell_requests(state: State<'_, ShellState>) -> Vec<ShellRequest> {
    std::mem::take(&mut *state.0.lock().unwrap_or_else(|p| p.into_inner()))
}

fn command_for(exe: &Path, action: &str) -> String {
    format!("\"{}\" --shell-{} \"%1\"", exe.display(), action)
}

#[cfg(windows)]
mod registry {
    use super::*;
    use winreg::{enums::*, RegKey};
    const ROOTS: [&str; 2] = [
        r"Software\Classes\*\shell",
        r"Software\Classes\Directory\shell",
    ];
    const MENU: &str = "UnzipRs";
    const LEGACY: [&str; 2] = ["UnzipRs.Pack", "UnzipRs.Unpack"];
    const VERBS: [(&str, &str, &str); 5] = [
        ("01.ExtractHere", "解压到当前目录", "extract-here"),
        ("02.ExtractFolder", "解压到同名文件夹", "extract-folder"),
        ("03.Unpack", "添加到解压列表…", "unpack"),
        ("04.Pack", "添加到压缩列表…", "pack"),
        ("05.QuickPack", "添加到同名压缩包", "pack-same-name"),
    ];
    pub fn enabled(exe: &Path) -> Result<bool, String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        enabled_at(&hkcu, &ROOTS, exe)
    }
    fn enabled_at(hkcu: &RegKey, roots: &[&str], exe: &Path) -> Result<bool, String> {
        for root in roots {
            let menu = match hkcu.open_subkey(format!(r"{}\{}", root, MENU)) {
                Ok(key) => key,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(e) => return Err(e.to_string()),
            };
            if menu.get_value::<String, _>("MUIVerb").ok().as_deref() != Some("unzip-rs")
                || menu.get_value::<String, _>("SubCommands").ok().as_deref() != Some("")
            {
                return Ok(false);
            }
            for (verb, _, action) in VERBS {
                let key = match menu.open_subkey(format!(r"shell\{}\command", verb)) {
                    Ok(key) => key,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                    Err(e) => return Err(format!("读取右键菜单失败：{e}")),
                };
                let command: String = key
                    .get_value("")
                    .map_err(|e| format!("读取右键菜单失败：{e}"))?;
                if command != command_for(exe, action) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    pub fn set(enabled: bool, exe: &Path) -> Result<bool, String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        set_at(&hkcu, &ROOTS, enabled, exe)?;
        notify_shell();
        Ok(enabled)
    }
    fn remove_owned(hkcu: &RegKey, path: &str) -> Result<(), String> {
        match hkcu.delete_subkey_all(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("移除右键菜单失败：{e}")),
        }
    }
    fn set_at(hkcu: &RegKey, roots: &[&str], enabled: bool, exe: &Path) -> Result<(), String> {
        for root in roots {
            for legacy in LEGACY {
                remove_owned(hkcu, &format!(r"{}\{}", root, legacy))?;
            }
            let path = format!(r"{}\{}", root, MENU);
            if !enabled {
                remove_owned(hkcu, &path)?;
                continue;
            }
            let (menu, _) = hkcu.create_subkey(&path).map_err(|e| e.to_string())?;
            menu.set_value("MUIVerb", &"unzip-rs")
                .and_then(|_| menu.set_value("SubCommands", &""))
                .and_then(|_| menu.set_value("Icon", &format!("\"{}\",0", exe.display())))
                .and_then(|_| menu.set_value("MultiSelectModel", &"Player"))
                .map_err(|e| e.to_string())?;
            for (verb, label, action) in VERBS {
                let path = format!(r"shell\{}", verb);
                let (key, _) = menu
                    .create_subkey(&path)
                    .map_err(|e| format!("添加右键菜单失败：{e}"))?;
                key.set_value("", &label)
                    .and_then(|_| key.set_value("Icon", &format!("\"{}\",0", exe.display())))
                    .and_then(|_| key.set_value("MultiSelectModel", &"Player"))
                    .map_err(|e| format!("添加右键菜单失败：{e}"))?;
                let (command, _) = key.create_subkey("command").map_err(|e| e.to_string())?;
                command
                    .set_value("", &command_for(exe, action))
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    fn notify_shell() {
        // SHChangeNotify refreshes cached shell associations without restarting Explorer.
        #[link(name = "shell32")]
        extern "system" {
            fn SHChangeNotify(
                event_id: i32,
                flags: u32,
                item1: *const std::ffi::c_void,
                item2: *const std::ffi::c_void,
            );
        }
        unsafe {
            SHChangeNotify(0x08000000, 0, std::ptr::null(), std::ptr::null());
        }
    }
    #[test]
    fn submenu_migrates_legacy_and_only_removes_owned_keys() {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = format!(r"Software\unzip-rs-tests\shell-{}", std::process::id());
        let (scratch, _) = hkcu.create_subkey(&path).unwrap();
        let exe = Path::new(r"C:\应用 空格\unzip-rs.exe");
        scratch
            .create_subkey(r"files\UnzipRs.Pack\command")
            .unwrap();
        scratch.create_subkey(r"files\OtherApp").unwrap();
        set_at(&scratch, &["files", "dirs"], true, exe).unwrap();
        assert!(enabled_at(&scratch, &["files", "dirs"], exe).unwrap());
        assert!(!enabled_at(&scratch, &["files", "dirs"], Path::new("moved.exe")).unwrap());
        assert!(scratch.open_subkey(r"files\UnzipRs.Pack").is_err());
        for (verb, label, action) in VERBS {
            let key = scratch
                .open_subkey(format!(r"files\UnzipRs\shell\{}", verb))
                .unwrap();
            assert_eq!(key.get_value::<String, _>("").unwrap(), label);
            assert_eq!(
                key.get_value::<String, _>("MultiSelectModel").unwrap(),
                "Player"
            );
            assert_eq!(
                key.open_subkey("command")
                    .unwrap()
                    .get_value::<String, _>("")
                    .unwrap(),
                command_for(exe, action)
            );
        }
        set_at(&scratch, &["files", "dirs"], false, exe).unwrap();
        assert!(!enabled_at(&scratch, &["files", "dirs"], exe).unwrap());
        assert!(scratch.open_subkey(r"files\OtherApp").is_ok());
        hkcu.delete_subkey_all(path).unwrap();
    }
}

#[tauri::command]
pub fn get_shell_integration() -> Result<bool, String> {
    #[cfg(windows)]
    {
        registry::enabled(&std::env::current_exe().map_err(|e| e.to_string())?)
    }
    #[cfg(not(windows))]
    {
        Err("资源管理器右键菜单仅支持 Windows".into())
    }
}

#[tauri::command]
pub fn set_shell_integration(enabled: bool) -> Result<bool, String> {
    #[cfg(windows)]
    {
        registry::set(
            enabled,
            &std::env::current_exe().map_err(|e| e.to_string())?,
        )
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Err("资源管理器右键菜单仅支持 Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_direct_actions_start_in_background() {
        for action in ["extract-here", "extract-folder", "pack-same-name"] {
            assert!(is_direct_request(&["app.exe".into(), format!("--shell-{action}")]));
        }
        for args in [vec!["app.exe".into()], vec!["app.exe".into(), "--shell-pack".into()], vec!["app.exe".into(), "--shell-unpack".into()]] {
            assert!(!is_direct_request(&args));
        }
    }
    #[test]
    fn selected_paths_are_parsed_without_shell_interpolation() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("中文 空格 ' & %.zip");
        std::fs::write(&file, "test").unwrap();
        let args = vec![
            "app.exe".into(),
            "--shell-unpack".into(),
            file.to_string_lossy().into_owned(),
            file.to_string_lossy().into_owned(),
            "missing.zip".into(),
        ];
        let request = parse_request(&args, dir.path()).unwrap();
        assert_eq!(request.action, "unpack");
        assert_eq!(request.paths.len(), 1);
        assert!(request.paths[0].ends_with("中文 空格 ' & %.zip"));
        assert!(parse_request(
            &[
                "app.exe".into(),
                "--unknown".into(),
                file.to_string_lossy().into_owned()
            ],
            dir.path()
        )
        .is_none());
        assert!(parse_request(&["app.exe".into(), "--shell-pack".into()], dir.path()).is_none());
        for action in ["extract-here", "extract-folder", "pack-same-name"] {
            let request = parse_request(
                &[
                    "app.exe".into(),
                    format!("--shell-{action}"),
                    "--".into(),
                    file.file_name().unwrap().to_string_lossy().into_owned(),
                ],
                dir.path(),
            )
            .unwrap();
            assert_eq!(request.action, action);
            assert_eq!(request.paths.len(), 1);
        }
    }
    #[test]
    fn command_quotes_paths_and_never_invokes_a_shell() {
        assert_eq!(
            command_for(Path::new(r"C:\工具 目录\app.exe"), "pack"),
            "\"C:\\工具 目录\\app.exe\" --shell-pack \"%1\""
        );
    }
}
