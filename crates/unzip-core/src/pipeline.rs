//! 解包与主流程：unwrap_folder 递归、lz4 链、extract_package、run()。
//! （移植自 unzip_core.py 783-1076 行）

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::assemble::{assemble_volumes, rescue_renamed_first};
use crate::config::{promote_password, Config};
use crate::extract::Extractor;
use crate::scan::{build_packages, scan_sources};
use crate::sniff::classify;
use crate::types::{ArchiveKind, LogLevel, Package, PkgKind, RunCallback, ScanResult, Summary};

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 类似 Python Path.stem：去掉最后一个后缀；点开头的名字整体视为 stem。
fn stem_of(p: &Path) -> String {
    let name = file_name(p);
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name,
    }
}

/// 类似 Python Path.suffix。
fn suffix_of(p: &Path) -> String {
    let name = file_name(p);
    match name.rfind('.') {
        Some(i) if i > 0 => name[i..].to_string(),
        _ => String::new(),
    }
}

/// 重名递增：base 不存在直接用；否则 "name (2)"…"name (999)"（对齐 Python unique_dir）。
fn unique_dir(base: &Path) -> PathBuf {
    if !base.exists() {
        return base.to_path_buf();
    }
    let empty_dir = base.is_dir() && fs::read_dir(base).map(|mut d| d.next().is_none()).unwrap_or(false);
    if empty_dir {
        return base.to_path_buf();
    }
    let dir_name = file_name(base);
    for i in 2..1000 {
        let cand = base.with_file_name(format!("{dir_name} ({i})"));
        if !cand.exists() {
            return cand;
        }
    }
    base.to_path_buf()
}

/// 同目录内重名递增的目标文件：dir/name 已存在则 "stem (i)suffix"。
fn unique_target(dir: &Path, src: &Path) -> PathBuf {
    let target = dir.join(file_name(src));
    if !target.exists() {
        return target;
    }
    let stem = stem_of(src);
    let suffix = suffix_of(src);
    for i in 2..1000 {
        let cand = dir.join(format!("{stem} ({i}){suffix}"));
        if !cand.exists() {
            return cand;
        }
    }
    target
}

/// 跨盘 rename 失败回退 copy+remove（对齐 shutil.move 的语义子集）。
fn move_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(src, dst)?;
            fs::remove_file(src)
        }
    }
}

fn max_depth_of(cfg: &Config) -> u32 {
    if cfg.max_depth == 0 {
        5
    } else {
        cfg.max_depth
    }
}

/// 按密码规则生成该包的尝试顺序：匹配规则（按配置顺序）的优先密码在前，
/// 全局密码库剩余项在后（去重保序）。
pub fn order_passwords(pkg: &Package, cfg: &Config) -> Vec<String> {
    let mut ordered: Vec<String> = Vec::new();
    let name_l = file_name(&pkg.first).to_lowercase();
    let path_l = pkg.first.to_string_lossy().to_lowercase();
    for rule in &cfg.password_rules {
        let sfx = rule.suffix.trim().to_lowercase();
        let kw = rule.keyword.trim().to_lowercase();
        let mut hit = true;
        if !sfx.is_empty() {
            hit = name_l.ends_with(&sfx);
        }
        if hit && !kw.is_empty() {
            hit = name_l.contains(&kw) || path_l.contains(&kw);
        }
        if hit {
            for pw in &rule.passwords {
                if !ordered.contains(pw) {
                    ordered.push(pw.clone());
                }
            }
        }
    }
    for pw in &cfg.passwords {
        if !ordered.contains(pw) {
            ordered.push(pw.clone());
        }
    }
    ordered
}

/// 包上下文：run 的 passwords 参数、输出/失败目录、回调。
struct Ctx<'a> {
    passwords: &'a [String],
    out_root: PathBuf,
    failed_root: Option<PathBuf>,
    cb: &'a dyn RunCallback,
}

/// 文件夹里只有压缩包（无实质内容/子目录/产物）→ 继续解到同一夹，直到露出内容。
fn unwrap_folder(
    dest: &Path,
    ex: &Extractor,
    cfg: &Config,
    passwords: &[String],
    ctx: &Ctx,
    depth: u32,
    max_depth: u32,
) {
    if depth >= max_depth {
        return;
    }
    let Ok(entries) = fs::read_dir(dest) else {
        return;
    };
    let entries: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    let dirs: Vec<&PathBuf> = entries.iter().filter(|p| p.is_dir()).collect();
    let files: Vec<&PathBuf> = entries.iter().filter(|p| p.is_file()).collect();
    let exts: Vec<String> = cfg.product_exts.iter().map(|e| e.to_lowercase()).collect();
    let mut archives: Vec<(PathBuf, PkgKind)> = Vec::new();
    for f in &files {
        if let Some(kind) = classify(f, &exts) {
            if kind.is_sevenz_kind() || kind == PkgKind::Lz4 {
                archives.push(((*f).clone(), kind));
            }
        }
    }
    if !dirs.is_empty() || archives.len() != files.len() || archives.is_empty() {
        return;
    }
    ctx.cb.on_log(
        &format!("[解包] {} 内仍是压缩包，继续解包…", file_name(dest)),
        LogLevel::Info,
    );
    for (f, kind) in &archives {
        let err = match kind {
            // Python 先走 extract_archive（7z 必失败）再进 _unwrap_lz4，结果等价。
            PkgKind::Lz4 => unwrap_lz4(f, dest, ex, cfg, passwords),
            PkgKind::Archive(ak) => match ex.extract_archive(f, dest, passwords, *ak) {
                Ok(_) => None,
                Err(e) => {
                    ctx.cb.on_log(
                        &format!("[警告] 内层 {} 解压失败：{e}", file_name(f)),
                        LogLevel::Warn,
                    );
                    Some(e)
                }
            },
            PkgKind::Product => Some(String::new()),
        };
        if err.is_none() {
            let _ = fs::remove_file(f);
        }
    }
    unwrap_folder(dest, ex, cfg, passwords, ctx, depth + 1, max_depth);
}

