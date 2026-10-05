//! 自动解压工具 — Tauri v2 外壳。
//! 启动时解析 app_data_dir，设 UNZIP_CONFIG_PATH / UNZIP_BUNDLED_DIR。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use unzip_core::config::{load_config_from, save_config_to, Config};
use unzip_core::pack::{run_pack, PackOptions, PackSummary};
use unzip_core::types::{LogLevel, RunCallback, Summary};

struct AppState {
    config_path: PathBuf,
    /// true = 有解压任务在跑（防重入）。
    busy: Arc<AtomicBool>,
    /// 取消标志：停止按钮置位，run 循环的 should_cancel 读取。
    cancel: Arc<AtomicBool>,
}

/// 线程结束时自动复位 busy，防 panic 后卡死「运行中」状态。
struct BusyGuard(Arc<AtomicBool>);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// 把 core 的日志/进度/取消桥接成 Tauri 事件。
/// 事件名 = "{prefix}-log" / "{prefix}-progress"：解压 "uz"，打包 "pack"，两页互不串扰。
struct TauriCallback {
    app: AppHandle,
    cancel: Arc<AtomicBool>,
    event_prefix: &'static str,
}

impl RunCallback for TauriCallback {
    fn on_log(&self, msg: &str, level: LogLevel) {
        let event = format!("{}-log", self.event_prefix);
        let _ = self
            .app
            .emit(&event, json!({ "msg": msg, "level": level.as_str() }));
    }

    fn on_progress(&self, done: usize, total: usize, name: &str) {
        let event = format!("{}-progress", self.event_prefix);
        let _ = self
            .app
            .emit(&event, json!({ "done": done, "total": total, "name": name }));
    }

