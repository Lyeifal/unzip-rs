//! 分卷组装与改名卷领养（移植自 unzip_core.py 581-778 行）。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use crate::extract::{link_or_copy, Extractor};
use crate::sniff::expected_vol_name;
use crate::types::{normalize, ArchiveKind, LogLevel, Package, PkgKind, RunCallback, ScanResult};

/// 每个缺失卷最多试装的候选数（对应 Python MAX_TRY_PER_SLOT）。
const MAX_TRY_PER_SLOT: usize = 12;
/// 试装总轮数上限（对齐 Python tries < 60）。
const MAX_TRIES: u32 = 60;

/// unrar 探针结果。
pub enum UnrarProbe {
    /// unrar 报告缺少该卷（basename）
    Missing(String),
    /// 加密（继续走密码流程）
    Encrypted,
    /// 卷组完整
    Complete,
    /// 其他失败
    Other(i32, String),
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 去重保序。
fn dedup(items: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for i in items {
        if !out.contains(i) {
            out.push(i.clone());
        }
    }
    out
}

/// 组装校验用的轻量测试。返回 (ok, text)。
/// rar 族用 unrar 循环试前 4 个密码（空密码不带 -p）；非 rar 用 7z `t -p`，rc≤1 为 ok。
fn test_archive(ex: &Extractor, path: &Path, is_rar: bool, passwords: &[String]) -> (bool, String) {
    if is_rar {
        let mut cand = vec![String::new()];
        cand.extend(passwords.iter().cloned());
        let mut text = String::new();
        for pw in dedup(&cand).iter().take(4) {
            let args = if pw.is_empty() {
                vec!["t".to_string(), path.to_string_lossy().into_owned()]
            } else {
                vec!["t".to_string(), format!("-p{pw}"), path.to_string_lossy().into_owned()]
            };
            let Some(r) = ex.run_unrar(&args) else {
                return (false, "找不到 UnRAR.exe".to_string());
            };
            text = r.text.clone();
            if r.code == Some(0) || !text.to_lowercase().contains("password") {
                return (r.code == Some(0), text);
            }
        }
        (false, text)
    } else {
        match ex.run_7z(&[
            "t".to_string(),
            path.to_string_lossy().into_owned(),
            "-p".to_string(),
        ]) {
            None => (false, "找不到 7z.exe".to_string()),
            Some(r) => (r.code.is_some_and(|c| c <= 1), r.text),
        }
    }
}

/// 在 unrar 输出里找 `Cannot find volume\s+(\S+)`（大小写敏感，与 Python 一致），
/// 返回缺失卷 basename（取 \ 或 / 后最后一段）。
fn find_missing_volume(text: &str) -> Option<String> {
    const KEY: &str = "Cannot find volume";
    let pos = text.find(KEY)?;
    let rest = text[pos + KEY.len()..].trim_start();
    let name: String = rest
        .chars()
        .take_while(|c| !c.is_whitespace())
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(
            name.rsplit(|c| c == '\\' || c == '/')
                .next()
                .unwrap_or(&name)
                .to_string(),
        )
    }
}

/// 跑 unrar t 探针：对去重(passwords) 前 4 个逐个 t -p{pw}（pw 为空则不带 -p）。
/// 循环体内无条件 return（与 Python 一致），只有密码列表为空时才落到最后一个状态。
fn unrar_probe(ex: &Extractor, path: &Path, passwords: &[String]) -> UnrarProbe {
    let cands = dedup(passwords);
    if cands.is_empty() {
        return UnrarProbe::Other(255, String::new());
    }
    let path_s = path.to_string_lossy().into_owned();
    for pw in cands.iter().take(4) {
        let args = if pw.is_empty() {
            vec!["t".to_string(), path_s.clone()]
        } else {
            vec!["t".to_string(), format!("-p{pw}"), path_s.clone()]
        };
        let Some(r) = ex.run_unrar(&args) else {
            return UnrarProbe::Other(255, "找不到 UnRAR.exe".to_string());
        };
        let rc = r.code.unwrap_or(255);
        let text = r.text;
        if rc == 0 {
            return UnrarProbe::Complete;
        }
        if let Some(name) = find_missing_volume(&text) {
            return UnrarProbe::Missing(name);
        }
        let low = text.to_lowercase();
        if low.contains("password") || low.contains("encrypted") {
            return UnrarProbe::Encrypted;
        }
        return UnrarProbe::Other(rc, text);
    }
    UnrarProbe::Other(255, String::new())
}

