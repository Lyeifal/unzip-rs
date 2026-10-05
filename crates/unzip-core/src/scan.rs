//! 全树扫描与建包。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::config::Config;
use crate::sniff::{archive_stem, classify_str, parse_volume_name, suffix_with_dot};
use crate::types::{is_under, normalize, ArchiveKind, Package, PkgKind, ScanResult, VolFamily};

/// 无魔数未知文件的后缀黑名单（.json 去重；
/// .pdf 为 Rust 版新增：附带广告 PDF 很常见，按普通文件跳过而非「无法识别」告警）。
const ORPHAN_BLACKLIST: &[&str] = &[
    ".txt", ".md", ".nfo", ".url", ".html", ".htm", ".json", ".log", ".ini",
    ".lnk", ".bat", ".cmd", ".exe", ".msi", ".dll", ".sys", ".py", ".pdf",
];

/// 未下载完成的临时文件后缀（后缀匹配；.qkdownloading
/// 为 Rust 版新增：夸克网盘下载中的临时文件，避免半成品被当压缩包移入失败目录）。
const DOWNLOADING_SUFFIXES: &[&str] =
    &[".baiduyun.p.downloading", ".downloading", ".download", ".qkdownloading"];

fn file_name_string(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn is_skipped_location(f: &Path, out_root: Option<&Path>, failed_root: Option<&Path>) -> bool {
    out_root.is_some_and(|o| is_under(f, o)) || failed_root.is_some_and(|d| is_under(f, d))
}

/// 扫描源根：归档/产物/分卷归组/孤儿/跳过/警告。out_root、failed_root 内部跳过。
pub fn scan_sources(
    roots: &[PathBuf],
    recursive: bool,
    cfg: &Config,
    out_root: Option<&Path>,
    failed_root: Option<&Path>,
) -> ScanResult {
    let product_exts: Vec<String> = cfg.product_exts.iter().map(|e| e.to_lowercase()).collect();
    let mut scan = ScanResult::default();
    for root in roots {
        if !root.is_dir() {
            scan.warns.push(format!("源目录不存在：{}", root.display()));
            continue;
        }
        let files: Vec<PathBuf> = WalkDir::new(root)
            .max_depth(if recursive { usize::MAX } else { 1 })
            .into_iter()
            .filter_map(|e| e.ok())
            .map(walkdir::DirEntry::into_path)
            .filter(|p| p.is_file())
            .collect();

        // kinds 备忘：同一文件两遍扫描只嗅探一次（嵌入识别有大窗口 I/O，媒体多的树省一半）
        let mut kinds: std::collections::HashMap<PathBuf, Option<&'static str>> =
            std::collections::HashMap::new();
        let mut kind_of = |f: &Path| -> Option<&'static str> {
            *kinds
                .entry(f.to_path_buf())
                .or_insert_with(|| classify_str(f, &product_exts))
        };

        // 第一遍：只收分卷（idx>1）与首卷（idx==1 且是归档/lz4）。
        // 单遍收集会让 stem.rar/stem.zip 的主卷补位依赖枚举顺序；这里先收齐全部
        // 分卷，第二遍再补位，结果与顺序无关（唯一的行为改进，其余逻辑相同）。
        let mut consumed: HashSet<PathBuf> = HashSet::new();
        for f in &files {
            if is_skipped_location(f, out_root, failed_root) {
                continue;
            }
            let lower = file_name_string(f).to_lowercase();
            if DOWNLOADING_SUFFIXES.iter().any(|s| lower.ends_with(s)) {
                continue;
            }
            let kind = kind_of(f);
            if kind == Some("media") || kind == Some("product") {
                continue;
            }
            let Some(volref) = parse_volume_name(&lower) else { continue };
            if volref.idx > 1 {
                // 只要求 kind != "product"（无魔数的 .002 也算分卷）
                scan.volumes
                    .entry((volref.family, volref.stem))
                    .or_default()
                    .insert(volref.idx, f.clone());
                consumed.insert(f.clone());
            } else if kind.and_then(PkgKind::from_kind_str).is_some() {
                // 压缩档/lz4 且 idx==1 → vol_firsts（7z 可解族与 lz4 才能当首卷）
                scan.vol_firsts.insert((volref.family, volref.stem), f.clone());
                consumed.insert(f.clone());
            }
        }

        // 第二遍：处理其余文件（idx==1 首卷已入 vol_firsts，不会走到这里）。
        for f in &files {
            if consumed.contains(f) {
                continue;
            }
            if is_skipped_location(f, out_root, failed_root) {
                continue;
            }
            let lower = file_name_string(f).to_lowercase();
            if DOWNLOADING_SUFFIXES.iter().any(|s| lower.ends_with(s)) {
                scan.skips.push((f.clone(), "未下载完成的文件".to_string()));
                continue;
            }
            let kind = kind_of(f);
            if kind == Some("media") {
                scan.skips
                    .push((f.clone(), "真实图片/视频，不是压缩包".to_string()));
                continue;
            }
            if kind == Some("product") {
                scan.products.push(f.clone());
                continue;
            }
            match kind.and_then(PkgKind::from_kind_str) {
                Some(pkg_kind) => {
                    // 主卷补位：stem.rar / stem.zip（volumes 已收齐，判定与顺序无关）
                    if lower.ends_with(".rar")
                        && scan
                            .volumes
                            .contains_key(&(VolFamily::RarR, lower[..lower.len() - 4].to_string()))
                    {
                        scan.vol_firsts
                            .insert((VolFamily::RarR, lower[..lower.len() - 4].to_string()), f.clone());
                        continue;
                    }
                    if lower.ends_with(".zip")
                        && scan
                            .volumes
                            .contains_key(&(VolFamily::ZipZ, lower[..lower.len() - 4].to_string()))
                    {
                        scan.vol_firsts
                            .insert((VolFamily::ZipZ, lower[..lower.len() - 4].to_string()), f.clone());
                        continue;
                    }
                    scan.archives.push((f.clone(), pkg_kind));
                    if pkg_kind == PkgKind::Archive(ArchiveKind::Rar) {
                        // 双重身份：可被别人领养，也自成包
                        scan.rar_orphans.push(f.clone());
                    }
                }
                None => {
                    let suffix = suffix_with_dot(&lower);
                    if ORPHAN_BLACKLIST.contains(&suffix) {
                        scan.skips.push((f.clone(), "普通文件".to_string()));
                    } else {
                        scan.orphans.push(f.clone());
                    }
                }
            }
        }
    }
    scan
}