/// 解一个内层 lz4：解码到临时文件；内层是压缩档解到 dest/{stem}/；
/// 仍是 lz4 → 套娃过深；产物/裸数据 → 落位 {stem}.bin。
fn unwrap_lz4(
    f: &Path,
    dest: &Path,
    ex: &Extractor,
    cfg: &Config,
    passwords: &[String],
) -> Option<String> {
    let tmp = tempfile::Builder::new().prefix(".lz4_").tempdir().ok()?;
    let out = tmp.path().join("decoded.bin");
    if let Some(err) = ex.decode_lz4(f, &out) {
        return Some(err);
    }
    let exts: Vec<String> = cfg.product_exts.iter().map(|e| e.to_lowercase()).collect();
    match classify(&out, &exts) {
        Some(PkgKind::Archive(ak)) => {
            let sub = dest.join(stem_of(f));
            if let Err(e) = fs::create_dir(&sub) {
                return Some(e.to_string());
            }
            ex.extract_archive(&out, &sub, passwords, ak).err()
        }
        Some(PkgKind::Lz4) => Some("lz4 套娃过深，已跳过".to_string()),
        _ => {
            let target = dest.join(format!("{}.bin", stem_of(f)));
            move_file(&out, &target).err().map(|e| e.to_string())
        }
    }
}

/// lz4 解码链：套娃 lz4 继续解；内层压缩包解到 dest；内层 apk 产物/裸数据落位。
fn lz4_pipeline(
    src: &Path,
    dest: &Path,
    ex: &Extractor,
    cfg: &mut Config,
    ctx: &Ctx,
    tmp_dirs: &mut Vec<TempDir>,
    pw_list: &[String],
) -> Option<String> {
    let tmp = match tempfile::Builder::new().prefix(".lz4_").tempdir() {
        Ok(t) => t,
        Err(e) => return Some(e.to_string()),
    };
    let tmp_path = tmp.path().to_path_buf();
    tmp_dirs.push(tmp);
    let exts: Vec<String> = cfg.product_exts.iter().map(|e| e.to_lowercase()).collect();
    let mut cur = src.to_path_buf();
    let max_depth = max_depth_of(cfg);
    for depth in 0..max_depth {
        let out = tmp_path.join(format!("decoded_{depth}.bin"));
        if let Some(err) = ex.decode_lz4(&cur, &out) {
            return Some(err);
        }
        match classify(&out, &exts) {
            Some(PkgKind::Lz4) => {
                cur = out;
                continue;
            }
            Some(PkgKind::Archive(ak)) => {
                return match ex.extract_archive(&out, dest, pw_list, ak) {
                    Ok(pw) => {
                        if pw.is_some() && cfg.password_strategy == "recent_first" {
                            promote_password(cfg, pw.as_deref().unwrap());
                        }
                        None
                    }
                    Err(e2) => Some(format!("内层压缩包解压失败：{e2}")),
                };
            }
            Some(PkgKind::Product) => {
                let name = format!("{}.apk", stem_of(src));
                let target = unique_target(dest, &dest.join(name));
                return match fs::copy(&out, &target) {
                    Ok(_) => {
                        ctx.cb.on_log(
                            &format!(
                                "[成功] {}  —  解码出产物 {}",
                                file_name(src),
                                file_name(&target)
                            ),
                            LogLevel::Ok,
                        );
                        None
                    }
                    Err(e) => Some(format!("解码产物落位失败：{e}")),
                };
            }
            None => {
                let target = dest.join(format!("{}.bin", stem_of(src)));
                return match move_file(&out, &target) {
                    Ok(()) => None,
                    Err(e) => Some(format!("解码结果移动失败：{e}")),
                };
            }
        }
    }
    Some("lz4 套娃超过深度上限".to_string())
}

/// 失败包（首卷 + 全部分卷）移入失败目录，重名递增；跨盘 rename 失败回退 copy+remove。
fn move_to_failed(pkg: &Package, ctx: &Ctx, summary: &mut Summary) {
    let Some(failed_root) = &ctx.failed_root else {
        return;
    };
    if fs::create_dir_all(failed_root).is_err() {
        return;
    }
    let mut files = vec![pkg.first.clone()];
    files.extend(pkg.volumes.values().cloned());
    for f in files {
        let target = unique_target(failed_root, &f);
        if let Err(e) = move_file(&f, &target) {
            summary
                .warns
                .push(format!("移入失败目录失败 {}：{e}", file_name(&f)));
        }
    }
}