fn size_of(p: &Path) -> u64 {
    p.metadata().map(|m| m.len()).unwrap_or(0)
}

/// 按 jaro_winkler 相似度降序、再按文件大小降序排序候选（只保留仍存在的文件）。
fn ranked_candidates(pool: &[PathBuf], expected_name: &str) -> Vec<PathBuf> {
    let exp = expected_name.to_lowercase();
    let mut live: Vec<PathBuf> = pool.iter().filter(|p| p.exists()).cloned().collect();
    live.sort_by(|a, b| {
        let sa = strsim::jaro_winkler(&file_name(a).to_lowercase(), &exp);
        let sb = strsim::jaro_winkler(&file_name(b).to_lowercase(), &exp);
        sb.partial_cmp(&sa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| size_of(b).cmp(&size_of(a)))
    });
    live
}

/// 同名（不区分大小写）候选优先 + 其余按相似度排序，取前 MAX_TRY_PER_SLOT（去重保序）。
fn slot_candidates(pool: &[PathBuf], needed: &str) -> Vec<PathBuf> {
    let mut cands: Vec<PathBuf> = pool
        .iter()
        .filter(|p| file_name(p).eq_ignore_ascii_case(needed))
        .cloned()
        .collect();
    for p in ranked_candidates(pool, needed) {
        if !cands.contains(&p) {
            cands.push(p);
        }
    }
    cands.truncate(MAX_TRY_PER_SLOT);
    cands
}

