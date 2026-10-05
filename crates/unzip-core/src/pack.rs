//! 打包压缩：命名递增、自动密码、阈值分卷、混淆 txt（网盘发布场景）。
//!
//! 实现要点：
//! - 输入先硬链接镜像到 temp 暂存目录（不占磁盘、不动源文件），混入「资源说明.txt」后
//!   cd 进暂存目录打包 "."，压缩包根 = 输入内容 + 混淆 txt，下载者解压第一眼可见；
//! - 内容总大小超过阈值 → `-v{size}g` 分卷（7z 自动产出 .7z.001/.002 或 .zip.001…）；
//! - 7z 格式 `-mhe=on` 连文件名一起加密，zip 格式 `-mem=AES256`（mhe 对 zip 无效）；
//! - 混淆 txt = 模板（{date} 占位）+ 8 位随机 hex：同一天同一批的每个包内容也唯一，
//!   压缩包 hash 必然不同，网盘无法按 hash 比对封杀。

use std::fs;
use std::path::{Path, PathBuf};

use rand::distributions::{Distribution, Uniform};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::config::Config;
use crate::extract::Extractor;
use crate::types::{LogLevel, RunCallback};

/// 混淆 txt 的默认模板（{date} 为打包当天日期）。
pub const DEFAULT_TXT_TEMPLATE: &str = "本资源于 {date} 打包整理，解压密码请查看发布页面。";
/// 混入暂存目录根的固定文件名（内容每包唯一，文件名无需变化）。
pub const MIX_TXT_NAME: &str = "资源说明.txt";

const GB: u64 = 1024 * 1024 * 1024;
const MB: u64 = 1024 * 1024;
/// 随机密码长度；字符集剔除 0/O、1/I/l 等易混淆字符。
const PW_LEN: usize = 16;

// ---------------------------------------------------------------------------
// 选项与结果
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PackOptions {
    pub format: String,          // "7z" | "zip"
    pub password_mode: String,   // random_per_pack | random_uniform | manual
    pub uniform_password: String, // manual 模式手填；random_uniform 模式留空
    pub volume_threshold_gb: f64, // 内容总大小超过才分卷
    pub volume_size_gb: f64,     // 每个分卷的大小
    pub name_start: String,      // 起始包名（如 G198）
    pub name_step: u32,          // 数字递增步长
    pub mix_txt: bool,           // 是否混入混淆 txt
    pub txt_template: String,    // 支持 {date} 占位符
    pub compression_level: u32,  // 0-9
    pub output_dir: String,
}

