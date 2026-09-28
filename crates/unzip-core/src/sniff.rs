//! 类型识别：魔数嗅探、媒体判定、卷名解析（移植自 unzip_core.py 150-232 行）。

use std::io::Read;
use std::path::Path;

use crate::types::{PkgKind, VolFamily, VolumeRef};

/// 魔数表（顺序即 Python ARCHIVE_MAGICS 的匹配顺序）。
const ARCHIVE_MAGICS: &[(&[u8], &str)] = &[
    (b"PK\x03\x04", "zip"),
    (b"PK\x05\x06", "zip"),
    (b"PK\x07\x08", "zip"),
    (b"Rar!\x1a\x07\x00", "rar"),
    (b"Rar!\x1a\x07\x01\x00", "rar"),
    (b"7z\xbc\xaf\x27\x1c", "7z"),
    (b"\x1f\x8b", "gzip"),
    (b"BZh", "bzip2"),
    (b"\xfd7zXZ\x00", "xz"),
    (b"\x04\x22\x4d\x18", "lz4"),
    (b"\x02\x21\x4c\x18", "lz4"), // lz4 legacy（lz4 -l，老发布组常用）
    (b"\x28\xb5\x2f\xfd", "zstd"),
];

/// 无魔数时按扩展名回退（对应 Python EXT_FALLBACK）。
const EXT_FALLBACK: &[(&str, &str)] = &[
    (".zip", "zip"),
    (".rar", "rar"),
    (".7z", "7z"),
    (".gz", "gzip"),
    (".bz2", "bzip2"),
    (".xz", "xz"),
    (".tar", "tar"),
    (".tgz", "gzip"),
    (".tbz2", "bzip2"),
    (".txz", "xz"),
    (".zst", "zstd"),
    (".lz4", "lz4"),
];

/// 复合 tar 后缀（对应 Python archive_stem 里的元组，顺序一致）。
const TAR_SUFFIXES: &[&str] = &[
    ".tar.gz", ".tar.bz2", ".tar.xz", ".tgz", ".tbz2", ".tbz", ".txz",
];

/// `.7z|.zip|.rar.\d{3}` 复合后缀中点号部分的长度（".7z"=3，其余 4）。
const NUMERIC_VOL_SEPS: &[(&str, usize)] = &[(".7z", 3), (".zip", 4), (".rar", 4)];

/// 读文件头识别，返回 kind 字符串（"zip"/"rar"/"7z"/"gzip"/"bzip2"/"xz"/"tar"/"zstd"/"lz4"/"media"）。
pub fn sniff(path: &Path) -> Option<&'static str> {
    let file = std::fs::File::open(path).ok()?;
    let mut head = Vec::new();
    file.take(600).read_to_end(&mut head).ok()?;
    for (magic, kind) in ARCHIVE_MAGICS {
        if head.starts_with(magic) {
            return Some(kind);
        }
    }
    if head.len() > 262 && &head[257..262] == b"ustar" {
        return Some("tar");
    }
    let media = head.starts_with(b"\xff\xd8\xff")
        || head.starts_with(b"\x89PNG\r\n\x1a\n")
        || head.starts_with(b"GIF87a")
        || head.starts_with(b"GIF89a")
        || head.starts_with(b"BM")
        || (head.len() > 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP")
        || (head.len() > 8 && &head[4..8] == b"ftyp")
        || head.starts_with(b"ID3")
        || head.starts_with(b"\xff\xfb")
        || head.starts_with(b"\xff\xf3")
        || head.starts_with(b"\xff\xf2");
    if media {
        return Some("media");
    }
    None
}