    fn should_cancel(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// Summary → JSON：ok/failed/skipped 为 [名称, 备注/原因] 二元组数组。
fn summary_payload(s: &Summary) -> Value {
    let pairs = |v: &[(String, String)]| -> Vec<Value> {
        v.iter().map(|(a, b)| json!([a, b])).collect()
    };
    json!({
        "ok": pairs(&s.ok),
        "failed": pairs(&s.failed),
        "skipped": pairs(&s.skipped),
        "warns": s.warns,
    })
}

/// PackSummary → JSON：ok 为含密码/产物/大小的对象数组，failed 为二元组数组。
fn pack_summary_payload(s: &PackSummary) -> Value {
    json!({
        "ok": s.ok.iter().map(|r| json!({
            "name": r.name,
            "password": r.password,
            "size_bytes": r.size_bytes,
            "outputs": r.outputs.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "failed": s.failed.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
        "warns": s.warns,
    })
}

#[tauri::command]
fn load_config(state: State<'_, AppState>) -> Config {
    load_config_from(&state.config_path)
}

#[tauri::command]
fn save_config(state: State<'_, AppState>, cfg: Config) -> bool {
    save_config_to(&cfg, &state.config_path)
}

#[tauri::command]
async fn pick_directory(current: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dlg = rfd::FileDialog::new();
        let cur = current.trim();
        if !cur.is_empty() {
            dlg = dlg.set_directory(PathBuf::from(cur));
        } else if let Some(home) = std::env::var_os("USERPROFILE") {
            dlg = dlg.set_directory(PathBuf::from(home));
        }
        dlg.pick_folder().map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
}

#[tauri::command]
async fn pick_file(current: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dlg = rfd::FileDialog::new().add_filter("程序", &["exe"]);
        let cur = current.trim();
        if !cur.is_empty() {
            let p = PathBuf::from(cur);
            dlg = if p.is_dir() {
                dlg.set_directory(p)
            } else {
                dlg.set_file_name(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
            };
        }
        dlg.pick_file().map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
}

#[tauri::command]
async fn pick_files(current: String) -> Vec<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dlg = rfd::FileDialog::new();
        let cur = current.trim();
        if !cur.is_empty() {
            let p = PathBuf::from(cur);
            dlg = if p.is_dir() {
                dlg.set_directory(p)
            } else {
                dlg.set_directory(p.parent().map(|d| d.to_path_buf()).unwrap_or(p))
            };
        }
        dlg.pick_files()
            .map(|ps| {
                ps.iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
async fn pick_directories_multi(current: String) -> Vec<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dlg = rfd::FileDialog::new();
        let cur = current.trim();
        if !cur.is_empty() {
            dlg = dlg.set_directory(PathBuf::from(cur));
        } else if let Some(home) = std::env::var_os("USERPROFILE") {
            dlg = dlg.set_directory(PathBuf::from(home));
        }
        dlg.pick_folders()
            .map(|ps| {
                ps.iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
fn start_run(
    app: AppHandle,
    state: State<'_, AppState>,
    source_dir: String,
    output_dir: String,
    failed_dir: String,
) -> Result<(), String> {
    let src = PathBuf::from(source_dir.trim());
    if source_dir.trim().is_empty() || !src.is_dir() {
        return Err("请先选择有效的压缩包所在目录".to_string());
    }
    if output_dir.trim().is_empty() {
        return Err("请先选择解压文件所在目录".to_string());
    }
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("已有解压任务正在进行中".to_string());
    }

    let cfg = load_config_from(&state.config_path);
    let out = PathBuf::from(output_dir.trim());
    let failed = if failed_dir.trim().is_empty() {
        out.join("解压失败")
    } else {
        PathBuf::from(failed_dir.trim())
    };

    let busy = state.busy.clone();
    let cancel = state.cancel.clone();
    cancel.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let _guard = BusyGuard(busy);
        let recursive = cfg.recursive;
        let passwords = cfg.passwords.clone();
        let cb = TauriCallback {
            app: app.clone(),
            cancel,
            event_prefix: "uz",
        };
        let summary = unzip_core::run(
            &[src],
            &out,
            Some(&failed),
            cfg,
            passwords,
            &cb,
            false,
            recursive,
        );
        let _ = app.emit("uz-done", summary_payload(&summary));
    });
    Ok(())
}

#[tauri::command]
fn cancel_run(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn start_pack(
    app: AppHandle,
    state: State<'_, AppState>,
    options: PackOptions,
    files: Vec<String>,
) -> Result<(), String> {
    if files.is_empty() {
        return Err("待打包列表为空".to_string());
    }
    let out = options.output_dir.trim();
    if out.is_empty() || !Path::new(out).is_dir() {
        return Err("请先选择有效的打包输出目录".to_string());
    }
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("已有任务正在进行中".to_string());
    }

    let cfg = load_config_from(&state.config_path);
    let mut sources: Vec<PathBuf> = Vec::new();
    for f in &files {
        let p = PathBuf::from(f);
        if p.exists() {
            sources.push(p);
        }
    }
    if sources.is_empty() {
        state.busy.store(false, Ordering::SeqCst);
        return Err("待打包的文件或目录均不存在".to_string());
    }

    let busy = state.busy.clone();
    let cancel = state.cancel.clone();
    cancel.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let _guard = BusyGuard(busy);
        let cb = TauriCallback {
            app: app.clone(),
            cancel,
            event_prefix: "pack",
        };
        let summary = run_pack(&sources, &options, &cfg, &cb);
        let _ = app.emit("pack-done", pack_summary_payload(&summary));
    });
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let app_data = app.path().app_data_dir().expect("无法解析 app_data_dir");
            std::fs::create_dir_all(&app_data)?;
            let config_path = app_data.join("config.json");
            std::env::set_var("UNZIP_CONFIG_PATH", &config_path);
            if let Ok(res_dir) = app.path().resource_dir() {
                std::env::set_var("UNZIP_BUNDLED_DIR", res_dir);
            }
            app.manage(AppState {
                config_path,
                busy: Arc::new(AtomicBool::new(false)),
                cancel: Arc::new(AtomicBool::new(false)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            pick_directory,
            pick_file,
            pick_files,
            pick_directories_multi,
            start_run,
            cancel_run,
            start_pack,
        ])
        .run(tauri::generate_context!())
        .expect("运行自动解压工具失败");
}