impl Default for PackOptions {
    fn default() -> Self {
        PackOptions {
            format: "7z".to_string(),
            password_mode: "random_per_pack".to_string(),
            uniform_password: String::new(),
            volume_threshold_gb: 3.0,
            volume_size_gb: 2.0,
            name_start: "G001".to_string(),
            name_step: 1,
            mix_txt: true,
            txt_template: DEFAULT_TXT_TEMPLATE.to_string(),
            compression_level: 5,
            output_dir: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PackResult {
    pub name: String,               // 最终包名（撞名时可能带 " (2)"）
    pub password: Option<String>,   // None = 无密码
    pub size_bytes: u64,            // 源内容总大小
    pub outputs: Vec<PathBuf>,      // 产物文件（单卷或分卷序列）
}

#[derive(Default)]
pub struct PackSummary {
    pub ok: Vec<PackResult>,
    pub failed: Vec<(String, String)>,
    pub warns: Vec<String>,
}

// ---------------------------------------------------------------------------
// 纯函数（可单测）
// ---------------------------------------------------------------------------

/// 包名末尾数字 +step 递增，保留原数字宽度（G198→G199；Z999→Z1000 溢出变宽；
/// 无数字则在末尾追加 step：Game→Game1）。在 ASCII 字节上找数字段，
/// 不按字符索引切片（中文包名安全）。
pub fn next_name(name: &str, step: u32) -> String {
    let bytes = name.as_bytes();
    let mut start = bytes.len();
    while start > 0 && bytes[start - 1].is_ascii_digit() {
        start -= 1;
    }
    if start == bytes.len() {
        return format!("{name}{step}");
    }
    let width = bytes.len() - start;
    let num: u64 = name[start..].parse().unwrap_or(0);
    let next = num + step as u64;
    let natural = next.to_string().len();
    format!(
        "{}{:0w$}",
        &name[..start],
        next,
        w = width.max(natural)
    )
}

/// 清洗包名中的 Windows 文件名非法字符（\ / : * ? " < > | 与控制字符 → _）。
pub fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// 16 位随机密码（rand OS 随机源；剔除易混淆字符，避免各解压器里手输出错）。
pub fn gen_password() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789";
    let range = Uniform::from(0..CHARSET.len());
    let mut rng = rand::thread_rng();
    (0..PW_LEN)
        .map(|_| CHARSET[range.sample(&mut rng)] as char)
        .collect()
}

/// n 位小写随机 hex（混淆 txt 的 nonce）。
fn random_hex(n: usize) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let range = Uniform::from(0..HEX.len());
    let mut rng = rand::thread_rng();
    (0..n).map(|_| HEX[range.sample(&mut rng)] as char).collect()
}

/// 文件=自身大小；目录=递归累加（读不到元数据的项跳过）。
pub fn content_size(path: &Path) -> u64 {
    if path.is_file() {
        return path.metadata().map(|m| m.len()).unwrap_or(0);
    }
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok().map(|m| m.len()))
        .sum()
}

/// 人类可读大小（3.2 GB / 512 MB / 300 B）。
pub fn fmt_size(n: u64) -> String {
    if n >= GB {
        format!("{:.1} GB", n as f64 / GB as f64)
    } else if n >= MB {
        format!("{:.1} MB", n as f64 / MB as f64)
    } else {
        format!("{n} B")
    }
}

/// 分卷参数：整 GB 用 g（2.0 → "-v2g"）；小数 GB 转 MB（2.5 → "-v2560m"，7z 不接受小数 g）；
/// 不足 1 MB 给 7z 下限 64 KB。
fn volume_arg(gb: f64) -> String {
    if gb >= 1.0 && (gb - gb.trunc()).abs() < f64::EPSILON {
        return format!("-v{}g", gb as u64);
    }
    let mb = (gb * 1024.0).round() as u64;
    if mb >= 1 {
        format!("-v{mb}m")
    } else {
        "-v64k".to_string()
    }
}

/// 拼 `7z a` 参数（不含 run_7z 自动附加的 -y -bd -sccUTF-8；输入为 "."，工作目录另指定）。
fn build_7z_args(archive: &Path, size: u64, opts: &PackOptions, password: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "a".to_string(),
        format!("-t{}", opts.format),
        format!("-mx{}", opts.compression_level.min(9)),
    ];
    if size > (opts.volume_threshold_gb * GB as f64) as u64 {
        args.push(volume_arg(opts.volume_size_gb));
    }
    if let Some(pw) = password {
        args.push(format!("-p{pw}"));
        if opts.format == "7z" {
            args.push("-mhe=on".to_string()); // 连文件名一起加密（仅 7z 支持）
        } else {
            args.push("-mem=AES256".to_string());
        }
    }
    args.push(archive.to_string_lossy().into_owned());
    args.push(".".to_string());
    args
}

/// 多输入同名去重：file.txt 已占 → file_2.txt…（ASCII 折叠比较，对齐卷名匹配惯例）。
fn dedupe_name(used: &mut Vec<String>, name: &str) -> String {
    let taken = |cand: &str| used.iter().any(|u| u.eq_ignore_ascii_case(cand));
    if !taken(name) {
        used.push(name.to_string());
        return name.to_string();
    }
    let (stem, suffix) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for i in 2..1000 {
        let cand = format!("{stem}_{i}{suffix}");
        if !taken(&cand) {
            used.push(cand.clone());
            return cand;
        }
    }
    name.to_string()
}