/// 把卷集组装进 tmp_dir（规范命名），卷齐且校验通过返回 (first_path, None)。
/// 支持全树规范卷归集、孤儿改名卷试装：rar 由 unrar 报缺失卷名驱动；
/// 7z/zip 推导卷名，用"错误文本不再提及该卷名"判定候选被接受。
pub fn assemble_volumes(
    ex: &Extractor,
    pkg: &Package,
    scan: &mut ScanResult,
    passwords: &[String],
    tmp_dir: &std::path::Path,
    cb: &dyn RunCallback,
) -> (PathBuf, Option<String>) {
    let Some(family) = pkg.vol_family else {
        return (pkg.first.clone(), Some("包无卷族".to_string()));
    };
    let stem = pkg.vol_stem.clone();
    let is_rar = family.is_rar();
    let mut have: BTreeMap<u32, PathBuf> = pkg.volumes.clone();
    have.insert(1, pkg.first.clone());
    let mut adopted: Vec<PathBuf> = Vec::new();

    let log = |msg: &str| cb.on_log(&format!("[分卷] {}：{msg}", pkg.name), LogLevel::Info);

    for (idx, p) in &have {
        link_or_copy(p, &tmp_dir.join(expected_vol_name(family, &stem, *idx)));
    }
    let first_tmp = tmp_dir.join(expected_vol_name(family, &stem, 1));
    let own: HashSet<PathBuf> = have.values().map(|p| normalize(p)).collect();
    let mut pool: Vec<PathBuf> = scan
        .rar_orphans
        .iter()
        .chain(scan.orphans.iter())
        .filter(|p| p.exists() && !own.contains(&normalize(p)) && !scan.adopted.contains(*p))
        .cloned()
        .collect();

    let mut tries = 0u32;
    if is_rar {
        let mut missing = match unrar_probe(ex, &first_tmp, passwords) {
            UnrarProbe::Missing(name) => Some(name),
            _ => None,
        };
        while missing.is_some() && tries < MAX_TRIES {
            tries += 1;
            let needed = missing.take().unwrap();
            let mut placed = false;
            for cand in slot_candidates(&pool, &needed) {
                if !cand.exists() {
                    continue;
                }
                let target = tmp_dir.join(&needed);
                link_or_copy(&cand, &target);
                let accept = match unrar_probe(ex, &first_tmp, passwords) {
                    UnrarProbe::Complete => true,
                    UnrarProbe::Missing(m2) if m2 != needed => {
                        missing = Some(m2);
                        true
                    }
                    _ => false,
                };
                if accept {
                    if let Some(pos) = pool.iter().position(|p| p == &cand) {
                        pool.remove(pos);
                    }
                    adopted.push(cand.clone());
                    log(&format!("{} 即 {needed}", file_name(&cand)));
                    placed = true;
                    break;
                }
                let _ = std::fs::remove_file(&target);
            }
            if !placed {
                log(&format!("缺少 {needed}，无候选可补"));
                break;
            }
        }
    } else {
        while tries < MAX_TRIES {
            tries += 1;
            let (ok, text) = test_archive(ex, &first_tmp, false, passwords);
            if ok {
                break;
            }
            if text.to_lowercase().contains("password") {
                break; // 卷齐但加密，交给正式解压按密码库处理
            }
            let mut idx = 1u32;
            while have.contains_key(&idx) {
                idx += 1;
            }
            let needed = expected_vol_name(family, &stem, idx);
            let mut placed = false;
            for cand in ranked_candidates(&pool, &needed)
                .into_iter()
                .take(MAX_TRY_PER_SLOT)
            {
                if !cand.exists() {
                    continue;
                }
                let target = tmp_dir.join(&needed);
                link_or_copy(&cand, &target);
                let (ok2, text2) = test_archive(ex, &first_tmp, false, passwords);
                if ok2 || !text2.to_lowercase().contains(&needed.to_lowercase()) {
                    if let Some(pos) = pool.iter().position(|p| p == &cand) {
                        pool.remove(pos);
                    }
                    adopted.push(cand.clone());
                    log(&format!("{} 即 {needed}", file_name(&cand)));
                    have.insert(idx, cand.clone());
                    placed = true;
                    break;
                }
                let _ = std::fs::remove_file(&target);
            }
            if !placed {
                log(&format!("缺少 {needed}，无候选可补"));
                break;
            }
        }
    }

    for p in &adopted {
        scan.rar_orphans.retain(|x| x != p);
        scan.orphans.retain(|x| x != p);
        scan.adopted.push(p.clone());
    }
    let (ok, text) = test_archive(ex, &first_tmp, is_rar, passwords);
    // 失败若是 "password" 字样，说明卷已齐、只是加密（7z 探针固定空密码 -p，不解密），
    // 放行交给正式解压按密码库尝试；缺卷/损坏才是真失败（对齐补卷循环的同款判断）。
    if !ok && !is_rar && !text.to_lowercase().contains("password") {
        // 夸克分段兜底：文件名像分卷但切点在任意字节（夸克按段下载），
        // 规范名归集后 7z 按多卷打不开 → 按序拼接成单档再试。
        if let Some(joined) = concat_volumes(&have, tmp_dir, &stem) {
            let (ok2, text2) = test_archive(ex, &joined, false, passwords);
            if ok2 || text2.to_lowercase().contains("password") {
                return (joined, None);
            }
        }
    }
    let err = if ok || text.to_lowercase().contains("password") {
        None
    } else {
        Some("分卷组装后校验未通过（可能缺卷或卷内容不对）".to_string())
    };
    (first_tmp, err)
}

