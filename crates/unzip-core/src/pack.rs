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
use serde_json::json;
use walkdir::WalkDir;

use crate::archive_hash::{hash_file, ArchiveHash};
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
    pub format: String,           // "7z" | "zip"
    pub password_mode: String,    // random_per_pack | random_uniform | manual
    pub uniform_password: String, // manual 模式手填；random_uniform 模式留空
    pub volume_threshold_gb: f64, // 内容总大小超过才分卷
    pub volume_size_gb: f64,      // 每个分卷的大小
    pub name_start: String,       // 起始包名（如 G198）
    pub name_step: u32,           // 数字递增步长
    pub name_mode: String,        // legacy（默认，兼容旧行为）| parts（前缀+数字+后缀）
    pub name_prefix: String,      // parts 模式前缀（prefix_enabled 时生效）
    pub name_suffix: String,      // parts 模式后缀（suffix_enabled 时生效）
    pub prefix_enabled: bool,
    pub suffix_enabled: bool,
    pub number_start: String, // parts 模式起始号码（纯 ASCII 数字，如 001）
    pub auto_number: bool,    // parts 模式按集中记录自动续号
    pub mix_txt: bool,        // 是否混入混淆 txt
    pub txt_template: String, // 支持 {date} 占位符
    pub compression_level: u32, // 0-9
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
            name_mode: "legacy".to_string(),
            name_prefix: String::new(),
            name_suffix: String::new(),
            prefix_enabled: false,
            suffix_enabled: false,
            number_start: "001".to_string(),
            auto_number: false,
            mix_txt: true,
            txt_template: DEFAULT_TXT_TEMPLATE.to_string(),
            compression_level: 5,
            output_dir: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PackResult {
    pub source_path: PathBuf,            // 源文件/目录路径
    pub source_name: String,             // 源文件/目录名称（用于 JSON 映射）
    pub name: String,                    // 最终包名（撞名时可能带 " (2)"）
    pub password: Option<String>,        // None = 无密码
    pub size_bytes: u64,                 // 源内容总大小
    pub outputs: Vec<PathBuf>,           // 产物文件（单卷或分卷序列）
    pub output_hashes: Vec<ArchiveHash>, // 每个压缩文件/分卷的 SHA-256
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
    format!("{}{:0w$}", &name[..start], next, w = width.max(natural))
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

/// 去掉已知压缩包扩展名（.7z/.zip/.rar，ASCII 不区分大小写）与可选数字卷尾
/// （结尾 `.` + 纯 ASCII 数字，如 .001），返回 stem；没有已知扩展名视为无关名称。
fn strip_archive_ext_and_tail(name: &str) -> Option<&str> {
    let mut s = name;
    if let Some(dot) = s.rfind('.') {
        let tail = &s[dot + 1..];
        if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) {
            s = &s[..dot];
        }
    }
    let lower = s.to_ascii_lowercase();
    for ext in [".7z", ".zip", ".rar"] {
        if lower.len() > ext.len() && lower.ends_with(ext) {
            return Some(&s[..s.len() - ext.len()]);
        }
    }
    None
}

/// catalog 里的名称是否属于指定系列：去扩展名/卷尾后须恰为
/// prefix + 非空纯 ASCII 数字 + suffix。忽略无关名称与撞名装饰（" (2)" 等，
/// 装饰使中间段不再是纯数字，自然失配）。
fn series_match(name: &str, prefix: &str, suffix: &str) -> Option<(u64, usize)> {
    let stem = strip_archive_ext_and_tail(name)?;
    let rest = stem.strip_prefix(prefix)?;
    let mid = rest.strip_suffix(suffix)?;
    if mid.is_empty() || !mid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number: u64 = mid.parse().ok()?;
    Some((number, mid.len()))
}