/// 输出撞名避让：{name}.{format} 已存在则 "{name} (2)"…（对齐 unique_dir 风格，绝不覆盖旧包）。
fn unique_pack_name(dir: &Path, name: &str, format: &str) -> String {
    if !dir.join(format!("{name}.{format}")).exists() {
        return name.to_string();
    }
    for i in 2..1000 {
        let cand = format!("{name} ({i})");
        if !dir.join(format!("{cand}.{format}")).exists() {
            return cand;
        }
    }
    name.to_string()
}

// ---------------------------------------------------------------------------
// 打包执行
// ---------------------------------------------------------------------------

/// 暂存目录根写入混淆 txt：模板替换 {date} 后追加随机 nonce 行（每包内容唯一）。
fn write_mix_txt(staging: &Path, template: &str) -> Result<(), String> {
    let date = chrono_like_today();
    let body = if template.trim().is_empty() {
        DEFAULT_TXT_TEMPLATE.to_string()
    } else {
        template.to_string()
    };
    let content = format!("{}\n#{}\n", body.replace("{date}", &date), random_hex(8));
    fs::write(staging.join(MIX_TXT_NAME), content)
        .map_err(|e| format!("写入混淆 txt 失败：{e}"))
}

/// 当前日期 YYYY-MM-DD。主目标平台是 Windows，直接取本地时间；
/// 其他平台退化为 UTC 日期（可能差一天，仅影响混淆 txt 里的日期文案）。
fn chrono_like_today() -> String {
    fmt_local_date()
}

#[cfg(windows)]
fn fmt_local_date() -> String {
    use std::os::raw::c_void;
    #[repr(C)]
    #[allow(non_snake_case)]
    struct SystemTime {
        w_year: u16,
        w_month: u16,
        w_day_of_week: u16,
        w_day: u16,
        w_hour: u16,
        w_minute: u16,
        w_second: u16,
        w_milliseconds: u16,
    }
    extern "system" {
        fn GetLocalTime(lpSystemTime: *mut c_void);
    }
    let mut st: SystemTime = unsafe { std::mem::zeroed() };
    unsafe {
        GetLocalTime(&mut st as *mut SystemTime as *mut c_void);
        format!("{:04}-{:02}-{:02}", st.w_year, st.w_month, st.w_day)
    }
}

#[cfg(not(windows))]
fn fmt_local_date() -> String {
    // 非 Windows 平台退化为 UTC 日期（打包工具主目标平台是 Windows）。
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// days since 1970-01-01 → (year, month, day)，Howard Hinnant 的 civil 算法。
#[cfg(not(windows))]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 把单个输入硬链接镜像进暂存目录根：文件直链、目录重建结构；
/// 同名自动 _2 去重；硬链接失败回退 copy（link_or_copy）。
fn stage_input(staging: &Path, source: &Path) -> Result<u64, String> {
    let base = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "input".to_string());
    let mut used: Vec<String> = Vec::new();
    let dst_name = dedupe_name(&mut used, &base);
    let dst = staging.join(&dst_name);
    let mut count = 0u64;
    if source.is_file() {
        crate::extract::link_or_copy(source, &dst);
        return Ok(1);
    }
    fs::create_dir_all(&dst).map_err(|e| format!("创建暂存目录失败：{e}"))?;
    for entry in WalkDir::new(source).min_depth(1) {
        let entry = entry.map_err(|e| format!("遍历源目录失败：{e}"))?;
        let rel = entry
            .path()
            .strip_prefix(source)
            .map_err(|e| format!("计算相对路径失败：{e}"))?;
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(|e| format!("创建暂存目录失败：{e}"))?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("创建暂存目录失败：{e}"))?;
            }
            crate::extract::link_or_copy(entry.path(), &target);
            count += 1;
        }
    }
    Ok(count)
}