/// 由扫描结果建包：首卷+卷族一个包；独立档一个包；产物一个包。
/// 无主分卷警告追加进 scan.warns。
pub fn build_packages(scan: &mut ScanResult, roots: &[PathBuf]) -> Vec<Package> {
    let mut pkgs = Vec::new();
    let mut used_vol_keys: HashSet<(VolFamily, String)> = HashSet::new();

    for ((family, stem), first) in &scan.vol_firsts {
        used_vol_keys.insert((*family, stem.clone()));
        let vols = scan
            .volumes
            .get(&(*family, stem.clone()))
            .cloned()
            .unwrap_or_default();
        let kind = if family.is_rar() {
            PkgKind::Archive(ArchiveKind::Rar)
        } else if *family == VolFamily::SevenZNum {
            PkgKind::Archive(ArchiveKind::SevenZ)
        } else {
            PkgKind::Archive(ArchiveKind::Zip)
        };
        pkgs.push(make_pkg(first, kind, roots, Some(*family), stem, vols));
    }
    for (f, kind) in &scan.archives {
        pkgs.push(make_pkg(f, *kind, roots, None, "", BTreeMap::new()));
    }
    for f in &scan.products {
        pkgs.push(make_pkg(f, PkgKind::Product, roots, None, "", BTreeMap::new()));
    }

    // 无主分卷：找不到首卷的卷族，按文件名列出（排序仅为输出确定；
    // 排序仅为输出确定）。
    let mut leftover: Vec<(VolFamily, String)> = scan
        .volumes
        .keys()
        .filter(|k| !used_vol_keys.contains(*k))
        .cloned()
        .collect();
    leftover.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()).then_with(|| a.1.cmp(&b.1)));
    for key in leftover {
        let names = scan.volumes[&key]
            .values()
            .map(|p| file_name_string(p))
            .collect::<Vec<_>>()
            .join(", ");
        scan.warns.push(format!("无主分卷（找不到首卷，已忽略）：{names}"));
    }

    // 按首卷全路径小写排序（稳定排序）
    pkgs.sort_by(|a, b| {
        a.first
            .to_string_lossy()
            .to_lowercase()
            .cmp(&b.first.to_string_lossy().to_lowercase())
    });
    pkgs
}