/// 处理单个包：产物落位 / lz4 链 / 组装分卷 → 解压 → 解包 → 密码置顶。返回是否成功。
#[allow(clippy::too_many_arguments)]
fn extract_package(
    ex: &Extractor,
    pkg: &Package,
    cfg: &mut Config,
    ctx: &Ctx,
    scan: &mut ScanResult,
    summary: &mut Summary,
    dry_run: bool,
    preassembled: Option<(PathBuf, Option<TempDir>, Option<String>)>,
) -> bool {
    let dest_base = pkg
        .dest_base
        .clone()
        .unwrap_or_else(|| ctx.out_root.join(&pkg.name));
    let dest = unique_dir(&dest_base);

    if pkg.kind == PkgKind::Product {
        if dry_run {
            ctx.cb.on_log(
                &format!("[预演] 产物 {} → {}", file_name(&pkg.first), dest.display()),
                LogLevel::Info,
            );
            return true;
        }
        if let Err(e) = fs::create_dir_all(&dest) {
            summary
                .failed
                .push((pkg.name.clone(), format!("产物复制失败：{e}")));
            ctx.cb.on_log(
                &format!("[失败] {}  —  产物复制失败：{e}", file_name(&pkg.first)),
                LogLevel::Error,
            );
            return false;
        }
        let target = unique_target(&dest, &pkg.first);
        match fs::copy(&pkg.first, &target) {
            Ok(_) => {
                summary
                    .ok
                    .push((pkg.name.clone(), format!("产物 → {}", file_name(&dest))));
                ctx.cb.on_log(
                    &format!(
                        "[成功] {}  —  产物已放入 {}",
                        file_name(&pkg.first),
                        file_name(&dest)
                    ),
                    LogLevel::Ok,
                );
                true
            }
            Err(e) => {
                summary
                    .failed
                    .push((pkg.name.clone(), format!("产物复制失败：{e}")));
                ctx.cb.on_log(
                    &format!("[失败] {}  —  产物复制失败：{e}", file_name(&pkg.first)),
                    LogLevel::Error,
                );
                false
            }
        }
    } else {
        if dry_run {
            let vol = if pkg.volumes.is_empty() {
                String::new()
            } else {
                format!("（含分卷 {} 个）", pkg.volumes.len())
            };
            ctx.cb.on_log(
                &format!(
                    "[预演] {}{vol} 类型 {} → {}",
                    file_name(&pkg.first),
                    pkg.kind.as_str(),
                    dest.display()
                ),
                LogLevel::Info,
            );
            return true;
        }

        if let Err(e) = fs::create_dir_all(&dest) {
            summary.failed.push((pkg.name.clone(), e.to_string()));
            ctx.cb.on_log(
                &format!("[失败] {}  —  {e}", file_name(&pkg.first)),
                LogLevel::Error,
            );
            return false;
        }
        let mut tmp_dirs: Vec<TempDir> = Vec::new();

        let mut first = pkg.first.clone();
        let mut err: Option<String> = None;
        let mut pw: Option<String> = None;
        let pw_list = order_passwords(pkg, cfg);
        if pkg.kind == PkgKind::Lz4 {
            err = lz4_pipeline(&pkg.first, &dest, ex, cfg, ctx, &mut tmp_dirs, &pw_list);
        } else {
            if let Some((pfirst, ptmp, perr)) = preassembled {
                if let Some(t) = ptmp {
                    tmp_dirs.push(t);
                }
                first = pfirst;
                err = perr;
            } else if pkg.vol_family.is_some() {
                match tempfile::Builder::new()
                    .prefix(".assemble_")
                    .tempdir_in(&ctx.out_root)
                {
                    Ok(t) => {
                        let tpath = t.path().to_path_buf();
                        tmp_dirs.push(t);
                        let (f, e) = assemble_volumes(ex, pkg, scan, &pw_list, &tpath, ctx.cb);
                        first = f;
                        err = e;
                    }
                    Err(e) => err = Some(e.to_string()),
                }
            }
            if err.is_none() {
                let kind = match pkg.kind {
                    PkgKind::Archive(k) => k,
                    _ => ArchiveKind::Zip,
                };
                match ex.extract_archive(&first, &dest, &pw_list, kind) {
                    Ok(p) => pw = p,
                    Err(e) => err = Some(e),
                }
            }
            if err.is_some() && pkg.vol_family.is_none() {
                if let Some((first2, tmp2)) =
                    rescue_renamed_first(ex, pkg, scan, &pw_list, &ctx.out_root, ctx.cb)
                {
                    first = first2;
                    tmp_dirs.push(tmp2);
                    match ex.extract_archive(&first, &dest, &pw_list, ArchiveKind::Rar) {
                        Ok(p) => {
                            pw = p;
                            err = None;
                        }
                        Err(e) => err = Some(e),
                    }
                }
            }
        }
        // tmp_dirs 在函数出口统一 drop 清理（含失败路径）
        if let Some(err) = err {
            let _ = fs::remove_dir_all(&dest);
            move_to_failed(pkg, ctx, summary);
            summary.failed.push((pkg.name.clone(), err.clone()));
            ctx.cb.on_log(
                &format!(
                    "[失败] {}  —  {err}（源文件已移入失败目录）",
                    file_name(&pkg.first)
                ),
                LogLevel::Error,
            );
            return false;
        }

        unwrap_folder(
            &dest,
            ex,
            cfg,
            ctx.passwords,
            ctx,
            0,
            max_depth_of(cfg),
        );
        if pw.is_some() && cfg.password_strategy == "recent_first" {
            promote_password(cfg, pw.as_deref().unwrap());
        }
        let note = pw
            .as_deref()
            .map(|p| format!("密码：{p}"))
            .unwrap_or_default();
        summary.ok.push((pkg.name.clone(), note.clone()));
        let line = format!(
            "[成功] {}  —  → {} {note}",
            file_name(&pkg.first),
            file_name(&dest)
        );
        ctx.cb.on_log(line.trim_end(), LogLevel::Ok);
        true
    }
}