/// 收集产物：单卷 {name}.{format}；分卷 {name}.{format}.001/.002…（按名排序）。
fn collect_outputs(dir: &Path, name: &str, format: &str) -> Vec<PathBuf> {
    let single = dir.join(format!("{name}.{format}"));
    if single.is_file() {
        return vec![single];
    }
    let prefix = format!("{name}.{format}.");
    let mut vols: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().starts_with(&prefix))
                    .unwrap_or(false)
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    vols.sort();
    vols
}

/// 打包单个源 → 一个包。返回 PackResult（含最终撞名后的名字与产物列表）。
fn pack_one(
    source: &Path,
    name: &str,
    opts: &PackOptions,
    password: Option<&str>,
    ex: &Extractor,
    cb: &dyn RunCallback,
) -> Result<PackResult, String> {
    let size = content_size(source);
    let out_dir = PathBuf::from(&opts.output_dir);
    let final_name = unique_pack_name(&out_dir, name, &opts.format);
    if final_name != name {
        let w = format!("{name}.{fmt} 已存在，改用 {final_name}.{fmt}", fmt = opts.format);
        cb.on_log(&format!("[警告] {w}"), LogLevel::Warn);
    }

    let staging = tempfile::Builder::new()
        .prefix("unzip-pack-")
        .tempdir()
        .map_err(|e| format!("创建暂存目录失败：{e}"))?;
    stage_input(staging.path(), source)?;
    if opts.mix_txt {
        write_mix_txt(staging.path(), &opts.txt_template)?;
    }

    let archive = out_dir.join(format!("{final_name}.{}", opts.format));
    let args = build_7z_args(&archive, size, opts, password);
    let Some(r) = ex.run_7z_in(staging.path(), &args) else {
        return Err("找不到 7z.exe，请检查 config.json 中的 seven_zip 路径".to_string());
    };
    if !(r.code.is_some_and(|c| c <= 1)) {
        let tail = r.text.trim().lines().last().unwrap_or("").chars().take(120).collect::<String>();
        return Err(format!("7z 打包失败：{tail}"));
    }

    let outputs = collect_outputs(&out_dir, &final_name, &opts.format);
    if outputs.is_empty() {
        return Err("打包未产出任何文件".to_string());
    }
    Ok(PackResult {
        name: final_name,
        password: password.map(|s| s.to_string()),
        size_bytes: size,
        outputs,
    })
}

