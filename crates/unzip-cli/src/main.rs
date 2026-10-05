//! unzip-cli — 自动解压工具命令行入口（移植自原 Python 版 auto_unzip，行为对齐）。

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use unzip_core::config::{self, Config};
use unzip_core::types::{LogLevel, RunCallback, Summary};

/// 自动解压工具：识别伪装/改名压缩包，分卷自动归集，lz4 解码，密码自动尝试，按最外层目录分组
#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// 打包压缩：命名递增、自动密码、阈值分卷、混淆 txt（见 unzip-cli pack --help）
    #[command(subcommand)]
    command: Option<Commands>,
    /// 要处理的文件或目录，可多个
    targets: Vec<PathBuf>,
    /// 解压产物根目录（默认取 config.json 的 output_dir）
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// 失败包移入的目录（默认 config.json 的 failed_dir 或 <输出目录>/解压失败）
    #[arg(long = "failed-dir")]
    failed_dir: Option<PathBuf>,
    /// 递归扫描子目录（默认已开启，可配置）
    #[arg(short, long)]
    recursive: bool,
    /// 不递归子目录
    #[arg(long = "no-recursive")]
    no_recursive: bool,
    /// 临时追加密码（可多次指定），仅本次运行有效
    #[arg(long)]
    password: Vec<String>,
    /// 把密码写入 config.json 密码库后退出
    #[arg(long = "add-password")]
    add_password: Option<String>,
    /// 从 config.json 密码库删除密码后退出
    #[arg(long = "remove-password")]
    remove_password: Option<String>,
    /// 列出密码库和排序规则后退出
    #[arg(long = "list-passwords")]
    list_passwords: bool,
    /// 只识别不执行，预览每个文件会被如何处理
    #[arg(long = "dry-run")]
    dry_run: bool,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// 打包压缩：每个源单独打一个包，命名递增、自动密码、超阈值分卷、混入混淆 txt
    Pack(PackArgs),
}

#[derive(clap::Args)]
struct PackArgs {
    /// 要打包的文件或目录，可多个（每个源单独打一个包）
    sources: Vec<PathBuf>,
    /// 打包输出目录（不存在会自动创建）
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// 起始包名，末尾数字按步长递增（如 G198 → G199 …）
    #[arg(long, default_value = "G001")]
    name: String,
    /// 数字递增步长
    #[arg(long, default_value_t = 1)]
    step: u32,
    /// 压缩格式：7z | zip
    #[arg(long, default_value = "7z")]
    format: String,
    /// 一批包共用一个自动生成的随机密码（与 --password 互斥）
    #[arg(long = "random-uniform")]
    random_uniform: bool,
    /// 一批包共用手填密码（与 --random-uniform 互斥）
    #[arg(long)]
    password: Option<String>,
    /// 内容总大小超过该值才分卷（GB，默认 3）
    #[arg(long, default_value_t = 3.0)]
    volume_threshold: f64,
    /// 每个分卷的大小（GB，默认 2）
    #[arg(long, default_value_t = 2.0)]
    volume_size: f64,
    /// 关闭混淆 txt（默认开启：向包内混入随机化的资源说明.txt，避免网盘按 hash 比对）
    #[arg(long = "no-txt")]
    no_txt: bool,
    /// 混淆 txt 模板文案，{date} 为打包日期占位符
    #[arg(long)]
    txt_template: Option<String>,
    /// 压缩级别 0-9（默认 5）
    #[arg(long, default_value_t = 5)]
    mx: u32,
    /// 只预览打包计划（包名/大小/是否分卷/密码策略）不执行
    #[arg(long = "dry-run")]
    dry_run: bool,
}

struct PrintCallback;

impl RunCallback for PrintCallback {
    fn on_log(&self, msg: &str, _level: LogLevel) {
        println!("{msg}");
    }
}