/// 按 idx 序把全族卷拼接成单档（1..=max 必须连续无缺卷），供夸克任意字节分段兜底。
fn concat_volumes(have: &BTreeMap<u32, PathBuf>, tmp_dir: &std::path::Path, stem: &str) -> Option<PathBuf> {
    let max = *have.keys().max()?;
    if (1..=max).any(|i| !have.contains_key(&i)) {
        return None; // 中段缺卷拼了也白拼
    }
    use std::io::{BufReader, BufWriter, Write};
    let out = tmp_dir.join(format!("{stem}.joined.7z"));
    let w = std::fs::File::create(&out).ok()?;
    let mut w = BufWriter::with_capacity(8 << 20, w);
    for i in 1..=max {
        let f = std::fs::File::open(have.get(&i)?).ok()?;
        let mut r = BufReader::with_capacity(8 << 20, f);
        std::io::copy(&mut r, &mut w).ok()?;
    }
    w.flush().ok()?;
    Some(out)
}

/// 独立包解压失败后的拯救：可能是改了名的首卷或散卷（rar 由 unrar 报真实卷名）。
/// 成功返回 (first_path, tmp_dir)，失败返回 None（tmp 目录随 TempDir drop 清理）。
pub fn rescue_renamed_first(
    ex: &Extractor,
    pkg: &Package,
    scan: &mut ScanResult,
    passwords: &[String],
    out_root: &std::path::Path,
    cb: &dyn RunCallback,
) -> Option<(PathBuf, tempfile::TempDir)> {
    if pkg.kind != PkgKind::Archive(ArchiveKind::Rar) {
        return None;
    }
    let own = normalize(&pkg.first);
    let pool: Vec<PathBuf> = scan
        .rar_orphans
        .iter()
        .chain(scan.orphans.iter())
        .filter(|p| p.exists() && normalize(p) != own && !scan.adopted.contains(*p))
        .cloned()
        .collect();
    if pool.is_empty() {
        return None;
    }
    let tmp = tempfile::Builder::new()
        .prefix(".rescue_")
        .tempdir_in(out_root)
        .ok()?;
    let tmp_dir = tmp.path().to_path_buf();
    let first_tmp = tmp_dir.join(file_name(&pkg.first));
    link_or_copy(&pkg.first, &first_tmp);
    let mut adopted: Vec<PathBuf> = Vec::new();

    let mut missing = match unrar_probe(ex, &first_tmp, passwords) {
        UnrarProbe::Missing(name) => Some(name),
        _ => None,
    };
    let mut tries = 0u32;
    while missing.is_some() && tries < MAX_TRIES {
        tries += 1;
        let needed = missing.take().unwrap();
        let mut placed = false;
        for cand in slot_candidates(&pool, &needed) {
            if !cand.exists() {
                continue;
            }
            let target = tmp_dir.join(&needed);
            link_or_copy(&cand, &target);
            let accept = match unrar_probe(ex, &first_tmp, passwords) {
                UnrarProbe::Complete => {
                    cb.on_log(
                        &format!(
                            "[分卷] {}：{} 即 {needed}，卷组完整",
                            pkg.name,
                            file_name(&cand)
                        ),
                        LogLevel::Info,
                    );
                    missing = None;
                    true
                }
                UnrarProbe::Missing(m2) if m2 != needed => {
                    cb.on_log(
                        &format!("[分卷] {}：{} 即 {needed}", pkg.name, file_name(&cand)),
                        LogLevel::Info,
                    );
                    missing = Some(m2);
                    true
                }
                _ => false,
            };
            if accept {
                adopted.push(cand.clone());
                placed = true;
                break;
            }
            let _ = std::fs::remove_file(&target);
        }
        if !placed {
            drop(tmp); // 清理临时目录
            return None;
        }
    }

    let (ok, _) = test_archive(ex, &first_tmp, true, passwords);
    if ok {
        for p in &adopted {
            scan.rar_orphans.retain(|x| x != p);
            scan.orphans.retain(|x| x != p);
            scan.adopted.push(p.clone());
        }
        scan.adopted.push(pkg.first.clone());
        Some((first_tmp, tmp)) // TempDir 交给调用方持有，drop 时清理
    } else {
        drop(tmp); // 清理临时目录
        None
    }
}