/// 入口：逐源打包。命名从 name_start 起按 step 递增；密码按模式生成；
/// 包间检查 should_cancel（与解压侧取消粒度对齐，不杀子进程）。
pub fn run_pack(
    sources: &[PathBuf],
    opts: &PackOptions,
    cfg: &Config,
    cb: &dyn RunCallback,
) -> PackSummary {
    let mut summary = PackSummary::default();
    let log = |msg: &str, lv: LogLevel| cb.on_log(msg, lv);

    if sources.is_empty() {
        log("[警告] 待打包列表为空", LogLevel::Warn);
        summary.warns.push("待打包列表为空".to_string());
        return summary;
    }
    let out_dir = opts.output_dir.trim();
    if out_dir.is_empty() || !Path::new(out_dir).is_dir() {
        log("[错误] 打包输出目录无效", LogLevel::Error);
        summary
            .failed
            .push(("（全部）".to_string(), "打包输出目录无效".to_string()));
        return summary;
    }
    let out_abs = abs_path(Path::new(out_dir));
    let format = opts.format.trim().to_lowercase();
    if format != "7z" && format != "zip" {
        log(&format!("[错误] 不支持的压缩格式：{}（仅支持 7z / zip）", opts.format), LogLevel::Error);
        summary
            .failed
            .push(("（全部）".to_string(), format!("不支持的压缩格式：{}", opts.format)));
        return summary;
    }

    let ex = Extractor::new(cfg, None);
    if !ex.has_7z() {
        let reason = "找不到 7z.exe，请检查 config.json 中的 seven_zip 路径".to_string();
        log(&format!("[错误] {reason}"), LogLevel::Error);
        for s in sources {
            let n = s
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| s.display().to_string());
            summary.failed.push((n, reason.clone()));
        }
        return summary;
    }

    let step = if opts.name_step == 0 { 1 } else { opts.name_step };
    let mut name = sanitize_name(opts.name_start.trim());
    if name.is_empty() {
        name = "Pack".to_string();
        log("[警告] 包名为空，改用 Pack", LogLevel::Warn);
        summary.warns.push("包名为空，改用 Pack".to_string());
    }
    let mut opts_eff = opts.clone();
    opts_eff.format = format.clone();
    // 7z 在暂存目录里执行（current_dir），产物路径必须绝对，否则写错位置。
    opts_eff.output_dir = out_abs.to_string_lossy().into_owned();

    let mut uniform_pw: Option<String> = None;
    for (i, src) in sources.iter().enumerate() {
        if cb.should_cancel() {
            log("[警告] 已取消，剩余包未处理", LogLevel::Warn);
            summary.warns.push("用户取消，剩余包未处理".to_string());
            break;
        }
        let pw: Option<String> = match opts.password_mode.as_str() {
            "manual" => {
                let p = opts.uniform_password.trim().to_string();
                if p.is_empty() {
                    if i == 0 {
                        log("[警告] 统一密码为空，按无密码打包", LogLevel::Warn);
                        summary.warns.push("统一密码为空，按无密码打包".to_string());
                    }
                    None
                } else {
                    Some(p)
                }
            }
            "random_uniform" => {
                if uniform_pw.is_none() {
                    uniform_pw = Some(gen_password());
                }
                uniform_pw.clone()
            }
            _ => Some(gen_password()),
        };

        let size = content_size(src);
        log(
            &format!("[打包] {name} ← {}（{}）", src.display(), fmt_size(size)),
            LogLevel::Info,
        );
        if size > (opts.volume_threshold_gb * GB as f64) as u64 {
            log(
                &format!(
                    "[打包] 超过 {} GB 阈值，分卷为 {} GB/个",
                    fmt_gb(opts.volume_threshold_gb),
                    fmt_gb(opts.volume_size_gb)
                ),
                LogLevel::Info,
            );
        }
        if opts.mix_txt {
            log("[打包] 混入资源说明.txt（混淆 hash）", LogLevel::Skip);
        }

        match pack_one(src, &name, &opts_eff, pw.as_deref(), &ex, cb) {
            Ok(res) => {
                let vols = if res.outputs.len() > 1 {
                    format!("（{} 个分卷）", res.outputs.len())
                } else {
                    String::new()
                };
                let pwtxt = match &res.password {
                    Some(p) => format!("，密码：{p}"),
                    None => String::new(),
                };
                log(
                    &format!(
                        "[完成] {name}.{format}{vols}{pwtxt}",
                    ),
                    LogLevel::Ok,
                );
                summary.ok.push(res);
            }
            Err(e) => {
                log(&format!("[错误] {name} 打包失败：{e}"), LogLevel::Error);
                summary.failed.push((name.clone(), e));
            }
        }
        cb.on_progress(i + 1, sources.len(), &name);
        name = next_name(&name, step);
    }

    log(
        &format!(
            "[完成] 打包结束：成功 {}，失败 {}，警告 {}",
            summary.ok.len(),
            summary.failed.len(),
            summary.warns.len()
        ),
        if summary.failed.is_empty() { LogLevel::Ok } else { LogLevel::Warn },
    );
    summary
}