fn run_pack_cmd(pa: PackArgs, cfg: &Config) -> ExitCode {
    if pa.sources.is_empty() {
        eprintln!("[错误] 请指定要打包的文件或目录");
        return ExitCode::FAILURE;
    }
    if pa.random_uniform && pa.password.is_some() {
        eprintln!("[错误] --random-uniform 与 --password 互斥，只能选一个");
        return ExitCode::FAILURE;
    }

    let out_dir = pa
        .output
        .or_else(|| (!cfg.output_dir.is_empty()).then(|| PathBuf::from(&cfg.output_dir)))
        .unwrap_or_else(|| config::exe_dir().join("打包结果"));
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("[错误] 无法创建输出目录 {}：{e}", out_dir.display());
        return ExitCode::FAILURE;
    }

    let password_mode = if pa.password.is_some() {
        "manual"
    } else if pa.random_uniform {
        "random_uniform"
    } else {
        "random_per_pack"
    };
    let opts = unzip_core::pack::PackOptions {
        format: pa.format.clone().trim().to_lowercase(),
        password_mode: password_mode.to_string(),
        uniform_password: pa.password.clone().unwrap_or_default(),
        volume_threshold_gb: pa.volume_threshold,
        volume_size_gb: pa.volume_size,
        name_start: pa.name.clone(),
        name_step: pa.step,
        mix_txt: !pa.no_txt,
        txt_template: pa
            .txt_template
            .clone()
            .unwrap_or_else(|| unzip_core::pack::DEFAULT_TXT_TEMPLATE.to_string()),
        compression_level: pa.mx,
        output_dir: out_dir.to_string_lossy().into_owned(),
    };

    // 校验源路径
    let mut sources: Vec<PathBuf> = Vec::new();
    for s in &pa.sources {
        if s.exists() {
            sources.push(s.clone());
        } else {
            println!("[警告] 路径不存在：{}", s.display());
        }
    }
    if sources.is_empty() {
        eprintln!("[错误] 没有有效的打包源");
        return ExitCode::FAILURE;
    }

    if pa.dry_run {
        println!("输出目录：{}", out_dir.display());
        println!(
            "格式：{}，密码模式：{}，分卷：>{:.1} GB 时按 {:.1} GB/个{}",
            opts.format,
            match password_mode {
                "manual" => format!("统一手填（{}）", pa.password.as_deref().unwrap_or("")),
                "random_uniform" => "统一随机".to_string(),
                _ => "每包随机".to_string(),
            },
            pa.volume_threshold,
            pa.volume_size,
            if opts.mix_txt { "，混入混淆 txt" } else { "" },
        );
        println!();
        let step = if pa.step == 0 { 1 } else { pa.step };
        let mut name = unzip_core::pack::sanitize_name(pa.name.trim());
        if name.is_empty() {
            name = "Pack".to_string();
        }
        let gb = 1024.0 * 1024.0 * 1024.0;
        for src in &sources {
            let size = unzip_core::pack::content_size(src);
            let split = size as f64 > pa.volume_threshold * gb;
            println!(
                "{} ← {}（{}{}）",
                name,
                src.display(),
                unzip_core::pack::fmt_size(size),
                if split {
                    format!("，超过 {:.1} GB → 分卷 {:.1} GB/个", pa.volume_threshold, pa.volume_size)
                } else {
                    String::new()
                },
            );
            name = unzip_core::pack::next_name(&name, step);
        }
        println!();
        println!("（--dry-run 预览完毕，未执行打包）");
        return ExitCode::SUCCESS;
    }

    println!("输出目录：{}", out_dir.display());
    println!();
    let summary = unzip_core::run_pack(&sources, &opts, cfg, &PrintCallback);

    println!("{}", "-".repeat(50));
    println!(
        "完成：成功 {}，失败 {}，警告 {}",
        summary.ok.len(),
        summary.failed.len(),
        summary.warns.len()
    );
    for r in &summary.ok {
        let pw = r.password.as_deref().unwrap_or("（无密码）");
        println!(
            "  - {}（{}，密码：{pw}）",
            r.name,
            unzip_core::pack::fmt_size(r.size_bytes)
        );
    }
    for (name, reason) in &summary.failed {
        println!("  - 失败：{name} — {reason}");
    }
    if summary.failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn list_passwords(cfg: &Config) {
    println!("密码排序规则：{}（list_order=按列表顺序；recent_first=最近成功的优先）",
             cfg.password_strategy);
    if cfg.passwords.is_empty() {
        println!("密码库为空，用 --add-password 密码 添加");
    }
    for (i, pw) in cfg.passwords.iter().enumerate() {
        println!("  {}. {}", i + 1, pw);
    }
    if !cfg.password_rules.is_empty() {
        println!("密码策略规则（命中的规则按序优先尝试，其余按密码库顺序）：");
        for (i, r) in cfg.password_rules.iter().enumerate() {
            println!("  {}. {}", i + 1, config::format_rule(r));
        }
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    let mut cfg = config::load_config();

    if let Some(Commands::Pack(pa)) = args.command {
        return run_pack_cmd(pa, &cfg);
    }

    if args.list_passwords {
        list_passwords(&cfg);
        return ExitCode::SUCCESS;
    }

    if let Some(pw) = args.add_password {
        if cfg.passwords.contains(&pw) {
            println!("密码已存在：{pw}");
        } else {
            cfg.passwords.push(pw.clone());
            println!("{}{pw}",
                if config::save_config(&cfg) { "已添加" } else { "添加成功但写入 config.json 失败：" });
        }
        return ExitCode::SUCCESS;
    }

    if let Some(pw) = args.remove_password {
        if let Some(pos) = cfg.passwords.iter().position(|p| *p == pw) {
            cfg.passwords.remove(pos);
            config::save_config(&cfg);
            println!("已删除密码：{pw}");
        } else {
            println!("密码库中没有：{pw}");
        }
        return ExitCode::SUCCESS;
    }

    if args.targets.is_empty() {
        use clap::CommandFactory;
        Args::command().print_help().ok();
        return ExitCode::SUCCESS;
    }

    let recursive = if args.no_recursive {
        false
    } else {
        args.recursive || cfg.recursive
    };

    let out_root = args.output
        .or_else(|| (!cfg.output_dir.is_empty()).then(|| PathBuf::from(&cfg.output_dir)))
        .unwrap_or_else(config::default_output_dir);
    let failed_root = args.failed_dir
        .or_else(|| (!cfg.failed_dir.is_empty()).then(|| PathBuf::from(&cfg.failed_dir)))
        .unwrap_or_else(|| out_root.join("解压失败"));

    // 目录目标作为分组根；文件目标按根目录散包处理（与 Python 版一致：取首个文件的父目录）
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    for s in &args.targets {
        if s.is_dir() {
            roots.push(s.clone());
        } else if s.is_file() {
            files.push(s.clone());
        } else {
            println!("[警告] 路径不存在：{}", s.display());
        }
    }
    if let Some(first) = files.first() {
        roots.push(first.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from(".")));
    }

    println!("输出目录：{}", out_root.display());
    println!("失败目录：{}", failed_root.display());
    if !cfg.passwords.is_empty() || !args.password.is_empty() {
        println!("密码库：{} 个（规则：{}）",
                 cfg.passwords.len() + args.password.len(), cfg.password_strategy);
    }
    println!();

    let mut passwords = cfg.passwords.clone();
    for pw in &args.password {
        if !passwords.contains(pw) {
            passwords.push(pw.clone());
        }
    }

    let summary: Summary = unzip_core::run(
        &roots,
        &out_root,
        Some(&failed_root),
        cfg,
        passwords,
        &PrintCallback,
        args.dry_run,
        recursive,
    );

    println!("{}", "-".repeat(50));
    println!("完成：成功 {}，失败 {}，跳过 {}，警告 {}",
             summary.ok.len(), summary.failed.len(),
             summary.skipped.len(), summary.warns.len());
    if !summary.failed.is_empty() {
        println!("失败明细：");
        for (name, reason) in &summary.failed {
            println!("  - {name}：{reason}");
        }
    }
    if summary.failed.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