/// 自动续号起点：max(用户输入, 目录最大匹配号 + step)；宽度至少
/// max(输入宽度, 匹配最大宽度)。matches 为 (number, width)。
fn continuation_start(
    input: (u64, usize),
    matches: &[(u64, usize)],
    step: u32,
) -> Result<(u64, usize), String> {
    if matches.is_empty() {
        return Ok(input);
    }
    let max_num = matches.iter().map(|m| m.0).max().unwrap_or(0);
    let max_width = matches.iter().map(|m| m.1).max().unwrap_or(0);
    let next = max_num
        .checked_add(step as u64)
        .ok_or("已有包名序号已达上限，无法继续递增")?;
    Ok(((input.0).max(next), (input.1).max(max_width)))
}

/// 包名计划：legacy 走 name_start/next_name 旧行为；parts 为
/// effective_prefix + 定宽数字 + effective_suffix（无额外分隔符，宽度只增不减）。
#[derive(Clone, Debug)]
enum NamePlan {
    Legacy {
        start: String,
        step: u32,
    },
    Parts {
        prefix: String,
        suffix: String,
        start: u64,
        width: usize,
        step: u32,
    },
}

/// 按选项解析命名计划。legacy_start/legacy_step 为 run_pack 已有的旧逻辑
/// 归一化结果（空名 → Pack、step 为 0 → 1）。parts 模式拒绝空/非数字起始
/// 与非正步长，绝不静默改成别的序列；auto_number 读集中记录续号，
/// 读取失败即报错（绝不静默沿用可能撞号的起点）。
fn build_name_plan(
    opts: &PackOptions,
    legacy_start: &str,
    legacy_step: u32,
) -> Result<NamePlan, String> {
    if opts.name_mode.trim() != "parts" {
        return Ok(NamePlan::Legacy {
            start: legacy_start.to_string(),
            step: legacy_step,
        });
    }
    let prefix = if opts.prefix_enabled {
        sanitize_name(opts.name_prefix.trim())
    } else {
        String::new()
    };
    let suffix = if opts.suffix_enabled {
        sanitize_name(opts.name_suffix.trim())
    } else {
        String::new()
    };
    let digits = opts.number_start.trim();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!(
            "包名起始号码无效：{}（须为纯数字）",
            opts.number_start
        ));
    }
    let start: u64 = digits
        .parse()
        .map_err(|_| format!("包名起始号码超出范围：{digits}"))?;
    if opts.name_step == 0 {
        return Err("包名递增步长必须为正数".to_string());
    }
    let (start, width) = if opts.auto_number {
        let names = crate::archive_catalog::archive_names()?;
        let matches: Vec<(u64, usize)> = names
            .iter()
            .filter_map(|n| series_match(n, &prefix, &suffix))
            .collect();
        continuation_start((start, digits.len()), &matches, opts.name_step)?
    } else {
        (start, digits.len())
    };
    Ok(NamePlan::Parts {
        prefix,
        suffix,
        start,
        width,
        step: opts.name_step,
    })
}

/// 计划第 index 个（0 起）包名；与 run_pack 循环内命名完全同一条路径。
fn plan_at(plan: &NamePlan, index: usize) -> String {
    match plan {
        NamePlan::Legacy { start, step } => {
            let mut name = start.clone();
            for _ in 0..index {
                name = next_name(&name, *step);
            }
            name
        }
        NamePlan::Parts {
            prefix,
            suffix,
            start,
            width,
            step,
        } => {
            let num = *start + index as u64 * *step as u64;
            let w = (*width).max(num.to_string().len());
            format!("{prefix}{num:0w$}{suffix}")
        }
    }
}

fn validate_plan_count(plan: &NamePlan, count: usize) -> Result<(), String> {
    if let NamePlan::Parts { start, step, .. } = plan {
        if count > 0 {
            let delta = (count - 1) as u64;
            let increment = delta.checked_mul(*step as u64).ok_or("包名序号超出范围")?;
            start.checked_add(increment).ok_or("包名序号超出范围")?;
        }
    }
    Ok(())
}