/// GB 数字去尾零显示（3.0 → "3"，2.5 → "2.5"）。
fn fmt_gb(gb: f64) -> String {
    let mut s = format!("{gb}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

/// 相对路径 → 基于进程 cwd 的绝对路径（不用 canonicalize，避免 Windows \\?\ 前缀）。
fn abs_path(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|c| c.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_name_basic() {
        assert_eq!(next_name("G198", 1), "G199");
        assert_eq!(next_name("H113", 1), "H114");
        assert_eq!(next_name("HG19238", 1), "HG19239");
        assert_eq!(next_name("G198", 5), "G203");
    }

    #[test]
    fn next_name_width_preserved_and_overflow() {
        assert_eq!(next_name("G001", 1), "G002");
        assert_eq!(next_name("G009", 1), "G010");
        assert_eq!(next_name("Z999", 1), "Z1000"); // 溢出变宽，不截断
    }

    #[test]
    fn next_name_no_digits() {
        assert_eq!(next_name("Game", 1), "Game1");
        assert_eq!(next_name("Game", 3), "Game3");
        assert_eq!(next_name("", 1), "1");
    }

    #[test]
    fn next_name_multibyte_safe() {
        assert_eq!(next_name("游戏G198", 1), "游戏G199");
        assert_eq!(next_name("整合包", 1), "整合包1");
    }

    #[test]
    fn gen_password_shape() {
        let a = gen_password();
        let b = gen_password();
        assert_eq!(a.len(), PW_LEN);
        assert_ne!(a, b);
        let charset: Vec<char> = "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789"
            .chars()
            .collect();
        assert!(a.chars().all(|c| charset.contains(&c)));
    }

    #[test]
    fn volume_arg_format() {
        assert_eq!(volume_arg(2.0), "-v2g");
        assert_eq!(volume_arg(3.0), "-v3g");
        assert_eq!(volume_arg(2.5), "-v2560m"); // 7z 不接受小数 g，转 MB
        assert_eq!(volume_arg(0.5), "-v512m");
        assert_eq!(volume_arg(0.0001), "-v64k"); // 下限
    }

    #[test]
    fn build_args_volumes_and_crypto() {
        let mut o = PackOptions::default();
        o.format = "7z".to_string();
        let small = build_7z_args(Path::new("o"), 2 * GB, &o, Some("pw123"));
        assert!(small.iter().any(|a| a == "-t7z"));
        assert!(small.iter().any(|a| a == "-mx5"));
        assert!(small.iter().any(|a| a == "-ppw123"));
        assert!(small.iter().any(|a| a == "-mhe=on"));
        assert!(!small.iter().any(|a| a.starts_with("-v")));

        let big = build_7z_args(Path::new("o"), 4 * GB, &o, Some("pw123"));
        assert!(big.iter().any(|a| a == "-v2g"));

        o.format = "zip".to_string();
        let zip = build_7z_args(Path::new("o"), 4 * GB, &o, Some("pw123"));
        assert!(zip.iter().any(|a| a == "-v2g"));
        assert!(zip.iter().any(|a| a == "-mem=AES256"));
        assert!(!zip.iter().any(|a| a == "-mhe=on"));

        let plain = build_7z_args(Path::new("o"), 4 * GB, &o, None);
        assert!(!plain.iter().any(|a| a.starts_with("-p")));
    }

    #[test]
    fn dedupe_name_suffix() {
        let mut used = Vec::new();
        assert_eq!(dedupe_name(&mut used, "a.txt"), "a.txt");
        assert_eq!(dedupe_name(&mut used, "a.txt"), "a_2.txt");
        assert_eq!(dedupe_name(&mut used, "a.txt"), "a_3.txt");
        assert_eq!(dedupe_name(&mut used, "b"), "b");
        assert_eq!(dedupe_name(&mut used, "B"), "B_2"); // ASCII 折叠：b 与 B 冲突
    }

    #[test]
    fn sanitize_strips_illegal() {
        assert_eq!(sanitize_name("a/b\\c:d"), "a_b_c_d");
        assert_eq!(sanitize_name("  G198  "), "G198");
    }

    #[test]
    fn content_size_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("a.bin"), vec![0u8; 100]).unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/b.bin"), vec![0u8; 50]).unwrap();
        assert_eq!(content_size(root), 150);
        assert_eq!(content_size(&root.join("a.bin")), 100);
        assert_eq!(content_size(&root.join("不存在")), 0);
    }

    #[test]
    fn fmt_size_human() {
        assert_eq!(fmt_size(300), "300 B");
        assert_eq!(fmt_size(2 * MB), "2.0 MB");
        assert_eq!(fmt_size((3.2 * GB as f64) as u64), "3.2 GB");
    }
}