/// 与 Python classify 完全一致的字符串版：返回 "product"/"media"/kind/None。
/// （`classify` 的 PkgKind 无法表示 "media"，扫描层需要用本函数保留该区分。）
pub(crate) fn classify_str(path: &Path, product_exts: &[String]) -> Option<&'static str> {
    let name = file_name_string(path);
    let ext = suffix_with_dot(&name);
    if !product_exts.is_empty() {
        let ext_lower = ext.to_lowercase();
        if product_exts.iter().any(|e| e.to_lowercase() == ext_lower) {
            return Some("product");
        }
    }
    if let Some(kind) = sniff(path) {
        return Some(kind);
    }
    let ext_lower = ext.to_lowercase();
    EXT_FALLBACK
        .iter()
        .find(|(e, _)| *e == ext_lower)
        .map(|(_, k)| *k)
}

/// 返回压缩包种类 / product / media / None。apk 族按扩展名优先判为产物。
pub fn classify(path: &Path, product_exts: &[String]) -> Option<PkgKind> {
    match classify_str(path, product_exts)? {
        // PkgKind 没有 media 变体；media 判定请走 sniff/classify_str（扫描层如此）。
        "media" => None,
        k => PkgKind::from_kind_str(k),
    }
}

/// 类似 Python Path.suffix：最后一个 `.` 起的部分（含点）；无前缀点或无点返回 ""。
pub(crate) fn suffix_with_dot(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[i..],
        _ => "",
    }
}

/// 类似 Python Path.stem：去掉最后一个后缀；点开头的名字整体视为 stem。
fn path_stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

fn file_name_string(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 解析纯 ASCII 十进制数字（容忍前导零；超长时饱和到 u32::MAX，
/// 保住 Python 语义里 "idx>1 / idx==1" 的判定）。
fn parse_idx(digits: &str) -> u32 {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        0
    } else {
        trimmed.parse::<u32>().unwrap_or(u32::MAX)
    }
}

/// ASCII 大小写折叠（不改字节长度）。卷名/复合后缀模式全是 ASCII：
/// 折叠后对 ASCII 的匹配结果与 Python lower() 一致，且 UTF-8 多字节序列
/// 不可能匹配 ASCII 模式字节，所有派生索引都落在字符边界上。
fn ascii_lower(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(|b| b.to_ascii_lowercase()).collect()
}

/// 在 hay 中从后往前找 needle（首次匹配取最靠后的起点）。
fn rfind_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len())
        .rev()
        .find(|&i| &hay[i..i + needle.len()] == needle)
}

/// 压缩包主名：a.tar.gz → a；a.7z.001 → a；a.part1.rar → a。
pub fn archive_stem(name: &str) -> String {
    let lower = ascii_lower(name.as_bytes());
    for suffix in TAR_SUFFIXES {
        if lower.ends_with(suffix.as_bytes()) {
            // suffix 全 ASCII 且等长折叠 → 索引即 name 的字符边界
            return name[..name.len() - suffix.len()].to_string();
        }
    }
    // ^(.*)\.(?:7z|zip|rar)\.\d{3}$（锚定结尾；贪婪 .* 对结尾锚无影响）
    for (sep, sep_len) in NUMERIC_VOL_SEPS {
        let total = sep_len + 4; // sep + "." + 3 位数字
        if lower.len() >= total {
            let tail = &lower[lower.len() - 4..];
            let head = &lower[lower.len() - total..lower.len() - 4];
            if tail[0] == b'.'
                && tail[1..].iter().all(|c| c.is_ascii_digit())
                && head == sep.as_bytes()
            {
                return name[..name.len() - total].to_string();
            }
        }
    }
    // ^(.*)\.part\d+\.rar$（贪婪 .* = 最后一个合法的 .part<数字>）
    if lower.ends_with(b".rar") {
        let body_len = lower.len() - 4;
        if let Some(pos) = rfind_subslice(&lower[..body_len], b".part") {
            let digits = &lower[pos + 5..body_len];
            if !digits.is_empty() && digits.iter().all(|c| c.is_ascii_digit()) {
                return name[..pos].to_string();
            }
        }
    }
    path_stem(name).to_string()
}