/// root/rel 归属 + archive_stem 取名。
fn make_pkg(
    first: &Path,
    kind: PkgKind,
    roots: &[PathBuf],
    vol_family: Option<VolFamily>,
    vol_stem: &str,
    volumes: BTreeMap<u32, PathBuf>,
) -> Package {
    let mut root = None;
    let mut rel = None;
    for r in roots {
        // resolve(strict=False) 后取相对路径
        if let Ok(stripped) = normalize(first).strip_prefix(normalize(r)) {
            root = Some(r.clone());
            rel = Some(stripped.to_path_buf());
            break;
        }
    }
    let (root, rel) = match (root, rel) {
        (Some(r), Some(l)) => (r, l),
        _ => (
            first.parent().map(|p| p.to_path_buf()).unwrap_or_default(),
            PathBuf::from(file_name_string(first)),
        ),
    };
    Package {
        name: archive_stem(&file_name_string(first)),
        first: first.to_path_buf(),
        kind,
        rel,
        root,
        vol_family,
        vol_stem: vol_stem.to_string(),
        volumes,
        dest_base: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    const ZIP_MAGIC: &[u8] = b"PK\x03\x04\x14\x00\x00\x00";
    const RAR_MAGIC: &[u8] = b"Rar!\x1a\x07\x01\x00";
    const SEVENZ_MAGIC: &[u8] = b"7z\xbc\xaf\x27\x1c";
    const JPG_MAGIC: &[u8] = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00";

    fn write(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, content).unwrap();
        p
    }

    fn cfg() -> Config {
        Config::default()
    }

    fn skip_reasons(scan: &ScanResult) -> Vec<&str> {
        scan.skips.iter().map(|(_, r)| r.as_str()).collect()
    }

    // ---------- 分卷归组与两遍扫描 ----------

    #[test]
    fn rar_r_family_across_subdirs_main_volume_wins() {
        for _round in 0..2 {
            // 跑两轮：无论 walkdir 枚举顺序如何，结果必须一致
            let tmp = tempdir().unwrap();
            let root = tmp.path();
            let sub = root.join("sub");
            fs::create_dir(&sub).unwrap();
            let rar = write(root, "game.rar", RAR_MAGIC);
            let r00 = write(&sub, "game.r00", RAR_MAGIC);
            let r01 = write(root, "game.r01", RAR_MAGIC);

            let mut scan = scan_sources(&[root.to_path_buf()], true, &cfg(), None, None);
            // r00（idx1）先入 vol_firsts，主卷 game.rar 在第二遍补位覆盖
            assert_eq!(scan.vol_firsts[&(VolFamily::RarR, "game".to_string())], rar);
            assert_eq!(
                scan.volumes[&(VolFamily::RarR, "game".to_string())].get(&2),
                Some(&r01)
            );

            let pkgs = build_packages(&mut scan, &[root.to_path_buf()]);
            assert_eq!(pkgs.len(), 1, "主卷与散卷必须合并为一个包");
            let p = &pkgs[0];
            assert_eq!(p.kind, PkgKind::Archive(ArchiveKind::Rar));
            assert_eq!(p.vol_family, Some(VolFamily::RarR));
            assert_eq!(p.vol_stem, "game");
            assert_eq!(p.first, rar);
            assert_eq!(p.name, "game");
            assert_eq!(p.root, *root);
            assert_eq!(p.rel, PathBuf::from("game.rar"));
            assert_eq!(p.volumes.get(&2), Some(&r01));
            assert!(!p.volumes.values().any(|v| v == &r00)); // idx1 不在 volumes 里
            assert!(scan.warns.is_empty());
        }
    }

    #[test]
    fn seven_zip_num_family_volume_without_magic_still_collected() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let sub = root.join("sub");
        fs::create_dir(&sub).unwrap();
        let first = write(root, "a.7z.001", SEVENZ_MAGIC);
        // 真实场景 .002 无魔数；只要求 kind != "product"，仍算分卷
        let second = write(&sub, "a.7z.002", b"\x00\x01\x02\x03");

        let mut scan = scan_sources(&[root.to_path_buf()], true, &cfg(), None, None);
        assert_eq!(scan.vol_firsts[&(VolFamily::SevenZNum, "a".to_string())], first);
        let pkgs = build_packages(&mut scan, &[root.to_path_buf()]);
        assert_eq!(pkgs.len(), 1);
        let p = &pkgs[0];
        // kind 由族决定，而非文件魔数
        assert_eq!(p.kind, PkgKind::Archive(ArchiveKind::SevenZ));
        assert_eq!(p.vol_family, Some(VolFamily::SevenZNum));
        assert_eq!(p.name, "a");
        assert_eq!(p.first, first);
        assert_eq!(p.rel, PathBuf::from("a.7z.001"));
        assert_eq!(p.volumes.get(&2), Some(&second));
        assert_eq!(p.group_key(), "a"); // 根目录散包用包名
    }

    #[test]
    fn zip_z_main_volume_overwrites_z00_first() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let z00 = write(root, "z.z00", ZIP_MAGIC);
        let z01 = write(root, "z.z01", ZIP_MAGIC);
        let zmain = write(root, "z.zip", ZIP_MAGIC);

        let mut scan = scan_sources(&[root.to_path_buf()], true, &cfg(), None, None);
        assert_eq!(scan.vol_firsts[&(VolFamily::ZipZ, "z".to_string())], zmain);
        let pkgs = build_packages(&mut scan, &[root.to_path_buf()]);
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].kind, PkgKind::Archive(ArchiveKind::Zip));
        assert_eq!(pkgs[0].vol_family, Some(VolFamily::ZipZ));
        assert_eq!(pkgs[0].first, zmain);
        assert_eq!(pkgs[0].volumes.len(), 1); // 只有 idx2；idx1 的 z00 被主卷补位覆盖
        assert_eq!(pkgs[0].volumes.get(&2), Some(&z01));
        assert!(scan.warns.is_empty());
        let _ = z00; // z00 是 idx1，被主卷补位覆盖
    }

    #[test]
    fn rar_part_and_zip_num_families() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let d1 = root.join("d1");
        let d2 = root.join("d2");
        fs::create_dir(&d1).unwrap();
        fs::create_dir(&d2).unwrap();
        let p1 = write(&d1, "x.part1.rar", RAR_MAGIC);
        let p2 = write(&d2, "x.part2.rar", RAR_MAGIC);
        let q1 = write(&d2, "q.zip.001", ZIP_MAGIC);
        let q2 = write(&d1, "q.zip.002", ZIP_MAGIC);

        let mut scan = scan_sources(&[root.to_path_buf()], true, &cfg(), None, None);
        let pkgs = build_packages(&mut scan, &[root.to_path_buf()]);
        assert_eq!(pkgs.len(), 2);

        let px = pkgs.iter().find(|p| p.name == "x").unwrap();
        assert_eq!(px.kind, PkgKind::Archive(ArchiveKind::Rar));
        assert_eq!(px.vol_family, Some(VolFamily::RarPart));
        assert_eq!(px.first, p1);
        assert_eq!(px.volumes.get(&2), Some(&p2));
        assert_eq!(px.rel, PathBuf::from("d1").join("x.part1.rar"));
        assert_eq!(px.group_key(), "d1");

        let pq = pkgs.iter().find(|p| p.name == "q").unwrap();
        assert_eq!(pq.kind, PkgKind::Archive(ArchiveKind::Zip));
        assert_eq!(pq.vol_family, Some(VolFamily::ZipNum));
        assert_eq!(pq.first, q1);
        assert_eq!(pq.volumes.get(&2), Some(&q2));
        assert!(scan.warns.is_empty());
    }

    // ---------- 跳过 / 黑名单 / 孤儿 ----------

    #[test]
    fn media_blacklist_downloading_and_excluded_roots() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let out = tempdir().unwrap();
        let failed = tempdir().unwrap();

        write(root, "photo.jpg", JPG_MAGIC);
        write(root, "fake.zip", JPG_MAGIC); // 改后缀伪装：魔数优先
        write(root, "readme.txt", b"hello");
        write(root, "data.json", b"{}");
        write(root, "mystery.bin", b"\x00\x01\x02");
        write(root, "movie.zip.downloading", b"partial");
        write(root, "clip.baiduyun.p.downloading", b"partial");
        write(root, "x.downloa", ZIP_MAGIC); // 不匹配下载中后缀 → 正常识别
        write(out.path(), "inner.zip", ZIP_MAGIC);
        write(failed.path(), "bad.zip", ZIP_MAGIC);

        let scan = scan_sources(
            &[root.to_path_buf()],
            true,
            &cfg(),
            Some(out.path()),
            Some(failed.path()),
        );
        let reasons = skip_reasons(&scan);
        assert_eq!(
            reasons.iter().filter(|r| **r == "真实图片/视频，不是压缩包").count(),
            2
        );
        assert_eq!(reasons.iter().filter(|r| **r == "普通文件").count(), 2);
        assert_eq!(reasons.iter().filter(|r| **r == "未下载完成的文件").count(), 2);
        assert_eq!(scan.skips.len(), 6);
        assert!(scan.orphans.iter().any(|p| p.ends_with("mystery.bin")));
        assert_eq!(scan.orphans.len(), 1);
        // out_root / failed_root 之下完全跳过
        assert!(scan.archives.iter().all(|(p, _)| !p.starts_with(out.path())));
        assert!(scan.archives.iter().all(|(p, _)| !p.starts_with(failed.path())));
        assert_eq!(scan.archives.len(), 1); // 只有 x.downloa
        assert_eq!(scan.archives[0].1, PkgKind::Archive(ArchiveKind::Zip));
        assert!(scan.warns.is_empty());
    }

    #[test]
    fn unknown_downloading_suffix_exact_match() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        write(root, "a.download", ZIP_MAGIC);
        write(root, "b.downloading", ZIP_MAGIC);
        let scan = scan_sources(&[root.to_path_buf()], true, &cfg(), None, None);
        assert_eq!(skip_reasons(&scan), vec!["未下载完成的文件"; 2]);
        assert!(scan.archives.is_empty());
    }

    #[test]
    fn pdf_is_ordinary_file_and_qkdownloading_skipped() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        // 真 PDF（%PDF 头，无压缩档魔数）→ 普通文件，不产生「无法识别」告警
        write(root, "ad.pdf", b"%PDF-1.6\r%\xe2\xe3\xcf\xd3\r\n1 0 obj\n<<>>\n");
        // 夸克下载中的临时文件（有 7z 魔数也不行）→ 未下载完成，不建包
        write(root, "clip.7z-aaa.001.qkdownloading", SEVENZ_MAGIC);

        let scan = scan_sources(&[root.to_path_buf()], true, &cfg(), None, None);
        // 顺序无关断言（枚举序随文件系统而变）
        let reasons = skip_reasons(&scan);
        assert_eq!(reasons.len(), 2);
        assert!(reasons.contains(&"普通文件"));
        assert!(reasons.contains(&"未下载完成的文件"));
        assert!(scan.archives.is_empty());
        assert!(scan.orphans.is_empty());
        assert!(scan.warns.is_empty());
    }

    #[test]
    fn pdf_named_disguised_archive_still_extracted() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        // .pdf 黑名单只作用于「无魔数」文件：改后缀伪装包必须照常识别（魔数优先于黑名单）
        write(root, "head.pdf", RAR_MAGIC);
        let mut junk = vec![0u8; 33];
        junk.extend_from_slice(b"Rar!\x1a\x07\x01\x00xx");
        write(root, "junk.pdf", &junk); // 垃圾前缀 + rar 体，靠嵌入识别
        write(root, "real.pdf", b"%PDF-1.6\r%\xe2\xe3\xcf\xd3\r\n1 0 obj\n<<>>\n");

        let scan = scan_sources(&[root.to_path_buf()], true, &cfg(), None, None);
        assert_eq!(scan.archives.len(), 2, "伪装 pdf 必须进归档：{:?}", scan.archives);
        assert!(
            scan.archives.iter().all(|(_, k)| *k == PkgKind::Archive(ArchiveKind::Rar)),
            "两个伪装 pdf 都应是 rar：{:?}",
            scan.archives
        );
        assert_eq!(skip_reasons(&scan), vec!["普通文件"]);
        assert!(scan.orphans.is_empty());
    }

    // ---------- 归档 / 产物 / 递归 ----------

    #[test]
    fn archives_products_and_recursive_flag() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let deep = root.join("deep");
        fs::create_dir(&deep).unwrap();
        let solo = write(root, "solo.zip", ZIP_MAGIC);
        let nested = write(&deep, "nested.zip", ZIP_MAGIC);
        write(root, "app.apk", ZIP_MAGIC); // zip 魔数但产物扩展名优先

        let mut scan = scan_sources(&[root.to_path_buf()], true, &cfg(), None, None);
        assert_eq!(scan.archives.len(), 2);
        assert_eq!(scan.products.len(), 1);
        let pkgs = build_packages(&mut scan, &[root.to_path_buf()]);
        assert_eq!(pkgs.len(), 3);

        let p_solo = pkgs.iter().find(|p| p.name == "solo").unwrap();
        assert_eq!(p_solo.kind, PkgKind::Archive(ArchiveKind::Zip));
        assert_eq!(p_solo.first, solo);
        assert_eq!(p_solo.rel, PathBuf::from("solo.zip"));
        assert_eq!(p_solo.root, *root);
        assert_eq!(p_solo.group_key(), "solo");

        let p_nested = pkgs.iter().find(|p| p.name == "nested").unwrap();
        assert_eq!(p_nested.rel, PathBuf::from("deep").join("nested.zip"));
        assert_eq!(p_nested.group_key(), "deep");

        let p_app = pkgs.iter().find(|p| p.name == "app").unwrap();
        assert_eq!(p_app.kind, PkgKind::Product);

        // recursive=false 只扫根目录一层
        let scan_flat = scan_sources(&[root.to_path_buf()], false, &cfg(), None, None);
        assert_eq!(scan_flat.archives.len(), 1);
        assert_eq!(scan_flat.archives[0].0, solo);
        assert_eq!(scan_flat.products.len(), 1);
        let _ = nested;
    }

    #[test]
    fn rar_archive_double_identity() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let lone = write(root, "lone.rar", RAR_MAGIC);
        let mut scan = scan_sources(&[root.to_path_buf()], true, &cfg(), None, None);
        assert_eq!(scan.archives.len(), 1);
        assert_eq!(scan.rar_orphans, vec![lone.clone()]);
        let pkgs = build_packages(&mut scan, &[root.to_path_buf()]);
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].kind, PkgKind::Archive(ArchiveKind::Rar));
        assert_eq!(pkgs[0].vol_family, None);
        assert!(pkgs[0].volumes.is_empty());
    }

    // ---------- 警告 / 排序 / 归属 ----------

    #[test]
    fn orphan_volumes_warn_and_missing_root_warn() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        write(root, "lonely.7z.002", SEVENZ_MAGIC);
        write(root, "lonely.7z.003", SEVENZ_MAGIC);
        let missing = root.join("nope");

        let mut scan = scan_sources(
            &[root.to_path_buf(), missing.clone()],
            true,
            &cfg(),
            None,
            None,
        );
        assert!(
            scan.warns
                .iter()
                .any(|w| w == &format!("源目录不存在：{}", missing.display()))
        );

        let pkgs = build_packages(&mut scan, &[root.to_path_buf()]);
        assert!(pkgs.is_empty());
        assert_eq!(scan.warns.len(), 2);
        // BTreeMap 按 idx 序：002 在前
        assert!(
            scan.warns
                .iter()
                .any(|w| w == "无主分卷（找不到首卷，已忽略）：lonely.7z.002, lonely.7z.003")
        );
    }

    #[test]
    fn packages_sorted_by_lowercase_first_path() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let sub = root.join("SubDir");
        fs::create_dir(&sub).unwrap();
        let b = write(root, "b.zip", ZIP_MAGIC);
        let a = write(root, "a.zip", ZIP_MAGIC);
        let c = write(&sub, "c.zip", ZIP_MAGIC);

        let mut scan = scan_sources(&[root.to_path_buf()], true, &cfg(), None, None);
        let pkgs = build_packages(&mut scan, &[root.to_path_buf()]);
        let firsts: Vec<PathBuf> = pkgs.iter().map(|p| p.first.clone()).collect();
        // 全路径小写排序：root/a.zip < root/b.zip < root/subDir/c.zip
        let mut expected = vec![a, b, c];
        expected.sort_by_key(|p| p.to_string_lossy().to_lowercase());
        assert_eq!(firsts, expected);
        let lowered: Vec<String> = firsts.iter().map(|p| p.to_string_lossy().to_lowercase()).collect();
        let mut sorted = lowered.clone();
        sorted.sort();
        assert_eq!(lowered, sorted);
    }

    #[test]
    fn make_pkg_root_fallback_when_not_under_roots() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let other = tempdir().unwrap();
        let f = write(root, "solo.zip", ZIP_MAGIC);

        let mut scan = scan_sources(&[root.to_path_buf()], true, &cfg(), None, None);
        // 用不含该文件的 roots 建包：root=父目录、rel=文件名
        let pkgs = build_packages(&mut scan, &[other.path().to_path_buf()]);
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].root, f.parent().unwrap());
        assert_eq!(pkgs[0].rel, PathBuf::from("solo.zip"));
    }
}
