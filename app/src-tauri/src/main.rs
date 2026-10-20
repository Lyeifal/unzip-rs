//! unzip-rs — Tauri v2 外壳。
//! 启动时解析 app_data_dir，设 UNZIP_CONFIG_PATH / UNZIP_BUNDLED_DIR。

// Explorer can invoke debug builds too: allocating a console here flashes before main().
#![cfg_attr(windows, windows_subsystem = "windows")]

mod shell_integration;
mod tool_settings;
mod task_console;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use unzip_core::config::{load_config_from, load_config_from_checked, Config};
use unzip_core::pack::{run_pack, PackOptions, PackSummary};
use unzip_core::types::{LogLevel, RetryableFailure, RunCallback, Summary};
use unzip_core::pipeline::{run_with_layout, retry_with_layout, OutputLayout};

struct PendingRetry {
    failure: RetryableFailure,
    output_root: PathBuf,
    layout: OutputLayout,
}

struct AppState {
    config_path: PathBuf,
    /// true = 有解压任务在跑（防重入）。
    busy: Arc<AtomicBool>,
    /// 取消标志：停止按钮置位，run 循环的 should_cancel 读取。
    cancel: Arc<AtomicBool>,
    retryable: Arc<Mutex<Vec<Option<PendingRetry>>>>,
    update_pending: AtomicBool,
}

/// 线程结束时自动复位 busy，防 panic 后卡死「运行中」状态。
struct BusyGuard(Arc<AtomicBool>);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn worker_failure() -> Value {
    json!({
        "ok": [], "failed": [["后台任务", "处理线程异常退出，请检查输出目录、失败目录和原文件后重试"]],
        "skipped": [], "warns": [], "retryable": []
    })
}

fn complete_archive_work(guard: BusyGuard, failure: Value, work: impl FnOnce() -> Value) -> Value {
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or(failure);
    drop(guard);
    payload
}