/// 从文件名解析卷引用（小写匹配，stem 取自小写文件名）。非卷文件返回 None。
pub fn parse_volume_name(name: &str) -> Option<VolumeRef> {
    let n = ascii_lower(name.as_bytes());
    // ^(.*)\.part(\d+)\.rar$
    if n.ends_with(b".rar") {
        let body_len = n.len() - 4;
        if let Some(pos) = rfind_subslice(&n[..body_len], b".part") {
            let digits = &n[pos + 5..body_len];
            if !digits.is_empty() && digits.iter().all(|c| c.is_ascii_digit()) {
                return Some(VolumeRef {
                    stem: String::from_utf8_lossy(&n[..pos]).into_owned(),
                    family: VolFamily::RarPart,
                    idx: parse_idx(std::str::from_utf8(digits).unwrap_or("")),
                });
            }
        }
    }
    // ^(.*)\.7z\.(\d{3})$ | ^(.*)\.zip\.(\d{3})$（注意：Python 没有 .rar.\d{3} 族）
    for (sep, family) in [(".7z", VolFamily::SevenZNum), (".zip", VolFamily::ZipNum)] {
        let total = sep.len() + 4;
        if n.len() >= total {
            let head = &n[n.len() - total..n.len() - 4];
            let dot = n[n.len() - 4];
            let digits = &n[n.len() - 3..];
            if dot == b'.' && digits.iter().all(|c| c.is_ascii_digit()) && head == sep.as_bytes() {
                return Some(VolumeRef {
                    stem: String::from_utf8_lossy(&n[..n.len() - total]).into_owned(),
                    family,
                    idx: parse_idx(std::str::from_utf8(digits).unwrap_or("")),
                });
            }
        }
    }
    // ^(.*)\.r(\d{2})$ | ^(.*)\.z(\d{2})$
    for (letter, family) in [(b'r', VolFamily::RarR), (b'z', VolFamily::ZipZ)] {
        if n.len() >= 4 && n[n.len() - 4] == b'.' && n[n.len() - 3] == letter {
            let digits = &n[n.len() - 2..];
            if digits.iter().all(|c| c.is_ascii_digit()) {
                return Some(VolumeRef {
                    stem: String::from_utf8_lossy(&n[..n.len() - 4]).into_owned(),
                    family,
                    idx: parse_idx(std::str::from_utf8(digits).unwrap_or(""))
                        .saturating_add(1),
                });
            }
        }
    }
    None
}