fn is_adopted(pkg: &Package, scan: &ScanResult) -> bool {
    if scan.adopted.contains(&pkg.first) {
        return true;
    }
    pkg.volumes.values().any(|v| scan.adopted.contains(v))
}

/// 与 pkg 同组且尚未被领养（仍算独立包）的数量。
fn alive_count(pkgs: &[Package], pkg: &Package, scan: &ScanResult) -> usize {
    let key = pkg.group_key();
    pkgs.iter()
        .filter(|q| q.group_key() == key)
        .filter(|q| !is_adopted(q, scan))
        .count()
}

/// 扫描 → 建包 → 预组装（领养改名卷）→ 分组 → 逐包解压。返回 Summary。
#[allow(clippy::too_many_arguments)]
pub fn run(
    source_roots: &[PathBuf],
    out_root: &Path,
    failed_root: Option<&Path>,
    mut cfg: Config,
    passwords: Vec<String>,
    cb: &dyn RunCallback,
    dry_run: bool,
    recursive: bool,
) -> Summary {
    let _ = fs::create_dir_all(out_root);
    let ex = Extractor::new(&cfg, None);
    let mut summary = Summary::default();
    let mut scan = scan_sources(source_roots, recursive, &cfg, Some(out_root), failed_root);
    for (f, reason) in &scan.skips {
        summary.skipped.push((file_name(f), reason.clone()));
        cb.on_log(
            &format!("[跳过] {}  —  {reason}", file_name(f)),
            LogLevel::Skip,
        );
    }
    let mut pkgs = build_packages(&mut scan, source_roots);

    // 预组装：卷族包先归集/领养孤儿（其结果决定分组存活计数与目标目录）
    let mut pre: HashMap<usize, (PathBuf, Option<TempDir>, Option<String>)> = HashMap::new();
    if !dry_run {
        for (i, pkg) in pkgs.iter().enumerate() {
            if cb.should_cancel() {
                break;
            }
            if is_adopted(pkg, &scan) || pkg.vol_family.is_none() {
                continue;
            }
            let Ok(tmp) = tempfile::Builder::new()
                .prefix(".assemble_")
                .tempdir_in(out_root)
            else {
                continue;
            };
            let tpath = tmp.path().to_path_buf();
            let pw_list = order_passwords(pkg, &cfg);
            let (first, err) = assemble_volumes(&ex, pkg, &mut scan, &pw_list, &tpath, cb);
            pre.insert(i, (first, Some(tmp), err));
        }
    }

    cb.on_log(
        &format!(
            "扫描完成：{} 个压缩包/产物，孤儿候选 {} 个",
            pkgs.len(),
            scan.rar_orphans.len() + scan.orphans.len()
        ),
        LogLevel::Info,
    );

    let total = pkgs.len();
    for i in 0..total {
        if cb.should_cancel() {
            summary.warns.push("用户取消".to_string());
            cb.on_log("[警告] 已取消", LogLevel::Warn);
            break;
        }
        if is_adopted(&pkgs[i], &scan) {
            cb.on_log(
                &format!(
                    "[跳过] {}  —  已作为分卷并入其他压缩包",
                    file_name(&pkgs[i].first)
                ),
                LogLevel::Skip,
            );
            continue;
        }
        let alive = alive_count(&pkgs, &pkgs[i], &scan);
        let dest_base = if alive > 1 {
            out_root.join(pkgs[i].group_key()).join(&pkgs[i].name)
        } else {
            out_root.join(pkgs[i].group_key())
        };
        pkgs[i].dest_base = Some(dest_base);
        let name = file_name(&pkgs[i].first);
        cb.on_progress(i, total, &name);
        if !dry_run && pkgs[i].kind != PkgKind::Product {
            cb.on_log(
                &format!("[处理] ({}/{total}) {name}", i + 1),
                LogLevel::Info,
            );
        }
        let ctx = Ctx {
            passwords: &passwords,
            out_root: out_root.to_path_buf(),
            failed_root: failed_root.map(|p| p.to_path_buf()),
            cb,
        };
        extract_package(
            &ex,
            &pkgs[i],
            &mut cfg,
            &ctx,
            &mut scan,
            &mut summary,
            dry_run,
            pre.remove(&i),
        );
        cb.on_progress(i + 1, total, &name);
    }

    for w in &scan.warns {
        summary.warns.push(w.clone());
        cb.on_log(&format!("[警告] {w}"), LogLevel::Warn);
    }
    let leftover: Vec<&PathBuf> = scan.orphans.iter().filter(|p| p.exists()).collect();
    if !leftover.is_empty() {
        let names: Vec<String> = leftover.iter().take(10).map(|p| file_name(p)).collect();
        let mut msg = format!(
            "{} 个文件无法识别（可能是不支持的压缩格式或已损坏）：{}",
            leftover.len(),
            names.join("、")
        );
        if leftover.len() > 10 {
            msg.push_str(" 等");
        }
        summary.warns.push(msg.clone());
        cb.on_log(&format!("[警告] {msg}"), LogLevel::Warn);
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use tempfile::{tempdir, TempDir};

    use crate::config::PasswordRule;

    const SEVENZ: &str = r"C:\Program Files\7-Zip\7z.exe";
    const RAR: &str = r"C:\Program Files\WinRAR\Rar.exe";
    const UNRAR: &str = r"C:\Program Files\WinRAR\UnRAR.exe";
    const LZ4: &str = r"D:\APP\解压工具\解压工具\lz4.exe";

    /// 缺工具则打印 skip 并返回 false（用例直接 return，不算失败）。
    fn have_tools(paths: &[&str]) -> bool {
        match paths.iter().find(|p| !Path::new(p).is_file()) {
            None => true,
            Some(m) => {
                eprintln!("SKIP：本机缺少外部工具 {m}");
                false
            }
        }
    }

    fn sh(cmd: &mut Command) {
        let st = cmd.status().unwrap();
        assert!(st.success(), "命令失败：{cmd:?}");
    }

    /// 伪随机填充（全零会被 rar 压成单卷，造不出分卷）。
    fn random_bytes(n: usize, seed: u64) -> Vec<u8> {
        let mut x = seed | 1;
        let mut v: Vec<u8> = (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x & 0xff) as u8
            })
            .collect();
        v[0] = 0x01; // 避开一切压缩档/媒体魔数首字节
        v
    }

    fn tool_cfg(passwords: &[&str]) -> Config {
        Config {
            seven_zip: SEVENZ.to_string(),
            unrar: UNRAR.to_string(),
            lz4: LZ4.to_string(),
            passwords: passwords.iter().map(|s| s.to_string()).collect(),
            // list_order：避免 promote_password 在测试里回写 exe 旁 config.json
            password_strategy: "list_order".to_string(),
            max_depth: 5,
            ..Default::default()
        }
    }

    fn three_dirs(tmp: &TempDir) -> (PathBuf, PathBuf, PathBuf) {
        let src = tmp.path().join("src");
        let out = tmp.path().join("out");
        let failed = tmp.path().join("failed");
        fs::create_dir_all(&src).unwrap();
        (src, out, failed)
    }

    /// 源目录文件清单（相对路径 + 大小），防误删回归。
    fn snapshot(dir: &Path) -> Vec<(PathBuf, u64)> {
        let mut out: Vec<(PathBuf, u64)> = walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                (
                    e.path().strip_prefix(dir).unwrap().to_path_buf(),
                    e.metadata().map(|m| m.len()).unwrap_or(0),
                )
            })
            .collect();
        out.sort();
        out
    }

    struct Collect {
        logs: Mutex<Vec<String>>,
    }
    impl Collect {
        fn new() -> Self {
            Collect {
                logs: Mutex::new(Vec::new()),
            }
        }
        fn contains(&self, needle: &str) -> bool {
            self.logs.lock().unwrap().iter().any(|m| m.contains(needle))
        }
        fn all(&self) -> Vec<String> {
            self.logs.lock().unwrap().clone()
        }
    }
    impl RunCallback for Collect {
        fn on_log(&self, msg: &str, _level: LogLevel) {
            self.logs.lock().unwrap().push(msg.to_string());
        }
    }

    // ---------- 纯函数单测 ----------

    #[test]
    fn unique_dir_appends_counter() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("d");
        assert_eq!(unique_dir(&base), base); // 不存在 → 直接用
        fs::create_dir(&base).unwrap();
        assert_eq!(unique_dir(&base), base); // 空目录 → 直接用
        fs::write(base.join("f"), b"x").unwrap();
        assert_eq!(unique_dir(&base), tmp.path().join("d (2)"));
        fs::create_dir(tmp.path().join("d (2)")).unwrap();
        assert_eq!(unique_dir(&base), tmp.path().join("d (3)"));
    }

    #[test]
    fn order_passwords_rules_first_then_library() {
        let tmp = tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.passwords = vec!["aaa".into(), "bbb".into(), "ccc".into()];
        cfg.password_rules = vec![
            PasswordRule {
                suffix: ".lz4".into(),
                keyword: String::new(),
                passwords: vec!["bbb".into(), "zzz".into()],
            },
            PasswordRule {
                suffix: String::new(),
                keyword: "game".into(),
                passwords: vec!["ccc".into()],
            },
        ];
        let mk_pkg = |name: &str| {
            let first = tmp.path().join(name);
            fs::write(&first, b"").unwrap();
            Package {
                name: name.to_string(),
                first,
                kind: PkgKind::Archive(ArchiveKind::Zip),
                rel: PathBuf::from(name),
                root: tmp.path().to_path_buf(),
                vol_family: None,
                vol_stem: String::new(),
                volumes: BTreeMap::new(),
                dest_base: None,
            }
        };
        // 只命中 keyword 规则：ccc 前置，其余按密码库顺序
        assert_eq!(order_passwords(&mk_pkg("game.lzh"), &cfg), ["ccc", "aaa", "bbb"]);
        // 后缀 + keyword 都命中：规则按配置顺序前置，规则内按列表顺序，去重
        assert_eq!(
            order_passwords(&mk_pkg("GAME.LZ4"), &cfg),
            ["bbb", "zzz", "ccc", "aaa"]
        );
        // 都不命中：纯密码库
        assert_eq!(order_passwords(&mk_pkg("x.7z"), &cfg), ["aaa", "bbb", "ccc"]);
    }

    // ---------- 集成测试（真实外部工具，全部只写 tempfile 目录） ----------

    /// 散卷跨目录 + zzz.bin 改名卷领养 → 解出完整 big.bin；源目录清单不变；临时目录清理干净。
    #[test]
    fn scattered_volumes_across_dirs_and_renamed_adoption() {
        if !have_tools(&[SEVENZ, RAR, UNRAR]) {
            return;
        }
        let tmp = tempdir().unwrap();
        let (src, out, failed) = three_dirs(&tmp);
        let vol = src.join("游戏A/vol");
        let other = src.join("游戏A/other");
        fs::create_dir_all(&vol).unwrap();
        fs::create_dir_all(&other).unwrap();

        let work = tempdir().unwrap();
        fs::write(work.path().join("big.bin"), random_bytes(60 * 1024, 42)).unwrap();
        sh(Command::new(RAR)
            .args(["a", "-v30k", "-ep1"])
            .arg("m.rar")
            .arg("big.bin")
            .current_dir(work.path()));
        fs::rename(work.path().join("m.part1.rar"), vol.join("m.part1.rar")).unwrap();
        fs::rename(work.path().join("m.part2.rar"), other.join("m.part2.rar")).unwrap();
        fs::rename(work.path().join("m.part3.rar"), other.join("zzz.bin")).unwrap();

        let before = snapshot(&src);
        let cb = Collect::new();
        // 注意：unrar 探针循环按密码列表驱动（对齐 Python：空列表时探针不执行），
        // 因此必须像真实使用一样给密码库（内容随意，未加密卷不受密码影响）。
        let summary = run(
            &[src.clone()],
            &out,
            Some(&failed),
            tool_cfg(&["testpw"]),
            vec![],
            &cb,
            false,
            true,
        );

        assert_eq!(
            out.join("游戏A/big.bin").metadata().unwrap().len(),
            60 * 1024,
            "散卷+改名卷领养应解出完整 big.bin，日志：{:?}",
            cb.all()
        );
        assert!(
            cb.contains("zzz.bin 即 m.part3.rar"),
            "领养日志缺失，日志：{:?}",
            cb.all()
        );
        assert!(
            cb.contains("[跳过] zzz.bin  —  已作为分卷并入其他压缩包"),
            "被领养包的跳过日志缺失，日志：{:?}",
            cb.all()
        );
        assert!(summary.failed.is_empty(), "失败列表应为空：{:?}", summary.failed);
        // 源目录文件清单不变（领养用 link/copy，不移动源文件）
        assert_eq!(snapshot(&src), before, "源目录文件清单被改变");
        // .assemble_/.rescue_ 临时目录必须清理干净
        let stray: Vec<String> = fs::read_dir(&out)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('.'))
            .collect();
        assert!(stray.is_empty(), "输出目录残留临时目录：{stray:?}");
        if failed.exists() {
            assert_eq!(fs::read_dir(&failed).unwrap().count(), 0);
        }
    }

    /// 加密 zip（7z 造，密码 123456）→ 密码库命中解出。
    #[test]
    fn encrypted_zip_password_from_library() {
        if !have_tools(&[SEVENZ]) {
            return;
        }
        let tmp = tempdir().unwrap();
        let (src, out, failed) = three_dirs(&tmp);
        let work = tempdir().unwrap();
        fs
::write(work.path().join("enc.txt"), "机密内容").unwrap();
        sh(Command::new(SEVENZ)
            .args(["a", "-tzip", "-p123456"])
            .arg(&src.join("enc.zip"))
            .arg("enc.txt")
            .current_dir(work.path()));

        let before = snapshot(&src);
        let cb = Collect::new();
        let summary = run(
            &[src.clone()],
            &out,
            Some(&failed),
            tool_cfg(&["wrongpw", "123456"]),
            vec![],
            &cb,
            false,
            true,
        );
        assert_eq!(
            fs::read_to_string(out.join("enc/enc.txt")).unwrap(),
            "机密内容",
            "密码库命中应解出，日志：{:?}",
            cb.all()
        );
        assert!(summary.failed.is_empty());
        assert_eq!(snapshot(&src), before, "源目录文件清单被改变");
    }

    /// apk 产物只落位、不解压。
    #[test]
    fn apk_product_placed_not_extracted() {
        if !have_tools(&[SEVENZ]) {
            return;
        }
        let tmp = tempdir().unwrap();
        let (src, out, failed) = three_dirs(&tmp);
        let work = tempdir().unwrap();
        fs::create_dir(work.path().join("apk")).unwrap();
        fs::write(work.path().join("apk/AndroidManifest.xml"), "manifest").unwrap();
        sh(Command::new(SEVENZ)
            .args(["a", "-tzip"])
            .arg("game.apk")
            .arg("apk")
            .current_dir(work.path()));
        fs::copy(work.path().join("game.apk"), src.join("game.apk")).unwrap();

        let before = snapshot(&src);
        run(
            &[src.clone()],
            &out,
            Some(&failed),
            tool_cfg(&[]),
            vec![],
            &Collect::new(),
            false,
            true,
        );
        let game_dir = out.join("game");
        assert!(game_dir.join("game.apk").is_file(), "产物应原样落位");
        let n = fs::read_dir(&game_dir).unwrap().count();
        assert_eq!(n, 1, "产物不应被解包（目录里只有 apk 自身）");
        assert_eq!(snapshot(&src), before, "源目录文件清单被改变");
    }

    /// 截断坏 zip → 移入失败目录且源位置消失。
    #[test]
    fn truncated_zip_moves_to_failed() {
        if !have_tools(&[SEVENZ]) {
            return;
        }
        let tmp = tempdir().unwrap();
        let (src, out, failed) = three_dirs(&tmp);
        fs::write(src.join("bad.zip"), b"this is not a zip at all, just garbage").unwrap();

        let cb = Collect::new();
        let summary = run(
            &[src.clone()],
            &out,
            Some(&failed),
            tool_cfg(&[]),
            vec![],
            &cb,
            false,
            true,
        );
        assert!(failed.join("bad.zip").is_file(), "坏包应移入失败目录");
        assert!(!src.join("bad.zip").exists(), "坏包不应留在源目录");
        assert!(!out.join("bad").exists(), "失败包的目标目录应被删除");
        assert_eq!(summary.failed.len(), 1, "summary.failed={:?}", summary.failed);
        assert!(
            cb.contains("[失败] bad.zip"),
            "失败日志缺失，日志：{:?}",
            cb.all()
        );
    }

    /// lz4 套 zip 链 → 解出内容。
    #[test]
    fn lz4_wrapped_zip_chain() {
        if !have_tools(&[SEVENZ, LZ4]) {
            return;
        }
        let tmp = tempdir().unwrap();
        let (src, out, failed) = three_dirs(&tmp);
        let work = tempdir().unwrap();
        fs::write(work.path().join("payload.txt"), "内层内容").unwrap();
        sh(Command::new(SEVENZ)
            .args(["a", "-tzip"])
            .arg("content.zip")
            .arg("payload.txt")
            .current_dir(work.path()));
        sh(Command::new(LZ4)
            .args(["-f"])
            .arg("content.zip")
            .current_dir(work.path())); // → content.zip.lz4
        fs::copy(
            work.path().join("content.zip.lz4"),
            src.join("content.lz4"),
        )
        .unwrap();

        let cb = Collect::new();
        run(
            &[src.clone()],
            &out,
            Some(&failed),
            tool_cfg(&[]),
            vec![],
            &cb,
            false,
            true,
        );
        assert_eq!(
            fs::read_to_string(out.join("content/payload.txt")).unwrap(),
            "内层内容",
            "lz4 套 zip 应解出内层文件，日志：{:?}",
            cb.all()
        );
    }

    /// legacy lz4（lz4 -l）伪装成 game.mp4 → 魔数识别并解出。
    #[test]
    fn legacy_lz4_disguised_as_mp4() {
        if !have_tools(&[LZ4]) {
            return;
        }
        let tmp = tempdir().unwrap();
        let (src, out, failed) = three_dirs(&tmp);
        let work = tempdir().unwrap();
        let raw = random_bytes(4096, 7);
        fs::write(work.path().join("raw.bin"), &raw).unwrap();
        sh(Command::new(LZ4)
            .args(["-f", "-l"]) // legacy 帧格式
            .arg("raw.bin")
            .current_dir(work.path()));
        fs::copy(work.path().join("raw.bin.lz4"), src.join("game.mp4")).unwrap();

        let cb = Collect::new();
        run(
            &[src.clone()],
            &out,
            Some(&failed),
            tool_cfg(&[]),
            vec![],
            &cb,
            false,
            true,
        );
        let decoded = out.join("game/game.bin");
        assert_eq!(
            fs::read(&decoded).unwrap(),
            raw,
            "legacy lz4 应被识别并解码，日志：{:?}",
            cb.all()
        );
    }

    /// 未知二进制 → 结尾逐字告警；.baiduyun.p.downloading 跳过；无主分卷警告。
    #[test]
    fn unknown_binary_warns_downloading_skipped_and_ownerless_volumes() {
        // 本用例无任何可解压包，不需要外部工具
        let tmp = tempdir().unwrap();
        let (src, out, failed) = three_dirs(&tmp);
        fs::write(src.join("mystery.bin"), b"\x00\x01\x02\x03").unwrap();
        fs::write(src.join("clip.baiduyun.p.downloading"), b"partial").unwrap();
        fs::write(src.join("a.7z.002"), b"7z\xbc\xaf\x27\x1c\x00\x01\x02\x03").unwrap();
        fs::write(src.join("a.7z.003"), b"7z\xbc\xaf\x27\x1c\x00\x01\x02\x03").unwrap();

        let cb = Collect::new();
        let summary = run(
            &[src.clone()],
            &out,
            Some(&failed),
            Config::default(),
            vec![],
            &cb,
            false,
            true,
        );
        assert!(
            summary
                .warns
                .iter()
                .any(|w| w == "1 个文件无法识别（可能是不支持的压缩格式或已损坏）：mystery.bin"),
            "逐字告警缺失，warns={:?}",
            summary.warns
        );
        assert!(
            summary
                .skipped
                .iter()
                .any(|(n, r)| n == "clip.baiduyun.p.downloading" && r == "未下载完成的文件"),
            "skipped={:?}",
            summary.skipped
        );
        assert!(
            summary
                .warns
                .iter()
                .any(|w| w == "无主分卷（找不到首卷，已忽略）：a.7z.002, a.7z.003"),
            "无主分卷告警缺失，warns={:?}",
            summary.warns
        );
    }

    /// dry_run：只打 [预演]，不产生任何文件。
    #[test]
    fn dry_run_creates_nothing() {
        if !have_tools(&[SEVENZ]) {
            return;
        }
        let tmp = tempdir().unwrap();
        let (src, out, failed) = three_dirs(&tmp);
        let work = tempdir().unwrap();
        fs::write(work.path().join("hello.txt"), "hi").unwrap();
        sh(Command::new(SEVENZ)
            .args(["a", "-tzip"])
            .arg(&src.join("plain.zip"))
            .arg("hello.txt")
            .current_dir(work.path()));
        fs::copy(src.join("plain.zip"), src.join("app.apk")).unwrap(); // 产物副本

        let before = snapshot(&src);
        let cb = Collect::new();
        let summary = run(
            &[src.clone()],
            &out,
            Some(&failed),
            tool_cfg(&[]),
            vec![],
            &cb,
            true,
            true,
        );
        assert!(
            cb.contains("[预演] plain.zip 类型 zip → "),
            "预演日志缺失，日志：{:?}",
            cb.all()
        );
        assert!(cb.contains("[预演] 产物 app.apk → "), "日志：{:?}", cb.all());
        assert_eq!(
            fs::read_dir(&out).unwrap().count(),
            0,
            "dry_run 不得产生任何文件"
        );
        assert!(summary.ok.is_empty() && summary.failed.is_empty());
        assert_eq!(snapshot(&src), before, "源目录文件清单被改变");
    }

    /// 第一个包完成后取消 → summary.warns 含「用户取消」，且只处理了一个包。
    #[test]
    fn cancel_after_first_package() {
        if !have_tools(&[SEVENZ]) {
            return;
        }
        let tmp = tempdir().unwrap();
        let (src, out, failed) = three_dirs(&tmp);
        let work = tempdir().unwrap();
        for (z, t) in [("a.zip", "a.txt"), ("b.zip", "b.txt")] {
            fs::write(work.path().join(t), t).unwrap();
            sh(Command::new(SEVENZ)
                .args(["a", "-tzip"])
                .arg(&src.join(z))
                .arg(t)
                .current_dir(work.path()));
        }

        struct Cancel {
            armed: AtomicBool,
        }
        impl RunCallback for Cancel {
            fn on_log(&self, _msg: &str, _level: LogLevel) {}
            fn on_progress(&self, done: usize, _total: usize, _name: &str) {
                if done >= 1 {
                    self.armed.store(true, Ordering::SeqCst);
                }
            }
            fn should_cancel(&self) -> bool {
                self.armed.load(Ordering::SeqCst)
            }
        }

        let summary = run(
            &[src.clone()],
            &out,
            Some(&failed),
            tool_cfg(&[]),
            vec![],
            &Cancel {
                armed: AtomicBool::new(false),
            },
            false,
            true,
        );
        assert!(
            summary.warns.iter().any(|w| w == "用户取消"),
            "warns={:?}",
            summary.warns
        );
        assert_eq!(summary.ok.len(), 1, "应只处理完第一个包：{:?}", summary.ok);
    }
}
