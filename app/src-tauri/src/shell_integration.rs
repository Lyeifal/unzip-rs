//! Explorer verbs enqueue paths; execution always requires confirmation in the UI.
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ShellRequest {
    pub action: String,
    pub paths: Vec<String>,
}

#[derive(Default)]
pub struct ShellState(pub Mutex<Vec<ShellRequest>>);

pub fn parse_request(args: &[String], cwd: &Path) -> Option<ShellRequest> {
    let action = match args.get(1).map(String::as_str) {
        Some("--shell-pack") => "pack",
        Some("--shell-unpack") => "unpack",
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
    const VERBS: [(&str, &str, &str); 2] = [
        ("UnzipRs.Pack", "添加到压缩列表", "pack"),
        ("UnzipRs.Unpack", "添加到解压列表", "unpack"),
    ];
    pub fn enabled(exe: &Path) -> Result<bool, String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for root in ROOTS {
            for (verb, _, action) in VERBS {
                let key = match hkcu.open_subkey(format!(r"{}\{}\command", root, verb)) {
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
        for root in ROOTS {
            for (verb, label, action) in VERBS {
                let path = format!(r"{}\{}", root, verb);
                if enabled {
                    let (key, _) = hkcu
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
                } else {
                    match hkcu.delete_subkey_all(&path) {
                        Ok(()) => (),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                        Err(e) => return Err(format!("移除右键菜单失败：{e}")),
                    }
                }
            }
        }
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
        Ok(enabled)
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
    }
    #[test]
    fn command_quotes_paths_and_never_invokes_a_shell() {
        assert_eq!(
            command_for(Path::new(r"C:\工具 目录\app.exe"), "pack"),
            "\"C:\\工具 目录\\app.exe\" --shell-pack \"%1\""
        );
    }
}