/// 规范卷名：family + stem + idx → 文件名。
pub fn expected_vol_name(family: VolFamily, stem: &str, idx: u32) -> String {
    match family {
        VolFamily::RarPart => format!("{stem}.part{idx}.rar"),
        VolFamily::SevenZNum => format!("{stem}.7z.{idx:03}"),
        VolFamily::ZipNum => format!("{stem}.zip.{idx:03}"),
        VolFamily::RarR => {
            if idx == 1 {
                format!("{stem}.rar")
            } else {
                format!("{stem}.r{:02}", idx - 1)
            }
        }
        VolFamily::ZipZ => {
            if idx == 1 {
                format!("{stem}.zip")
            } else {
                format!("{stem}.z{idx:02}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_file(dir: &Path, name: &str, content: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        fs::write(&p, content).unwrap();
        p
    }

    // ---------- 魔数 ----------

    #[test]
    fn sniff_all_archive_magics() {
        let tmp = tempdir().unwrap();
        let cases: &[(&[u8], &str)] = &[
            (b"PK\x03\x04rest", "zip"),
            (b"PK\x05\x06rest", "zip"),
            (b"PK\x07\x08rest", "zip"),
            (b"Rar!\x1a\x07\x00", "rar"),
            (b"Rar!\x1a\x07\x01\x00", "rar"),
            (b"7z\xbc\xaf\x27\x1c", "7z"),
            (b"\x1f\x8b\x08", "gzip"),
            (b"BZh91", "bzip2"),
            (b"\xfd7zXZ\x00", "xz"),
            (b"\x04\x22\x4d\x18", "lz4"),
            (b"\x02\x21\x4c\x18", "lz4"), // lz4 legacy
            (b"\x28\xb5\x2f\xfd", "zstd"),
        ];
        for (i, (magic, kind)) in cases.iter().enumerate() {
            let p = write_file(tmp.path(), &format!("m{i}.bin"), magic);
            assert_eq!(sniff(&p), Some(*kind), "magic {:?}", magic);
        }
    }

    #[test]
    fn sniff_tar_ustar_at_offset_257() {
        let tmp = tempdir().unwrap();
        let mut buf = vec![0u8; 300];
        buf[257..262].copy_from_slice(b"ustar");
        let p = write_file(tmp.path(), "a.tar", &buf);
        assert_eq!(sniff(&p), Some("tar"));

        // 偏移不对 / 长度不够 → 不算 tar
        let mut buf2 = vec![0u8; 300];
        buf2[256..261].copy_from_slice(b"ustar");
        let p2 = write_file(tmp.path(), "b.tar", &buf2);
        assert_eq!(sniff(&p2), None);

        let mut buf3 = vec![0u8; 262]; // len == 262，不满足 > 262
        buf3[257..262].copy_from_slice(b"ustar");
        let p3 = write_file(tmp.path(), "c.tar", &buf3);
        assert_eq!(sniff(&p3), None);
    }

    #[test]
    fn sniff_media_signatures() {
        let tmp = tempdir().unwrap();
        let mut mp4 = vec![0u8; 16];
        mp4[4..8].copy_from_slice(b"ftyp");
        let mut webp = vec![0u8; 16];
        webp[0..4].copy_from_slice(b"RIFF");
        webp[8..12].copy_from_slice(b"WEBP");
        let cases: &[(&[u8], &str)] = &[
            (b"\xff\xd8\xff\xe0\x00\x10JFIF", "jpg"),
            (b"\x89PNG\r\n\x1a\n1234", "png"),
            (b"GIF87a", "gif"),
            (b"GIF89a", "gif"),
            (b"BM6\x00\x00\x00", "bmp"),
            (&webp, "webp"),
            (&mp4, "mp4"),
            (b"ID3\x04\x00\x00\x00", "mp3"),
            (b"\xff\xfb\x90\x00", "mp3"),
            (b"\xff\xf3\x90\x00", "mp3"),
            (b"\xff\xf2\x90\x00", "mp3"),
        ];
        for (i, (head, label)) in cases.iter().enumerate() {
            let p = write_file(tmp.path(), &format!("media{i}.bin"), head);
            assert_eq!(sniff(&p), Some("media"), "{label}");
        }
        // RIFF 但非 WEBP / ftyp 长度不足 → 不判媒体
        let riff_other = write_file(tmp.path(), "r.avif", b"RIFF\x00\x00\x00\x00AVI ");
        assert_eq!(sniff(&riff_other), None);
        // 空文件 / 未知内容 → None
        let empty = write_file(tmp.path(), "empty.bin", b"");
        assert_eq!(sniff(&empty), None);
        let unknown = write_file(tmp.path(), "unk.dat", b"\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09");
        assert_eq!(sniff(&unknown), None);
    }

    // ---------- classify ----------

    #[test]
    fn classify_product_ext_wins_over_magic() {
        let tmp = tempdir().unwrap();
        let cfg_exts: Vec<String> = [".apk", ".xapk", ".apks", ".aab"].iter().map(|s| s.to_string()).collect();
        // zip 魔数但 .apk 扩展名 → 产物
        let apk = write_file(tmp.path(), "app.apk", b"PK\x03\x04xxxx");
        assert_eq!(classify_str(&apk, &cfg_exts), Some("product"));
        assert_eq!(classify(&apk, &cfg_exts), Some(PkgKind::Product));
        // 大写扩展名同样命中（小写比较）
        let apk2 = write_file(tmp.path(), "app2.APK", b"\xff\xd8\xffxxxx");
        assert_eq!(classify_str(&apk2, &cfg_exts), Some("product"));
        // 不在 product 名单 → 走魔数
        let jpg = write_file(tmp.path(), "photo.jpg", b"\xff\xd8\xffxxxx");
        assert_eq!(classify_str(&jpg, &cfg_exts), Some("media"));
        assert_eq!(classify(&jpg, &cfg_exts), None); // PkgKind 无 media 变体
        // 无魔数 → 扩展名回退
        let zip = write_file(tmp.path(), "plain.zip", b"not a zip");
        assert_eq!(classify_str(&zip, &cfg_exts), Some("zip"));
        assert_eq!(classify(&zip, &cfg_exts), Some(PkgKind::Archive(crate::types::ArchiveKind::Zip)));
    }

    #[test]
    fn classify_ext_fallback_table() {
        let tmp = tempdir().unwrap();
        let cases: &[(&str, &str)] = &[
            (".zip", "zip"), (".rar", "rar"), (".7z", "7z"), (".gz", "gzip"),
            (".bz2", "bzip2"), (".xz", "xz"), (".tar", "tar"), (".tgz", "gzip"),
            (".tbz2", "bzip2"), (".txz", "xz"), (".zst", "zstd"), (".lz4", "lz4"),
        ];
        for (i, (ext, kind)) in cases.iter().enumerate() {
            let p = write_file(tmp.path(), &format!("f{i}{ext}"), b"no magic here");
            assert_eq!(classify_str(&p, &[]), Some(*kind), "{ext}");
        }
        // 未知扩展名 → None
        let other = write_file(tmp.path(), "x.xyz123", b"no magic here");
        assert_eq!(classify_str(&other, &[]), None);
    }

    // ---------- archive_stem ----------

    #[test]
    fn archive_stem_all_forms() {
        assert_eq!(archive_stem("a.tar.gz"), "a");
        assert_eq!(archive_stem("a.tar.bz2"), "a");
        assert_eq!(archive_stem("a.tar.xz"), "a");
        assert_eq!(archive_stem("a.tgz"), "a");
        assert_eq!(archive_stem("a.tbz2"), "a");
        assert_eq!(archive_stem("a.tbz"), "a");
        assert_eq!(archive_stem("a.txz"), "a");
        assert_eq!(archive_stem("multi.part.name.tar.gz"), "multi.part.name");
        // 大小写混合：小写匹配，stem 取自原文件名
        assert_eq!(archive_stem("A.TAR.GZ"), "A");
        assert_eq!(archive_stem("MyPack.7Z.001"), "MyPack");
        assert_eq!(archive_stem("a.7z.001"), "a");
        assert_eq!(archive_stem("a.zip.002"), "a");
        assert_eq!(archive_stem("a.rar.003"), "a");
        assert_eq!(archive_stem("a.part1.rar"), "a");
        assert_eq!(archive_stem("a.part07.rar"), "a");
        assert_eq!(archive_stem("a.PART07.RAR"), "a");
        assert_eq!(archive_stem("a.part1.part2.rar"), "a.part1"); // 贪婪 .*：最后一个 .part
        // 无卷模式 → 普通 stem
        assert_eq!(archive_stem("plain.zip"), "plain");
        assert_eq!(archive_stem("plain"), "plain");
        assert_eq!(archive_stem(".bashrc"), ".bashrc");
        // 不合法形态
        assert_eq!(archive_stem("a.7z.0001"), "a.7z"); // \d{3} 锚定结尾：仅去最后一段
        assert_eq!(archive_stem("a.part.rar"), "a.part"); // 无数字
    }

    // ---------- parse_volume_name ----------

    #[test]
    fn parse_volume_name_families() {
        let v = parse_volume_name("a.part1.rar").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("a", VolFamily::RarPart, 1));
        let v = parse_volume_name("a.part12.rar").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("a", VolFamily::RarPart, 12));

        let v = parse_volume_name("a.7z.001").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("a", VolFamily::SevenZNum, 1));
        let v = parse_volume_name("a.7z.123").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("a", VolFamily::SevenZNum, 123));

        let v = parse_volume_name("a.zip.001").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("a", VolFamily::ZipNum, 1));

        // .r00 是 idx 1（int+1）
        let v = parse_volume_name("a.r00").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("a", VolFamily::RarR, 1));
        let v = parse_volume_name("a.r03").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("a", VolFamily::RarR, 4));
        let v = parse_volume_name("a.z00").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("a", VolFamily::ZipZ, 1));
        let v = parse_volume_name("a.z99").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("a", VolFamily::ZipZ, 100));

        // 大小写混合：小写匹配，stem 取自小写文件名
        let v = parse_volume_name("GAME.R00").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("game", VolFamily::RarR, 1));
        let v = parse_volume_name("Game.7Z.002").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("game", VolFamily::SevenZNum, 2));
        let v = parse_volume_name("MiX.Part3.RAR").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("mix", VolFamily::RarPart, 3));
        // part0 是合法卷名（idx 0，Python int("0")=0；扫描时既非 idx>1 也非首卷）
        let v = parse_volume_name("a.part0.rar").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("a", VolFamily::RarPart, 0));
    }

    #[test]
    fn parse_volume_name_non_volumes() {
        for name in [
            "a.rar", "a.zip", "a.7z", "a.r0", "a.z1", "a.r000", "a.7z.0001", "a.rar.001",
            "a.part.rar", "a.part1.zip", "readme.txt", "a", ".rar",
        ] {
            assert!(parse_volume_name(name).is_none(), "{name}");
        }
        // 贪婪：最后一个合法 .part
        let v = parse_volume_name("b.part1.part2.rar").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("b.part1", VolFamily::RarPart, 2));
    }

    // ---------- expected_vol_name ----------

    #[test]
    fn unicode_filename_no_panic() {
        // 回归：UTF-8 多字节文件名的字节切片必须落在字符边界上（曾对中文名 panic）。
        let pdf = "落象luoxiang - 副本 - 副本 - 副本.pdf";
        assert!(parse_volume_name(pdf).is_none());
        assert_eq!(archive_stem(pdf), "落象luoxiang - 副本 - 副本 - 副本");
        // 中文 + 卷名后缀
        let v = parse_volume_name("副本.7z.002").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("副本", VolFamily::SevenZNum, 2));
        let v = parse_volume_name("某游戏.PART02.RAR").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("某游戏", VolFamily::RarPart, 2));
        assert_eq!(archive_stem("副本.tar.gz"), "副本");
        // 长数字不是 r/z 族卷名（Python 正则 \d{2} 严格两位）
        assert!(parse_volume_name("a.r99999999999").is_none());
        // 两位数字上限：r99 → idx 100
        let v = parse_volume_name("a.r99").unwrap();
        assert_eq!(v.idx, 100);
    }

    #[test]
    fn expected_vol_name_all_families() {
        assert_eq!(expected_vol_name(VolFamily::RarPart, "a", 3), "a.part3.rar");
        assert_eq!(expected_vol_name(VolFamily::SevenZNum, "a", 1), "a.7z.001");
        assert_eq!(expected_vol_name(VolFamily::SevenZNum, "a", 12), "a.7z.012");
        assert_eq!(expected_vol_name(VolFamily::ZipNum, "a", 2), "a.zip.002");
        // rar-r / zip-z 的 idx1 是主卷
        assert_eq!(expected_vol_name(VolFamily::RarR, "a", 1), "a.rar");
        assert_eq!(expected_vol_name(VolFamily::RarR, "a", 2), "a.r01");
        assert_eq!(expected_vol_name(VolFamily::ZipZ, "a", 1), "a.zip");
        assert_eq!(expected_vol_name(VolFamily::ZipZ, "a", 3), "a.z03");
    }
}