/// Always notify the UI, including a worker panic. Release the slot before sending completion.
fn spawn_archive_task(
    app: AppHandle,
    busy: Arc<AtomicBool>,
    event: &'static str,
    failure: Value,
    work: impl FnOnce() -> Value + Send + 'static,
) -> Result<(), String> {
    let guard = BusyGuard(busy);
    std::thread::Builder::new().name(event.into()).spawn(move || {
        let payload = complete_archive_work(guard, failure, work);
        let _ = app.emit(event, payload);
    }).map(|_| ()).map_err(|error| format!("无法启动处理线程：{error}"))
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
        task_console::log(msg);
        let event = format!("{}-log", self.event_prefix);
        let _ = self
            .app
            .emit(&event, json!({ "msg": msg, "level": level.as_str() }));
    }

    fn on_progress(&self, done: usize, total: usize, name: &str) {
        task_console::log(&format!("[进度] {done}/{total}：{name}"));
        let event = format!("{}-progress", self.event_prefix);
        let _ = self.app.emit(
            &event,
            json!({ "done": done, "total": total, "name": name }),
        );
    }

    fn should_cancel(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// Summary → JSON：ok/failed/skipped 为 [名称, 备注/原因] 二元组数组。
fn summary_payload(s: &Summary) -> Value {
    let pairs =
        |v: &[(String, String)]| -> Vec<Value> { v.iter().map(|(a, b)| json!([a, b])).collect() };
    json!({
        "ok": pairs(&s.ok),
        "failed": pairs(&s.failed),
        "skipped": pairs(&s.skipped),
        "warns": s.warns,
        "retryable": s.retryable.iter().enumerate().map(|(slot, r)| json!({
            "slot": slot, "failed_index": r.failed_index, "name": r.name,
        })).collect::<Vec<_>>(),
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
fn load_config(state: State<'_, AppState>) -> Result<Config, String> {
    load_config_from_checked(&state.config_path)
}

#[tauri::command]
fn save_config(state: State<'_, AppState>, cfg: Config) -> bool {
    tool_settings::save_unpack_config(&state.config_path, cfg)
}

/// Direct Explorer actions reuse a running instance without stealing focus.
/// An instance started for a direct action exits after all frontend/backend work drains.
#[tauri::command]
fn finish_shell_requests(app: AppHandle, state: State<'_, AppState>, reveal: bool) {
    if reveal { shell_integration::show_main(&app); return; }
    if load_config_from(&state.config_path).shell_show_console { return; }
    let shell = app.state::<shell_integration::ShellState>();
    let pending = shell.0.lock().unwrap_or_else(|p| p.into_inner());
    if shell.1.load(Ordering::SeqCst) && !state.busy.load(Ordering::SeqCst) && pending.is_empty() {
        app.exit(0);
    }
}

#[tauri::command]
fn save_tool_paths(
    state: State<'_, AppState>,
    paths: tool_settings::ToolPaths,
) -> Result<tool_settings::ToolPaths, String> {
    tool_settings::save_paths(&state.config_path, paths)
}

#[tauri::command]
fn begin_update_install(state: State<'_, AppState>) -> Result<(), String> {
    acquire_update_install(&state.busy, &state.update_pending)
}

fn acquire_update_install(busy: &AtomicBool, update_pending: &AtomicBool) -> Result<(), String> {
    if busy.swap(true, Ordering::SeqCst) {
        return Err("请先等待压缩或解压任务完成，再安装更新".into());
    }
    update_pending.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
fn end_update_install(state: State<'_, AppState>) {
    release_update_install(&state.busy, &state.update_pending);
}

fn release_update_install(busy: &AtomicBool, update_pending: &AtomicBool) {
    if update_pending.swap(false, Ordering::SeqCst) {
        busy.store(false, Ordering::SeqCst);
    }
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
                dlg.set_file_name(
                    p.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                )
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
    files: Option<Vec<String>>,
    source_mode: Option<String>,
) -> Result<(), String> {
    let sources = collect_unpack_sources(&source_dir, files, source_mode.as_deref())?;
    if output_dir.trim().is_empty() {
        return Err("请先选择解压输出目录".to_string());
    }
    let cfg = load_config_from_checked(&state.config_path)?;
    let accepted = launch_unpack(app, &state, sources, PathBuf::from(output_dir.trim()), failed_dir, cfg, OutputLayout::Grouped)?;
    if !accepted { return Err("已有压缩、解压或更新任务正在进行中".into()); }
    Ok(())
}

#[tauri::command]
fn start_shell_run(
    app: AppHandle, state: State<'_, AppState>, action: String, paths: Vec<String>,
) -> Result<bool, String> {
    let layout = match action.as_str() {
        "extract-here" => OutputLayout::Here,
        "extract-folder" => OutputLayout::NamedFolder,
        _ => return Err("不支持的右键解压操作".into()),
    };
    let sources = collect_unpack_sources("", Some(paths), Some("selected"))?;
    let out = if sources[0].is_dir() { sources[0].clone() }
        else { sources[0].parent().ok_or("无法确定压缩包所在目录")?.to_path_buf() };
    let cfg = load_config_from_checked(&state.config_path)?;
    let failed_dir = cfg.failed_dir.clone();
    launch_unpack(app, &state, sources, out, failed_dir, cfg, layout)
}

#[allow(clippy::too_many_arguments)]
fn launch_unpack(
    app: AppHandle, state: &AppState, sources: Vec<PathBuf>, out: PathBuf,
    failed_dir: String, cfg: Config, layout: OutputLayout,
) -> Result<bool, String> {
    if state.busy.swap(true, Ordering::SeqCst) {
        return Ok(false);
    }
    let failed = if failed_dir.trim().is_empty() {
        out.join("failed")
    } else {
        PathBuf::from(failed_dir.trim())
    };

    let busy = state.busy.clone();
    let cancel = state.cancel.clone();
    let retryable = state.retryable.clone();
    if let Ok(mut pending) = retryable.lock() {
        pending.clear();
    } else {
        state.busy.store(false, Ordering::SeqCst);
        return Err("重试状态不可用".into());
    }
    cancel.store(false, Ordering::SeqCst);
    spawn_archive_task(app.clone(), busy, "uz-done", worker_failure(), move || {
        let recursive = cfg.recursive;
        let passwords = cfg.passwords.clone();
        let cb = TauriCallback {
            app: app.clone(),
            cancel,
            event_prefix: "uz",
        };
        let summary = run_with_layout(
            &sources,
            &out,
            cfg.failed_dir_enabled.then_some(failed.as_path()),
            cfg,
            passwords,
            &cb,
            false,
            recursive,
            layout,
        );
        task_console::log(&format!("[完成] 解压成功 {}，失败 {}，跳过 {}，警告 {}", summary.ok.len(), summary.failed.len(), summary.skipped.len(), summary.warns.len()));
        if let Ok(mut pending) = retryable.lock() {
            *pending = summary.retryable.iter().map(|r| Some(PendingRetry {
                failure: RetryableFailure { name: r.name.clone(), paths: r.paths.clone(), failed_index: r.failed_index, source_parent: r.source_parent.clone() },
                output_root: if layout == OutputLayout::Grouped { out.clone() } else { r.source_parent.clone() }, layout,
            })).collect();
        }
        summary_payload(&summary)
    })?;
    Ok(true)
}

#[tauri::command]
fn retry_unpack_password(
    app: AppHandle,
    state: State<'_, AppState>,
    slot: usize,
    password: String,
    output_dir: String,
) -> Result<(), String> {
    if password.is_empty() { return Err("请输入新密码".into()); }
    let _ = output_dir; // Retry uses the original job's output and layout, including Explorer jobs.
    let mut cfg = load_config_from_checked(&state.config_path)?;
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("已有解压任务正在进行中".into());
    }
    let target = match state.retryable.lock() {
        Ok(pending) => pending.get(slot).and_then(|r| r.as_ref()).map(|r| (r.failure.paths.clone(), r.failure.failed_index, r.output_root.clone(), r.layout)),
        Err(_) => None,
    };
    let Some((paths, failed_index, out, layout)) = target else {
        state.busy.store(false, Ordering::SeqCst);
        return Err("该任务已不可重试，请重新扫描".into());
    };
    if paths.is_empty() || paths.iter().any(|p| !p.is_file()) {
        state.busy.store(false, Ordering::SeqCst);
        return Err("失败压缩包已移动或删除，请重新扫描".into());
    }
    cfg.retry_password_only = true;
    cfg.hash_match = false;
    cfg.passwords.clear();
    cfg.password_rules.clear();
    cfg.password_strategy = "list_order".into();
    let busy = state.busy.clone();
    let cancel = state.cancel.clone();
    let pending = state.retryable.clone();
    cancel.store(false, Ordering::SeqCst);
    spawn_archive_task(app.clone(), busy, "uz-retry-done",
        json!({"slot": slot, "failedIndex": failed_index, "result": worker_failure()}), move || {
        let cb = TauriCallback { app: app.clone(), cancel, event_prefix: "uz" };
        let mut summary = retry_with_layout(&paths, &out, cfg, vec![password.clone()], &cb, layout);
        if let Ok(mut entries) = pending.lock() {
            if let Some(entry) = entries.get_mut(slot) {
                if !summary.ok.is_empty() || !summary.failed.is_empty() {
                    *entry = summary.retryable.first().map(|r| PendingRetry {
                        failure: RetryableFailure { name: r.name.clone(), paths: r.paths.clone(), failed_index, source_parent: out.clone() },
                        output_root: out.clone(), layout,
                    });
                }
            }
        }
        if !summary.ok.is_empty() {
            let mut saved = load_config_from(&app.state::<AppState>().config_path);
            if !saved.passwords.contains(&password) {
                saved.passwords.push(password);
                if !tool_settings::save_unpack_config(&app.state::<AppState>().config_path, saved) {
                    summary.warns.push("重试成功，但新密码未能保存到密码库".into());
                }
            }
        }
        json!({ "slot": slot, "failedIndex": failed_index, "result": summary_payload(&summary) })
    })?;
    Ok(())
}

fn collect_unpack_sources(
    source_dir: &str,
    files: Option<Vec<String>>,
    source_mode: Option<&str>,
) -> Result<Vec<PathBuf>, String> {
    // Missing mode preserves the old API: explicit files override directory scan.
    let mode = source_mode.unwrap_or(if files.is_some() { "selected" } else { "directory" });
    if !["selected", "directory", "both"].contains(&mode) {
        return Err("解压来源方式无效".into());
    }
    let mut sources = Vec::new();
    if mode == "directory" || mode == "both" {
        let src = PathBuf::from(source_dir.trim());
        if source_dir.trim().is_empty() || !src.is_dir() {
            return Err("请先选择有效的压缩包所在目录".into());
        }
        sources.push(src);
    }
    if mode == "selected" || mode == "both" {
        let files = files.ok_or("选中文件列表为空")?;
        if files.is_empty() {
            return Err("选中文件列表为空".into());
        }
        let paths: Vec<_> = files.into_iter().map(PathBuf::from).collect();
        if paths.iter().any(|p| !p.is_file() && !p.is_dir()) {
            return Err("选中的部分文件或目录已不存在，请更新列表".into());
        }
        sources.extend(paths);
    }
    Ok(sources)
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
    let sources = collect_unpack_sources("", Some(files), Some("selected"))?;
    let cfg = load_config_from_checked(&state.config_path)?;
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("已有任务正在进行中".to_string());
    }

    let busy = state.busy.clone();
    let cancel = state.cancel.clone();
    cancel.store(false, Ordering::SeqCst);
    spawn_archive_task(app.clone(), busy, "pack-done", worker_failure(), move || {
        let cb = TauriCallback {
            app: app.clone(),
            cancel,
            event_prefix: "pack",
        };
        let summary = run_pack(&sources, &options, &cfg, &cb);
        pack_summary_payload(&summary)
    })?;
    Ok(())
}

#[tauri::command]
async fn preview_pack_names(options: PackOptions, count: usize) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        unzip_core::pack::preview_pack_names(&options, count)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_quick_pack_format(state: State<'_, AppState>) -> String {
    match load_config_from(&state.config_path).quick_pack_format.as_str() { "zip" => "zip", _ => "7z" }.into()
}

fn save_quick_pack_format(path: &Path, format: &str) -> Result<String, String> {
    if !matches!(format, "7z" | "zip") { return Err("快捷压缩仅支持 7z / zip".into()); }
    if !unzip_core::config::update_config_to(path, |cfg| cfg.quick_pack_format = format.into()) {
        return Err("快捷压缩设置保存失败，请检查配置目录的写入权限或磁盘空间".into());
    }
    Ok(format.into())
}

#[tauri::command]
fn set_quick_pack_format(state: State<'_, AppState>, format: String) -> Result<String, String> {
    save_quick_pack_format(&state.config_path, &format)
}

#[tauri::command]
fn get_shell_display(state: State<'_, AppState>) -> Value {
    let cfg = load_config_from(&state.config_path);
    json!({"showProgress": cfg.shell_show_progress, "showConsole": cfg.shell_show_console})
}

fn save_shell_display(path: &Path, show_progress: bool, show_console: bool) -> Result<(), String> {
    if !unzip_core::config::update_config_to(path, |cfg| {
        cfg.shell_show_progress = show_progress; cfg.shell_show_console = show_console;
    }) { return Err("右键任务显示设置保存失败，请检查写入权限或磁盘空间".into()); }
    Ok(())
}

#[tauri::command]
fn set_shell_display(state: State<'_, AppState>, show_progress: bool, show_console: bool) -> Result<(), String> {
    save_shell_display(&state.config_path, show_progress, show_console)
}

#[tauri::command]
fn start_shell_pack(app: AppHandle, state: State<'_, AppState>, paths: Vec<String>) -> Result<bool, String> {
    let sources = collect_unpack_sources("", Some(paths), Some("selected"))?;
    let cfg = load_config_from_checked(&state.config_path)?;
    let format = if cfg.quick_pack_format == "zip" { "zip" } else { "7z" };
    if state.busy.swap(true, Ordering::SeqCst) { return Ok(false); }
    let busy = state.busy.clone();
    let cancel = state.cancel.clone();
    cancel.store(false, Ordering::SeqCst);
    spawn_archive_task(app.clone(), busy, "pack-done", worker_failure(), move || {
        let cb = TauriCallback { app: app.clone(), cancel, event_prefix: "pack" };
        let summary = unzip_core::pack::run_quick_pack(&sources, format, &cfg, &cb);
        task_console::log(&format!("[完成] 压缩成功 {}，失败 {}，警告 {}", summary.ok.len(), summary.failed.len(), summary.warns.len()));
        pack_summary_payload(&summary)
    })?;
    Ok(true)
}

#[tauri::command]
async fn load_archive_library() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        Ok(json!({"path": unzip_core::archive_catalog::database_path(), "catalog": unzip_core::archive_catalog::load()?}))
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn query_archive_library(
    page: u32,
    page_size: u32,
    search: String,
    sort: String,
    filters: HashMap<String, String>,
    option_fields: Vec<String>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        Ok(json!({
            "path": unzip_core::archive_catalog::database_path(),
            "result": unzip_core::archive_catalog::query_page(
                page, page_size, &search, &sort, &filters, &option_fields
            )?
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn update_archive_metadata(record_id: String, metadata: Value) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        unzip_core::archive_catalog::update_metadata(&record_id, metadata)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn import_archive_library(path: Option<String>) -> Result<Option<usize>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let file = match path {
            Some(path) => {
                let file = PathBuf::from(path);
                // Automatic migration only imports a known old output-directory mapping.
                if !file.is_file() {
                    return Ok(None);
                }
                file
            }
            None => match rfd::FileDialog::new()
                .add_filter("压缩记录 JSON", &["json"])
                .pick_file()
            {
                Some(file) => file,
                None => return Ok(None),
            },
        };
        unzip_core::archive_catalog::import_legacy(&file).map(Some)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn export_archive_library() -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let Some(file) = rfd::FileDialog::new()
            .add_filter("压缩记录 JSON", &["json"])
            .set_file_name("pack_mapping_backup.json")
            .save_file()
        else {
            return Ok(None);
        };
        unzip_core::archive_catalog::export(&file)?;
        Ok(Some(file.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn main() {
    let startup_args = std::env::args().collect::<Vec<_>>();
    let background = shell_integration::is_direct_request(&startup_args);
    tauri::Builder::default()
        .manage(shell_integration::ShellState(Mutex::new(Vec::new()), AtomicBool::new(background)))
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            shell_integration::enqueue(app, &args, Path::new(&cwd));
            if !shell_integration::is_direct_request(&args) {
                shell_integration::show_main(app);
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(move |app| {
            let app_data = app.path().app_data_dir()?;
            let local_data = app.path().app_local_data_dir()?;
            let legacy = unzip_core::storage::LEGACY_APP_DIR_NAME;
            // Window creation is delayed until both migrations complete, before WebView opens its files.
            unzip_core::storage::migrate_directory(&app.path().data_dir()?.join(legacy), &app_data, true)
                .map_err(std::io::Error::other)?;
            unzip_core::storage::migrate_directory(&app.path().local_data_dir()?.join(legacy), &local_data, false)
                .map_err(std::io::Error::other)?;
            let config_path = app_data.join("config.json");
            std::env::set_var("UNZIP_CONFIG_PATH", &config_path);
            std::env::set_var("UNZIP_CATALOG_PATH", app_data.join("pack_mapping.json"));
            if let Ok(res_dir) = app.path().resource_dir() {
                if let Some(bundled_dir) = tool_settings::find_bundled_tools_dir(&res_dir) {
                    std::env::set_var("UNZIP_BUNDLED_DIR", &bundled_dir);
                    if !tool_settings::initialize_bundled_tool_paths(&config_path, &bundled_dir) {
                        eprintln!("无法保存内置工具默认路径；运行时仍会尝试内置工具");
                    }
                }
            }
            app.manage(AppState {
                config_path,
                busy: Arc::new(AtomicBool::new(false)),
                cancel: Arc::new(AtomicBool::new(false)),
                retryable: Arc::new(Mutex::new(Vec::new())),
                update_pending: AtomicBool::new(false),
            });
            shell_integration::enqueue(
                app.handle(),
                &std::env::args().collect::<Vec<_>>(),
                &std::env::current_dir().unwrap_or_default(),
            );
            tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                .visible(!app.state::<shell_integration::ShellState>().1.load(Ordering::SeqCst)).build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            save_tool_paths,
            begin_update_install,
            end_update_install,
            pick_directory,
            pick_file,
            pick_files,
            pick_directories_multi,
            start_run,
            start_shell_run,
            retry_unpack_password,
            cancel_run,
            start_pack,
            start_shell_pack,
            get_quick_pack_format,
            set_quick_pack_format,
            get_shell_display,
            set_shell_display,
            preview_pack_names,
            load_archive_library,
            query_archive_library,
            update_archive_metadata,
            import_archive_library,
            export_archive_library,
            shell_integration::get_shell_integration,
            shell_integration::set_shell_integration,
            shell_integration::take_shell_requests,
            finish_shell_requests,
        ])
        .run(tauri::generate_context!())
        .expect("运行 unzip-rs 失败");
}

#[cfg(test)]
mod unpack_sources_tests {
    use super::collect_unpack_sources;

    #[test]
    fn worker_completion_and_panic_release_busy_before_returning_ui_payload() {
        use super::*;
        let busy = Arc::new(AtomicBool::new(true));
        let result = complete_archive_work(BusyGuard(busy.clone()), worker_failure(), || {
            assert!(busy.load(Ordering::SeqCst));
            json!({"ok": ["completed"]})
        });
        assert_eq!(result["ok"][0], "completed");
        assert!(!busy.load(Ordering::SeqCst));
        busy.store(true, Ordering::SeqCst);
        let result = complete_archive_work(BusyGuard(busy.clone()), worker_failure(), || panic!("test worker panic"));
        assert_eq!(result["failed"].as_array().unwrap().len(), 1);
        assert!(!busy.load(Ordering::SeqCst));
    }

    #[test]
    fn shell_display_defaults_persist_without_stale_unpack_overwrite() {
        use unzip_core::config::load_config_from;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        std::fs::write(&path, r#"{"passwords":["keep"],"quick_pack_format":"zip"}"#).unwrap();
        let stale = load_config_from(&path);
        assert!(stale.shell_show_progress);
        assert!(!stale.shell_show_console);
        super::save_shell_display(&path, false, true).unwrap();
        let current = load_config_from(&path);
        assert!(!current.shell_show_progress);
        assert!(current.shell_show_console);
        assert_eq!(current.passwords, ["keep"]);
        assert_eq!(current.quick_pack_format, "zip");
        assert!(super::tool_settings::save_unpack_config(&path, stale));
        let current = load_config_from(&path);
        assert!(!current.shell_show_progress);
        assert!(current.shell_show_console);
        assert!(super::save_shell_display(&temp.path().join("missing/config.json"), true, false).is_err());
    }

    #[test]
    fn quick_pack_format_defaults_validates_and_survives_stale_unpack_save() {
        use unzip_core::config::{load_config_from, save_config_to, Config};
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        assert_eq!(load_config_from(&path).quick_pack_format, "7z");
        std::fs::write(&path, r#"{"passwords":["keep"],"output_dir":"unchanged"}"#).unwrap();
        let stale = load_config_from(&path);
        assert_eq!(stale.quick_pack_format, "7z");
        assert_eq!(super::save_quick_pack_format(&path, "zip").unwrap(), "zip");
        let loaded = load_config_from(&path);
        assert_eq!(loaded.quick_pack_format, "zip");
        assert_eq!(loaded.passwords, ["keep"]);
        assert_eq!(loaded.output_dir, "unchanged");
        assert!(super::tool_settings::save_unpack_config(&path, stale));
        assert_eq!(load_config_from(&path).quick_pack_format, "zip");
        let before = std::fs::read(&path).unwrap();
        assert!(super::save_quick_pack_format(&path, "rar").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(super::save_quick_pack_format(&temp.path().join("missing/config.json"), "7z").is_err());
        assert!(save_config_to(&Config::default(), &path));
        assert_eq!(load_config_from(&path).quick_pack_format, "7z");
    }

    #[test]
    fn update_install_never_unlocks_an_active_archive_task() {
        use super::{acquire_update_install, release_update_install, AtomicBool, Ordering};
        let busy = AtomicBool::new(true);
        let pending = AtomicBool::new(false);
        assert!(acquire_update_install(&busy, &pending).is_err());
        release_update_install(&busy, &pending);
        assert!(busy.load(Ordering::SeqCst));
        busy.store(false, Ordering::SeqCst);
        assert!(acquire_update_install(&busy, &pending).is_ok());
        assert!(busy.swap(true, Ordering::SeqCst), "更新期间不得开始压缩或解压任务");
        release_update_install(&busy, &pending);
        assert!(!busy.load(Ordering::SeqCst));
    }

    #[test]
    fn selected_directory_and_both_modes_choose_the_requested_sources() {
        let temp = tempfile::tempdir().unwrap();
        let selected = temp.path().join("one.zip");
        std::fs::write(&selected, b"fixture").unwrap();
        let directory = temp.path().to_string_lossy().into_owned();
        let selected_text = selected.to_string_lossy().into_owned();

        let chosen = collect_unpack_sources(&directory, Some(vec![selected_text.clone()]), Some("selected")).unwrap();
        assert_eq!(chosen, vec![selected.clone()]);
        let directory_only = collect_unpack_sources(&directory, Some(vec![selected_text.clone()]), Some("directory")).unwrap();
        assert_eq!(directory_only, vec![temp.path().to_path_buf()]);
        let both = collect_unpack_sources(&directory, Some(vec![selected_text.clone()]), Some("both")).unwrap();
        assert_eq!(both, vec![temp.path().to_path_buf(), selected.clone()]);
        // Old callers without a mode continue to let selected files override the directory.
        assert_eq!(collect_unpack_sources(&directory, Some(vec![selected_text]), None).unwrap(), vec![selected]);
    }

    #[test]
    fn source_modes_require_only_the_inputs_they_use() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("one.zip");
        std::fs::write(&file, b"fixture").unwrap();
        let selected = Some(vec![file.to_string_lossy().into_owned()]);
        assert!(collect_unpack_sources("", selected.clone(), Some("selected")).is_ok());
        assert!(collect_unpack_sources("", selected.clone(), Some("both")).is_err());
        assert!(collect_unpack_sources(&temp.path().to_string_lossy(), None, Some("both")).is_err());
        assert!(collect_unpack_sources(&temp.path().to_string_lossy(), selected, Some("unknown")).is_err());
    }
}
