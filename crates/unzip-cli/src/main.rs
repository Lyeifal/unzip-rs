//! unzip-cli — 自动解压工具命令行入口（移植自 D:\Code\unzip\auto_unzip.py，行为对齐）。

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use unzip_core::config::{self, Config};
use unzip_core::types::{LogLevel, RunCallback, Summary};

/// 自动解压工具：识别伪装/改名压缩包，分卷自动归集，lz4 解码，密码自动尝试，按最外层目录分组
#[derive(Parser)]
#[command(version, about)]
struct Args {
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

struct PrintCallback;

impl RunCallback for PrintCallback {
    fn on_log(&self, msg: &str, _level: LogLevel) {
        println!("{msg}");
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