/// 预览 run_pack 将生成的 count 个包名（仅基名，不含 .7z/.zip 扩展名），
/// 与 run_pack 使用同一条命名解析路径。count 上限 1000。
pub fn preview_pack_names(opts: &PackOptions, count: usize) -> Result<Vec<String>, String> {
    if count > 1000 {
        return Err("预览数量过多（最多 1000）".to_string());
    }
    let step = if opts.name_step == 0 {
        1
    } else {
        opts.name_step
    };
    let start = sanitize_name(opts.name_start.trim());
    let legacy_start = if start.is_empty() {
        "Pack".to_string()
    } else {
        start
    };
    let plan = build_name_plan(opts, &legacy_start, step)?;
    validate_plan_count(&plan, count)?;
    Ok((0..count).map(|i| plan_at(&plan, i)).collect())
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
    (0..n)
        .map(|_| HEX[range.sample(&mut rng)] as char)
        .collect()
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
fn build_7z_args(
    archive: &Path,
    size: u64,
    opts: &PackOptions,
    password: Option<&str>,
) -> Vec<String> {
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
    fs::write(staging.join(MIX_TXT_NAME), content).map_err(|e| format!("写入混淆 txt 失败：{e}"))
}

/// 当前日期 YYYY-MM-DD。主目标平台是 Windows，直接取本地时间；
/// 其他平台退化为 UTC 日期（可能差一天，仅影响混淆 txt 里的日期文案）。
pub(crate) fn chrono_like_today() -> String {
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

/// 打包单个源 → 一个包。返回 PackResult（含源路径/名称、最终撞名后的名字与产物列表）。
fn pack_one(
    source: &Path,
    name: &str,
    opts: &PackOptions,
    password: Option<&str>,
    ex: &Extractor,
    cb: &dyn RunCallback,
) -> Result<PackResult, String> {
    let source_name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| source.display().to_string());
    let size = content_size(source);
    let out_dir = PathBuf::from(&opts.output_dir);
    let final_name = unique_pack_name(&out_dir, name, &opts.format);
    if final_name != name {
        let w = format!(
            "{name}.{fmt} 已存在，改用 {final_name}.{fmt}",
            fmt = opts.format
        );
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
        let tail = r
            .text
            .trim()
            .lines()
            .last()
            .unwrap_or("")
            .chars()
            .take(120)
            .collect::<String>();
        return Err(format!("7z 打包失败：{tail}"));
    }

    let mut outputs = collect_outputs(&out_dir, &final_name, &opts.format);
    if outputs.is_empty() {
        return Err("打包未产出任何文件".to_string());
    }

    // 优化：启用了分卷但实际只生成一个文件（压缩后小于分卷大小）时，去掉 .001 后缀
    if outputs.len() == 1 {
        let single = &outputs[0];
        if let Some(fname) = single.file_name().and_then(|n| n.to_str()) {
            // 检查是否为 {name}.{format}.001 形态
            let expected_vol = format!("{}.{}.001", final_name, opts.format);
            if fname == expected_vol {
                let target = out_dir.join(format!("{}.{}", final_name, opts.format));
                match fs::rename(single, &target) {
                    Ok(_) => {
                        outputs[0] = target;
                        cb.on_log(
                            &format!("[优化] 单文件分卷已重命名为 {}.{}", final_name, opts.format),
                            LogLevel::Info,
                        );
                    }
                    Err(e) => {
                        cb.on_log(&format!("[警告] 重命名单文件分卷失败：{e}"), LogLevel::Warn);
                    }
                }
            }
        }
    }

    let mut output_hashes = Vec::new();
    for output in &outputs {
        if cb.should_cancel() {
            break;
        }
        cb.on_log(
            &format!(
                "[哈希] 正在计算 {} 的 SHA-256",
                output.file_name().unwrap_or_default().to_string_lossy()
            ),
            LogLevel::Info,
        );
        match hash_file(output, cb) {
            Ok(hash) => output_hashes.push(hash),
            Err(e) => cb.on_log(
                &format!(
                    "[警告] 压缩已完成，但未能记录 {} 的哈希：{e}",
                    output.display()
                ),
                LogLevel::Warn,
            ),
        }
    }

    Ok(PackResult {
        source_path: source.to_path_buf(),
        source_name,
        name: final_name,
        password: password.map(|s| s.to_string()),
        size_bytes: size,
        outputs,
        output_hashes,
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
        log(
            &format!(
                "[错误] 不支持的压缩格式：{}（仅支持 7z / zip）",
                opts.format
            ),
            LogLevel::Error,
        );
        summary.failed.push((
            "（全部）".to_string(),
            format!("不支持的压缩格式：{}", opts.format),
        ));
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

    let step = if opts.name_step == 0 {
        1
    } else {
        opts.name_step
    };
    let start_name = sanitize_name(opts.name_start.trim());
    let legacy_empty = opts.name_mode.trim() != "parts" && start_name.is_empty();
    let legacy_start = if start_name.is_empty() {
        "Pack".to_string()
    } else {
        start_name
    };
    if legacy_empty {
        log("[警告] 包名为空，改用 Pack", LogLevel::Warn);
        summary.warns.push("包名为空，改用 Pack".to_string());
    }
    // parts 模式在这里（进入逐源循环前）重新解析起点；续号失败直接整体报错，
    // 绝不静默沿用可能撞号的起点。
    let plan = match build_name_plan(opts, &legacy_start, step).and_then(|plan| {
        validate_plan_count(&plan, sources.len())?;
        Ok(plan)
    }) {
        Ok(p) => p,
        Err(e) => {
            log(&format!("[错误] {e}"), LogLevel::Error);
            for s in sources {
                let n = s
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_else(|| s.display().to_string());
                summary.failed.push((n, e.clone()));
            }
            return summary;
        }
    };
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
        let name = plan_at(&plan, i);
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
                if res.output_hashes.len() != res.outputs.len() {
                    summary.warns.push(format!(
                        "{} 的部分文件未记录 SHA-256（已取消或读取失败），压缩文件和密码记录仍保留",
                        res.name
                    ));
                }
                if let Err(e) = crate::archive_catalog::upsert(vec![mapping_record(&res)]) {
                    let warning = format!("集中保存压缩记录失败：{e}");
                    log(&format!("[警告] {warning}"), LogLevel::Warn);
                    summary.warns.push(warning);
                    // Keep a recovery copy next to the outputs if AppData is unwritable.
                    if let Err(e) = write_mapping_json(&out_abs, std::slice::from_ref(&res), cb) {
                        let warning = format!("密码记录备份也失败，请保存打包结果中的密码：{e}");
                        log(&format!("[警告] {warning}"), LogLevel::Warn);
                        summary.warns.push(warning);
                    }
                }
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
                    &format!("[完成] {name}.{format}{vols}{pwtxt}",),
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
    }

    log(
        &format!(
            "[完成] 打包结束：成功 {}，失败 {}，警告 {}",
            summary.ok.len(),
            summary.failed.len(),
            summary.warns.len()
        ),
        if summary.failed.is_empty() {
            LogLevel::Ok
        } else {
            LogLevel::Warn
        },
    );

    if !summary.ok.is_empty() {
        log(
            &format!(
                "[记录] 集中记录位置：{}；保存异常请查看上方警告",
                crate::archive_catalog::database_path().display()
            ),
            LogLevel::Info,
        );
    }

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

fn mapping_record(r: &PackResult) -> serde_json::Value {
    let compressed_bytes = r
        .outputs
        .iter()
        .filter_map(|p| p.metadata().ok())
        .map(|m| m.len())
        .sum();
    let size = |bytes: u64| {
        json!({
            "bytes": bytes, "mb": format!("{:.2}", bytes as f64 / MB as f64),
            "gb": format!("{:.3}", bytes as f64 / GB as f64), "human": fmt_size(bytes),
        })
    };
    let first = r.outputs.first().cloned().unwrap_or_default();
    json!({
        "record_id": crate::archive_catalog::record_id(&first),
        "archive_path": first,
        "output_dir": first.parent(),
        "source_name": r.source_name,
        "archive_name": first.file_name().unwrap_or_default().to_string_lossy(),
        "password": r.password,
        "source_size": size(r.size_bytes),
        "compressed_size": size(compressed_bytes),
        "compression_ratio": if r.size_bytes > 0 { format!("{:.1}%", compressed_bytes as f64 / r.size_bytes as f64 * 100.0) } else { "0%".into() },
        "is_volume": r.outputs.len() > 1,
        "volume_count": r.outputs.len(),
        "created_at": chrono_like_today(),
        "hash_algorithm": "sha256",
        "output_hashes": r.output_hashes,
    })
}

/// Emergency recovery JSON if the centralized AppData catalog cannot be written.
/// 支持增量更新：按 archive_name 去重，新包覆盖同名旧包。
fn write_mapping_json(
    out_dir: &Path,
    results: &[PackResult],
    cb: &dyn RunCallback,
) -> Result<(), String> {
    let json_path = out_dir.join("pack_mapping.json");
    let mut data: serde_json::Value = if json_path.exists() {
        let text = fs::read_to_string(&json_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("已有备份 JSON 无效，未覆盖：{e}"))?
    } else {
        json!({"mappings": []})
    };
    let mappings = data["mappings"]
        .as_array_mut()
        .ok_or("已有备份缺少映射列表，未覆盖")?;
    for result in results {
        let item = mapping_record(result);
        if let Some(old) = mappings
            .iter_mut()
            .find(|old| old["archive_name"] == item["archive_name"])
        {
            let old = old.as_object_mut().ok_or("旧映射无效")?;
            for (key, value) in item.as_object().ok_or("新映射无效")? {
                if key != "metadata" {
                    old.insert(key.clone(), value.clone());
                }
            }
        } else {
            mappings.push(item);
        }
    }
    data["total_count"] = json!(mappings.len());
    data["generated_at"] = json!(chrono_like_today());
    data["schema_version"] = json!(2);
    let mut tmp = tempfile::NamedTempFile::new_in(out_dir).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut tmp, &data).map_err(|e| e.to_string())?;
    tmp.as_file().sync_all().map_err(|e| e.to_string())?;
    tmp.persist(&json_path).map_err(|e| e.error.to_string())?;
    cb.on_log(
        &format!("[备份] 已保留恢复记录：{}", json_path.display()),
        LogLevel::Warn,
    );
    Ok(())
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

    #[test]
    fn write_mapping_json_generates_correct_structure() {
        use crate::types::NullCallback;

        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path();

        let results = vec![
            PackResult {
                source_path: PathBuf::from("D:/source/game1"),
                source_name: "game1".to_string(),
                name: "G001".to_string(),
                password: Some("pass123".to_string()),
                size_bytes: 1024 * 1024 * 500, // 500 MB
                outputs: vec![out_dir.join("G001.7z")],
                output_hashes: Vec::new(),
            },
            PackResult {
                source_path: PathBuf::from("D:/source/game2"),
                source_name: "game2".to_string(),
                name: "G002".to_string(),
                password: None,
                size_bytes: 1024 * 1024 * 1024 * 3, // 3 GB
                outputs: vec![out_dir.join("G002.7z.001"), out_dir.join("G002.7z.002")],
                output_hashes: Vec::new(),
            },
        ];

        let cb = NullCallback;
        write_mapping_json(out_dir, &results, &cb).unwrap();

        let json_path = out_dir.join("pack_mapping.json");
        assert!(json_path.exists());

        let content = fs::read_to_string(&json_path).unwrap();
        let data: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(data["total_count"], 2);
        assert_eq!(data["mappings"].as_array().unwrap().len(), 2);

        let m0 = &data["mappings"][0];
        assert_eq!(m0["source_name"], "game1");
        assert_eq!(m0["archive_name"], "G001.7z");
        assert_eq!(m0["password"], "pass123");
        assert_eq!(m0["is_volume"], false);
        assert_eq!(m0["volume_count"], 1);

        let m1 = &data["mappings"][1];
        assert_eq!(m1["source_name"], "game2");
        assert_eq!(m1["archive_name"], "G002.7z.001");
        assert!(m1["password"].is_null());
        assert_eq!(m1["is_volume"], true);
        assert_eq!(m1["volume_count"], 2);
    }

    // ---- parts 模式命名（PACK-NAMING-CORE）----

    fn parts_opts() -> PackOptions {
        let mut o = PackOptions::default();
        o.name_mode = "parts".to_string();
        o
    }

    #[test]
    fn parts_fixed_suffix_increments_middle_only() {
        let mut o = parts_opts();
        o.suffix_enabled = true;
        o.name_suffix = "v".to_string();
        o.number_start = "001".to_string();
        o.name_step = 1;
        let plan = build_name_plan(&o, "ignored", 1).unwrap();
        assert_eq!(plan_at(&plan, 0), "001v");
        assert_eq!(plan_at(&plan, 1), "002v");
        assert_eq!(plan_at(&plan, 2), "003v");
    }

    #[test]
    fn parts_prefix_and_suffix_enabled_disabled() {
        let mut o = parts_opts();
        o.prefix_enabled = true;
        o.name_prefix = "G".to_string();
        o.number_start = "7".to_string();
        let plan = build_name_plan(&o, "x", 1).unwrap();
        assert_eq!(plan_at(&plan, 0), "G7");
        assert_eq!(plan_at(&plan, 1), "G8");

        // 双关闭 = 纯数字序列。
        let plan2 = build_name_plan(&parts_opts(), "x", 1).unwrap();
        assert_eq!(plan_at(&plan2, 0), "001");
        assert_eq!(plan_at(&plan2, 1), "002");

        // 双开 + 步长；非法字符按 Windows 规则清洗。
        let mut o3 = parts_opts();
        o3.prefix_enabled = true;
        o3.name_prefix = "游戏-".to_string();
        o3.suffix_enabled = true;
        o3.name_suffix = "_整?版".to_string();
        o3.number_start = "12".to_string();
        o3.name_step = 10;
        let plan3 = build_name_plan(&o3, "x", 1).unwrap();
        assert_eq!(plan_at(&plan3, 0), "游戏-12_整_版");
        assert_eq!(plan_at(&plan3, 1), "游戏-22_整_版");
    }

    #[test]
    fn parts_width_preserved_and_rollover_widens() {
        let mut o = parts_opts();
        o.number_start = "098".to_string();
        o.name_step = 5;
        let plan = build_name_plan(&o, "x", 1).unwrap();
        assert_eq!(plan_at(&plan, 0), "098");
        assert_eq!(plan_at(&plan, 1), "103"); // 宽度保持 3

        let mut o2 = parts_opts();
        o2.number_start = "999".to_string();
        o2.name_step = 1;
        let plan2 = build_name_plan(&o2, "x", 1).unwrap();
        assert_eq!(plan_at(&plan2, 0), "999");
        assert_eq!(plan_at(&plan2, 1), "1000"); // 溢出变宽，不截断
    }

    #[test]
    fn parts_rejects_batch_sequence_overflow_before_packing() {
        let mut opts = parts_opts();
        opts.number_start = u64::MAX.to_string();
        let plan = build_name_plan(&opts, "x", 1).unwrap();
        assert!(validate_plan_count(&plan, 1).is_ok());
        assert!(validate_plan_count(&plan, 2).is_err());
        assert!(preview_pack_names(&opts, 2).is_err());
    }

    #[test]
    fn parts_rejects_bad_start_and_nonpositive_step() {
        let mut o = parts_opts();
        o.number_start = String::new();
        assert!(build_name_plan(&o, "x", 1).is_err());
        o.number_start = "12a".to_string();
        assert!(build_name_plan(&o, "x", 1).is_err());
        o.number_start = "-3".to_string();
        assert!(build_name_plan(&o, "x", 1).is_err());
        o.number_start = "9".repeat(30); // 超出 u64
        assert!(build_name_plan(&o, "x", 1).is_err());
        o.number_start = "001".to_string();
        o.name_step = 0;
        assert!(build_name_plan(&o, "x", 1).is_err());
    }

    #[test]
    fn series_match_exact_series_only() {
        assert_eq!(series_match("G198.7z", "G", ""), Some((198, 3)));
        assert_eq!(series_match("G198.7z", "G", "v"), None);
        assert_eq!(series_match("G198v.7z", "G", "v"), Some((198, 3)));
        assert_eq!(series_match("G198 (2).7z", "G", ""), None); // 撞名装饰忽略
        assert_eq!(series_match("HG198.7z", "G", ""), None); // 前缀须从头精确
        assert_eq!(series_match("G198x.7z", "G", ""), None);
        assert_eq!(series_match("游戏.7z", "", ""), None); // 中间须为非空数字
        assert_eq!(series_match("readme.txt", "", ""), None); // 未知扩展名
        assert_eq!(series_match("g198.ZIP", "G", ""), None); // 前后缀精确（大小写敏感）
    }

    #[test]
    fn series_match_mixed_formats_and_volume_tails() {
        assert_eq!(series_match("G198.zip", "G", ""), Some((198, 3)));
        assert_eq!(series_match("G198.rar", "G", ""), Some((198, 3)));
        assert_eq!(series_match("G198.7z.001", "G", ""), Some((198, 3)));
        assert_eq!(series_match("G198.zip.002", "G", ""), Some((198, 3)));
        assert_eq!(series_match("g198.zip", "g", ""), Some((198, 3)));
        assert_eq!(series_match("G198", "G", ""), None); // 无已知扩展名
        assert_eq!(series_match("G198.7z.00a", "G", ""), None); // 卷尾非数字
    }

    #[test]
    fn continuation_max_plus_step_and_user_floor() {
        assert_eq!(
            continuation_start((1, 3), &[(198, 3)], 1).unwrap(),
            (199, 3)
        );
        assert_eq!(
            continuation_start((500, 3), &[(198, 3)], 1).unwrap(),
            (500, 3)
        ); // 用户输入更大
        assert_eq!(continuation_start((7, 2), &[], 5).unwrap(), (7, 2)); // 无匹配沿用输入
        assert_eq!(continuation_start((1, 3), &[(99, 5)], 1).unwrap(), (100, 5)); // 匹配更宽则变宽
        assert_eq!(
            continuation_start((1, 3), &[(999, 3)], 1).unwrap(),
            (1000, 3)
        ); // 溢出宽度由格式化自然加宽
        assert!(continuation_start((1, 1), &[(u64::MAX, 20)], 1).is_err());
    }

    struct CatalogEnvGuard(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

    impl Drop for CatalogEnvGuard {
        fn drop(&mut self) {
            std::env::remove_var("UNZIP_CATALOG_PATH");
        }
    }

    fn catalog_env() -> (CatalogEnvGuard, tempfile::TempDir) {
        let guard = crate::archive_catalog::CATALOG_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("UNZIP_CATALOG_PATH", dir.path().join("pack_mapping.json"));
        (CatalogEnvGuard(guard), dir)
    }

    fn catalog_record(record_id: &str, archive_name: &str) -> serde_json::Value {
        serde_json::json!({
            "record_id": record_id,
            "archive_path": format!("D:/Out/{archive_name}"),
            "output_dir": "D:/Out",
            "source_name": archive_name,
            "archive_name": archive_name,
            "password": null,
            "source_size": {"bytes": 1, "mb": "0.00", "gb": "0.000", "human": "1 B"},
            "compressed_size": {"bytes": 1, "mb": "0.00", "gb": "0.000", "human": "1 B"},
            "compression_ratio": "100.0%",
            "is_volume": false,
            "volume_count": 1,
            "created_at": "2026-10-17",
        })
    }

    #[test]
    fn auto_number_continues_from_catalog_across_output_dirs() {
        let (_guard, _dir) = catalog_env();
        crate::archive_catalog::upsert(vec![
            catalog_record("d:/out1/g198.7z", "G198.7z"),
            catalog_record("d:/other/g199.7z", "G199.7z"), // 其他输出目录同样计入
            catalog_record("d:/out1/h300.7z", "H300.7z"),  // 别的系列不匹配
            catalog_record("d:/out1/g7.zip.001", "G7.zip.001"), // 分卷名也匹配
            catalog_record("d:/out1/g198 (2).7z", "G198 (2).7z"), // 撞名装饰不匹配
        ])
        .unwrap();
        let mut o = parts_opts();
        o.prefix_enabled = true;
        o.name_prefix = "G".to_string();
        o.number_start = "001".to_string();
        o.name_step = 1;
        o.auto_number = true;
        let plan = build_name_plan(&o, "x", 1).unwrap();
        assert_eq!(plan_at(&plan, 0), "G200"); // max(198,199,7)+1
        assert_eq!(plan_at(&plan, 1), "G201");
        let names = preview_pack_names(&o, 2).unwrap();
        assert_eq!(names, vec!["G200".to_string(), "G201".to_string()]);
    }

    #[test]
    fn real_pack_uses_middle_number_and_next_batch_continues_from_catalog() {
        let seven_zip = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/bundled/7z.exe");
        if !seven_zip.is_file() {
            return; // Integration component is optional for source-only builds.
        }
        let (_guard, dir) = catalog_env();
        let out = dir.path().join("out");
        fs::create_dir_all(&out).unwrap();
        let sources: Vec<PathBuf> = (0..3)
            .map(|index| {
                let path = dir.path().join(format!("source{index}.txt"));
                fs::write(&path, format!("content {index}")).unwrap();
                path
            })
            .collect();
        let mut cfg = Config::default();
        cfg.seven_zip = seven_zip.to_string_lossy().into_owned();
        let mut opts = parts_opts();
        opts.output_dir = out.to_string_lossy().into_owned();
        opts.prefix_enabled = true;
        opts.name_prefix = "G".into();
        opts.suffix_enabled = true;
        opts.name_suffix = "_v".into();
        opts.number_start = "001".into();
        opts.name_step = 2;
        opts.auto_number = false;
        opts.password_mode = "manual".into();
        opts.mix_txt = false;
        let callback = crate::types::NullCallback;

        let first = run_pack(&sources[..2], &opts, &cfg, &callback);
        assert!(first.failed.is_empty(), "{:?}", first.failed);
        assert_eq!(
            first.ok.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["G001_v", "G003_v"]
        );
        assert!(out.join("G001_v.7z").is_file());
        assert!(out.join("G003_v.7z").is_file());

        opts.auto_number = true;
        let second = run_pack(&sources[2..], &opts, &cfg, &callback);
        assert!(second.failed.is_empty(), "{:?}", second.failed);
        assert_eq!(second.ok[0].name, "G005_v");
        assert!(out.join("G005_v.7z").is_file());
    }

    #[test]
    fn auto_number_fails_loud_on_catalog_read_error() {
        let (_guard, dir) = catalog_env();
        fs::write(dir.path().join("pack_mapping.json"), "{ not json").unwrap();
        let mut o = parts_opts();
        o.auto_number = true;
        let err = build_name_plan(&o, "x", 1).expect_err("invalid catalog must error");
        assert!(err.contains("记录"), "unexpected error: {err}");
    }

    #[test]
    fn preview_matches_run_pack_resolution_and_bounds_count() {
        let mut o = PackOptions::default(); // legacy 兼容
        o.name_start = "G198".to_string();
        o.name_step = 1;
        assert_eq!(
            preview_pack_names(&o, 3).unwrap(),
            vec!["G198", "G199", "G200"]
        );

        let mut e = PackOptions::default();
        e.name_start = String::new();
        assert_eq!(preview_pack_names(&e, 2).unwrap(), vec!["Pack", "Pack1"]);

        assert!(preview_pack_names(&o, 1001).is_err());
        assert_eq!(preview_pack_names(&o, 0).unwrap(), Vec::<String>::new());
    }
}
