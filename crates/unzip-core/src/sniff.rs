//! 类型识别：魔数嗅探、媒体判定、嵌入伪装识别、卷名解析
//! 类型识别：魔数嗅探、媒体判定、嵌入伪装识别、卷名解析。

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::types::{PkgKind, VolFamily, VolumeRef};

/// 魔数表（匹配顺序即表序）。
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

/// 无魔数时按扩展名回退。
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

/// 复合 tar 后缀（archive_stem 按此表序匹配）。
const TAR_SUFFIXES: &[&str] = &[
    ".tar.gz", ".tar.bz2", ".tar.xz", ".tgz", ".tbz2", ".tbz", ".txz",
];

/// `.7z|.zip|.rar.\d{3}` 复合后缀中点号部分的长度（".7z"=3，其余 4）。
const NUMERIC_VOL_SEPS: &[(&str, usize)] = &[(".7z", 3), (".zip", 4), (".rar", 4)];

// ---------- 嵌入伪装识别（扩展能力） ----------
// 真实发布组常见手法：真图片/视频做封面 + 压缩档紧随其后（jpg+rar 多形体），
// 或给压缩档加一段垃圾前缀，或「完整媒体 + 追加压缩档到文件尾」（大视频常见：
// mp4 本体完整可播，zip/rar 接在后面，见 embedded_tail_offset）。文件头嗅探只能
// 看偏移 0，识别不了这三种。媒体/未知头的文件因此做两类大窗口扫描：
// 前缀窗找长魔数（≥6 字节，误报率可忽略；gzip/bzip2/lz4/zstd 魔数太短不进嵌入表），
// 尾部窗找「追加到文件尾」的长魔数；zip 习惯从尾部读，用 EOCD 反推起点
// （拖尾容忍见 ZIP_TRAIL_LIMIT：mp4+zip+rar 三连体里 zip 的 EOCD 不在文件尾）。
// 可执行后缀不启用（避免把 SFX 安装包/程序文件误判成压缩档）。

/// 前缀嵌入扫描上限（封面图/垫片之后紧跟压缩档，64MiB 足够覆盖真实场景）。
const EMBED_PREFIX_WINDOW: u64 = 64 * 1024 * 1024;
/// 尾部嵌入扫描上限：完整媒体后「追加到文件尾」的压缩档（rar/7z/xz 长魔数），
/// 超过窗体的超大追加档仍漏（已知可接受，zip 不受此限——EOCD 始终在尾部窗内）。
const EMBED_TAIL_WINDOW: u64 = 64 * 1024 * 1024;
/// zip EOCD 后允许的最大拖尾。实测案例：mp4+zip(1.exe)+rar+7z 四连体，
/// zip 的 EOCD 后还拖着 11KB 的 rar 与 7z（存档/说明），拖尾 ≤16MiB 的 EOCD 仍认。
const ZIP_TRAIL_LIMIT: u64 = 16 * 1024 * 1024;

/// 前缀窗内认的长魔数（长度 ≥6，误报率 ≈ 窗口/2^48）。
const EMBED_PREFIX_MAGICS: &[(&[u8], &str)] = &[
    (b"Rar!\x1a\x07\x00", "rar"),
    (b"Rar!\x1a\x07\x01\x00", "rar"),
    (b"7z\xbc\xaf\x27\x1c", "7z"),
    (b"\xfd7zXZ\x00", "xz"),
];

/// 不启用嵌入识别的后缀：可执行体（防 SFX 安装包误判）+ 文档/脚本类
/// （内容里可能真的包含魔数字符串，如压缩格式文档/测试夹具；误判会被移入失败目录）。
/// 注意 .pdf 不在此列：伪装成 pdf 的压缩包必须照常识别（用户明确要求）。
const EMBED_EXCLUDE_EXTS: &[&str] = &[
    ".exe", ".msi", ".dll", ".sys",
    ".txt", ".md", ".nfo", ".log", ".ini", ".json", ".url", ".html", ".htm",
    ".bat", ".cmd", ".py",
];

/// 前缀窗嵌入扫描：rar/7z/xz 长魔数出现在偏移 > 0 处即认。
fn sniff_embedded_prefix(path: &Path) -> Option<&'static str> {
    embedded_prefix_offset(path).map(|(_, kind)| kind)
}

/// 在 buf 中找 magic 的全部命中点（memchr 首字节预筛 + 全串验证）。
fn magic_hits(buf: &[u8], magic: &[u8]) -> Vec<usize> {
    let mut hits = Vec::new();
    if buf.len() < magic.len() {
        return hits;
    }
    let mut pos = 0usize;
    while let Some(rel) = memchr::memchr(magic[0], &buf[pos..]) {
        let p = pos + rel;
        if p + magic.len() > buf.len() {
            break; // 剩余长度装不下整条魔数，后续命中也不可能
        }
        if &buf[p..p + magic.len()] == magic {
            hits.push(p);
        }
        pos = p + 1;
    }
    hits
}

/// 前缀窗找第一个长魔数命中，返回 (偏移, kind)。
fn embedded_prefix_offset(path: &Path) -> Option<(u64, &'static str)> {
    let file = std::fs::File::open(path).ok()?;
    let win = file.metadata().ok()?.len().min(EMBED_PREFIX_WINDOW);
    let mut buf = Vec::new();
    file.take(win).read_to_end(&mut buf).ok()?;
    EMBED_PREFIX_MAGICS
        .iter()
        .find_map(|(magic, kind)| magic_hits(&buf, magic).first().map(|i| (*i as u64, *kind)))
}

/// 尾部窗找位置最靠前的长魔数命中（完整媒体后追加到文件尾的压缩档），返回 (偏移, kind)。
/// 命中必须过结构校验：视频数据里会出现巧合的魔数字节串（实测 12.6GB mp4 尾部
/// 就有假的 rar4 魔数），不校验会被当成压缩包、源文件误移入失败目录。
/// rar5 校验头类型（主头/加密头）+头长；rar4 校验头类型字节 + 头长；
/// 7z 校验 StartHeaderCRC32；xz 校验流标志 CRC32（公式均对着真实档案实测）。
fn embedded_tail_offset(path: &Path) -> Option<(u64, &'static str)> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(EMBED_TAIL_WINDOW);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.take(len - start).read_to_end(&mut buf).ok()?;
    let mut best: Option<(usize, &'static str)> = None;
    for (magic, kind) in EMBED_PREFIX_MAGICS {
        for p in magic_hits(&buf, magic) {
            let ok = match *kind {
                "7z" => buf.get(p + 6).copied() == Some(0x00) && start_crc32_ok(&buf, p),
                "xz" => xz_flags_crc_ok(&buf, p),
                "rar" if magic.len() == 8 => rar5_head_ok(&buf, p), // Rar!\x1a\x07\x01\x00
                "rar" => rar4_head_ok(&buf, p),                     // Rar!\x1a\x07\x00
                _ => true,
            };
            if ok && best.is_none_or(|(bp, _)| p < bp) {
                best = Some((p, *kind));
            }
        }
    }
    best.map(|(p, k)| (start + p as u64, k))
}

/// IEEE CRC32（zlib 同款多项式 0xEDB88320），用于 7z/xz 头校验。
fn crc32_ieee(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// rar5 vint：7 位/字节，小端序，最高位续行。
fn read_rar5_vint(buf: &[u8], mut pos: usize) -> Option<u64> {
    let mut v: u64 = 0;
    let mut shift = 0u32;
    while shift < 64 {
        let b = *buf.get(pos)? as u64;
        pos += 1;
        v |= (b & 0x7F) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
        shift += 7;
    }
    None
}

/// rar5 头：head_size vint 合理（含自身至少装下 size+type+flags），
/// head_type 必须是主头(1)或加密头(4)（档案首块只可能是这两者）。
fn rar5_head_ok(buf: &[u8], p: usize) -> bool {
    let Some(head_size) = read_rar5_vint(buf, p + 12) else {
        return false;
    };
    if !(5..=1_000_000).contains(&head_size) {
        return false;
    }
    let size_vint_len = rar5_vint_len(buf, p + 12);
    let Some(head_type) = read_rar5_vint(buf, p + 12 + size_vint_len) else {
        return false;
    };
    matches!(head_type, 1 | 4)
}

fn rar5_vint_len(buf: &[u8], pos: usize) -> usize {
    let mut n = 0usize;
    while n < 10 {
        match buf.get(pos + n) {
            Some(b) if b & 0x80 != 0 => n += 1,
            _ => return n + 1,
        }
    }
    10
}

/// rar4 头：HEAD_TYPE 必须落在归档块类型区间，HEAD_SIZE 合理。
fn rar4_head_ok(buf: &[u8], p: usize) -> bool {
    let (Some(&b11), Some(&b12)) = (buf.get(p + 11), buf.get(p + 12)) else {
        return false;
    };
    let head_size = u16::from_le_bytes([b11, b12]) as usize;
    (7..=1_000_000).contains(&head_size) && matches!(buf.get(p + 9), Some(0x73..=0x7b))
}

/// 7z：版本主字节须为 0，StartHeaderCRC32（[p+8..p+12)）须等于
/// NextHeaderOffset+NextHeaderSize+NextHeaderCRC 共 20 字节的 CRC32。
fn start_crc32_ok(buf: &[u8], p: usize) -> bool {
    let Some(stored) = buf.get(p + 8..p + 12) else {
        return false;
    };
    let Some(data) = buf.get(p + 12..p + 32) else {
        return false;
    };
    let stored = u32::from_le_bytes([stored[0], stored[1], stored[2], stored[3]]);
    crc32_ieee(data) == stored
}

/// xz：6 字节魔数 + 2 字节流标志，[p+8..p+12) 须等于流标志的 CRC32。
fn xz_flags_crc_ok(buf: &[u8], p: usize) -> bool {
    let Some(flags) = buf.get(p + 6..p + 8) else {
        return false;
    };
    let Some(stored) = buf.get(p + 8..p + 12) else {
        return false;
    };
    let stored = u32::from_le_bytes([stored[0], stored[1], stored[2], stored[3]]);
    crc32_ieee(flags) == stored
}

/// 解析尾部 EOCD 并反推 zip 起点，返回 (zip 起始偏移, EOCD 偏移)。
/// 从文件尾倒序找 PK\x05\x06：拖尾 ≤ ZIP_TRAIL_LIMIT（三连体：zip 后还追加着
/// 小体积 rar/7z），且 eocd_pos − cd_offset − cd_size 反推的起点处必须真是
/// zip 本地头（防视频数据里巧合的 EOCD 字样误判成 zip）。
/// （zip 偏移以 zip 起始为 0，公式天然兼容带前缀/SFX。注释最长 64KiB，
/// 窗口 = 拖尾上限 + EOCD 头，注释超限的 EOCD 罩不住，属已知边界。）
fn find_zip_eocd(path: &Path) -> Option<(u64, u64)> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(ZIP_TRAIL_LIMIT + 22);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.take(len - start).read_to_end(&mut buf).ok()?;
    if buf.len() < 22 {
        return None;
    }
    // 从文件尾倒序找 PK\x05\x06（memrchr 首字节预筛）
    let mut search_end = buf.len();
    while search_end >= 4 {
        let Some(rel) = memchr::memrchr(b'P', &buf[..search_end]) else {
            break;
        };
        search_end = rel;
        if rel + 22 > buf.len() || &buf[rel..rel + 4] != b"PK\x05\x06" {
            continue;
        }
        let i = rel;
        let clen = u16::from_le_bytes([buf[i + 20], buf[i + 21]]) as usize;
        if i + 22 + clen > buf.len() {
            continue; // 注释越出文件尾（截窗口的候选放弃）
        }
        let eocd_pos = start + i as u64;
        let eocd_end = eocd_pos + 22 + clen as u64;
        if len - eocd_end > ZIP_TRAIL_LIMIT {
            continue; // 拖尾过大：不是「zip + 小尾巴」形态
        }
        let cd_size = u32::from_le_bytes([buf[i + 12], buf[i + 13], buf[i + 14], buf[i + 15]]) as u64;
        let cd_offset = u32::from_le_bytes([buf[i + 16], buf[i + 17], buf[i + 18], buf[i + 19]]) as u64;
        let Some(zip_start) = eocd_pos.checked_sub(cd_offset).and_then(|v| v.checked_sub(cd_size)) else {
            continue;
        };
        let mut head_file = std::fs::File::open(path).ok()?;
        if head_file.seek(SeekFrom::Start(zip_start)).is_err() {
            continue;
        }
        let mut head = [0u8; 4];
        if head_file.read_exact(&mut head).is_err() {
            continue;
        }
        if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") || head.starts_with(b"PK\x07\x08") {
            return Some((zip_start, eocd_pos));
        }
    }
    None
}

/// 尾部 EOCD 判定 zip。
fn sniff_embedded_zip_tail(path: &Path) -> Option<&'static str> {
    find_zip_eocd(path).map(|_| "zip")
}

/// zip 本地文件头链的最小连续环数（EOCD 损坏/缺失时靠链识别）。
/// 单环命中 2^-32，连续两环以上 + 方法字段校验后误报 ≈ 0。
const ZIP_CHAIN_MIN: u32 = 2;
/// 链识别允许的压缩方式（0=store 8=deflate 9=deflate64 12=bzip2 14=lzma
/// 95=xz 96=jpeg 97=ppmd 98/99=aes；发布组 zip 基本只用到 0/8/99）。
const ZIP_METHODS: &[u16] = &[0, 8, 9, 12, 14, 95, 96, 97, 98, 99];

/// 解析 zip 本地文件头；数据描述符（flag bit3）时 csize 不可信 → None。
fn read_lfh_at(f: &mut std::fs::File, off: u64) -> Option<(u16, u16, u64)> {
    f.seek(SeekFrom::Start(off)).ok()?;
    let mut h = [0u8; 30];
    f.read_exact(&mut h).ok()?;
    if &h[0..4] != b"PK\x03\x04" {
        return None;
    }
    let flag = u16::from_le_bytes([h[6], h[7]]);
    let method = u16::from_le_bytes([h[8], h[9]]);
    if flag & 0x08 != 0 || !ZIP_METHODS.contains(&method) {
        return None;
    }
    let nlen = u16::from_le_bytes([h[26], h[27]]);
    let elen = u16::from_le_bytes([h[28], h[29]]);
    if nlen == 0 || nlen > 512 {
        return None;
    }
    let csize = u32::from_le_bytes([h[18], h[19], h[20], h[21]]) as u64;
    Some((nlen, elen, csize))
}

/// 从 seed 起沿本地头链走：每环的下一偏移必须精确落在 30+nlen+elen+csize 处，
/// 命中下一个本地头继续、命中中央目录头（PK\x01\x02）即坐实；
/// 连续 ZIP_CHAIN_MIN 环本地头同样坐实（大条目 zip 提前收链）。
fn follow_zip_chain(f: &mut std::fs::File, len: u64, seed: u64) -> bool {
    let mut cur = seed;
    let mut links = 0u32;
    loop {
        let lfh = read_lfh_at(f, cur);
        let Some((nlen, elen, csize)) = lfh else {
            return false;
        };
        let Some(next) = cur
            .checked_add(30)
            .and_then(|v| v.checked_add(nlen as u64))
            .and_then(|v| v.checked_add(elen as u64))
            .and_then(|v| v.checked_add(csize))
        else {
            return false;
        };
        if next + 4 > len {
            return false;
        }
        let mut sig = [0u8; 4];
        if f.seek(SeekFrom::Start(next)).is_err() || f.read_exact(&mut sig).is_err() {
            return false;
        }
        links += 1;
        if &sig == b"PK\x01\x02" {
            return links >= 1; // 本地头→中央目录：铁证
        }
        if &sig != b"PK\x03\x04" {
            return false;
        }
        if links >= ZIP_CHAIN_MIN {
            return true;
        }
        cur = next;
    }
}

/// 全文件流式扫 zip 本地头链（memchr 首字节预筛 + 链式跟随）。
/// EOCD 校验全部失败时的兜底——发布组会故意损坏 EOCD 防网盘内容扫描
/// （实测荒野独居 151/152：zip 实体完好但 EOCD 区被破坏，7z 靠本地头重建）。
fn find_zip_lfh_chain(path: &Path) -> Option<u64> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut pos = 0u64;
    let mut buf = Vec::new();
    while pos < len {
        let want = (len - pos).min(64 << 20) as usize;
        if f.seek(SeekFrom::Start(pos)).is_err() {
            return None;
        }
        buf.clear();
        if (&mut f).take(want as u64).read_to_end(&mut buf).is_err() {
            return None;
        }
        let mut i = 0usize;
        while let Some(rel) = memchr::memchr(b'P', &buf[i..]) {
            let p = i + rel;
            if p + 4 > buf.len() {
                break;
            }
            if &buf[p..p + 4] == b"PK\x03\x04" {
                let seed = pos + p as u64;
                if follow_zip_chain(&mut f, len, seed) {
                    return Some(seed);
                }
            }
            i = p + 1;
        }
        if buf.len() < want || buf.len() <= 4 {
            break;
        }
        pos += buf.len() as u64 - 4; // 重叠 4 字节防跨界漏（尾部余量 ≤4 时收链防死循环）
    }
    None
}

/// 链识别结果单槽缓存：embedded_kind 判定与 embedded_offset 取偏移各调一次，
/// 12.6GB 的媒体文件免二次全文件扫描。键 = 路径 + 文件大小。
static ZIP_CHAIN_CACHE: std::sync::Mutex<Option<(std::path::PathBuf, u64, Option<u64>)>> =
    std::sync::Mutex::new(None);

fn zip_lfh_chain_cached(path: &Path) -> Option<u64> {
    let len = std::fs::metadata(path).ok()?.len();
    {
        let cache = ZIP_CHAIN_CACHE.lock().ok()?;
        if let Some((p, l, r)) = cache.as_ref() {
            if p == path && *l == len {
                return *r;
            }
        }
    }
    let result = find_zip_lfh_chain(path);
    if let Ok(mut cache) = ZIP_CHAIN_CACHE.lock() {
        *cache = Some((path.to_path_buf(), len, result));
    }
    result
}

/// 嵌入压缩档的起始偏移（用于雕出；rar 前缀命中时 7z/unrar 原生支持前缀偏移，不雕）：
/// - zip：EOCD 反推起点（拖尾容忍 + 起点魔数验证，见 find_zip_eocd）；
/// - 7z/xz：前缀窗首个命中，退而求其次尾部窗命中（大视频后追加到文件尾）；
/// - rar：仅尾部窗命中才需雕出（前缀命中走原生）；前缀窗有 rar 时尾部 rar 不再雕。
pub(crate) fn embedded_offset(path: &Path, kind: &str) -> Option<u64> {
    match kind {
        // EOCD 优先；发布组会故意损坏 EOCD 防网盘扫描（实测荒野独居 151/152：
        // zip 实体完好、EOCD 区被破坏，7z 靠本地头链重建），兜底走链识别。
        "zip" => find_zip_eocd(path)
            .map(|(start, _)| start)
            .or_else(|| zip_lfh_chain_cached(path)),
        "7z" | "xz" => embedded_prefix_offset(path)
            .filter(|(off, k)| *k == kind && *off > 0)
            .map(|(off, _)| off)
            .or_else(|| {
                embedded_tail_offset(path)
                    .filter(|(_, k)| *k == kind)
                    .map(|(off, _)| off)
            }),
        "rar" => {
            if embedded_prefix_offset(path).is_some_and(|(_, k)| k == "rar") {
                return None;
            }
            embedded_tail_offset(path)
                .filter(|(_, k)| *k == "rar")
                .map(|(off, _)| off)
        }
        _ => None,
    }
}

/// 只读文件头 600 字节的魔数判定（不含媒体判定与嵌入扫描）。
/// 用于区分「正常压缩档」与「多形体/前缀复合包」（解包层决定是否需要雕出）。
pub(crate) fn sniff_head(path: &Path) -> Option<&'static str> {
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
    if head.starts_with(b"MZ") {
        // SFX 自解压包：PE overlay 带校验通过的压缩档结构（游戏启动 exe 没有，不误伤）
        if let Some(kind) = sniff_sfx(path) {
            return Some(kind);
        }
    }
    None
}

/// SFX 自解压包探测：PE 文件（MZ）按节表算出 overlay（最后一个节原始数据结束处），
/// overlay 起点或紧随其后 1MiB 窗内须有校验通过的压缩档结构（7z StartHeaderCRC /
/// rar5 / rar4 头；zip SFX 惯例本地头恰在 overlay 起点）。判定通过后按正常压缩档
/// 处理——7z 原生能读 SFX，无需雕出。游戏启动 exe 的 overlay 不含这些结构
/// （≈2^-56 巧合），不误伤；改后缀的 SFX 走嵌入扫描，不经过本路径。
fn sniff_sfx(path: &Path) -> Option<&'static str> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = vec![0u8; 4096];
    let n = f.read(&mut head).ok()?;
    head.truncate(n);
    let pe_off = u32::from_le_bytes(head.get(0x3C..0x40)?.try_into().ok()?) as usize;
    let pe = head.get(pe_off..pe_off + 24)?;
    if &pe[0..4] != b"PE\x00\x00" {
        return None;
    }
    let nsec = u16::from_le_bytes([pe[6], pe[7]]) as usize;
    let opt_size = u16::from_le_bytes([pe[20], pe[21]]) as usize;
    let sec_tbl = pe_off + 24 + opt_size;
    let mut overlay = 0u64;
    for i in 0..nsec {
        let s = head.get(sec_tbl + i * 40..sec_tbl + i * 40 + 24)?;
        let raw_size = u32::from_le_bytes([s[16], s[17], s[18], s[19]]) as u64;
        let raw_ptr = u32::from_le_bytes([s[20], s[21], s[22], s[23]]) as u64;
        overlay = overlay.max(raw_ptr + raw_size);
    }
    if overlay == 0 {
        return None;
    }
    let mut f2 = std::fs::File::open(path).ok()?;
    f2.seek(SeekFrom::Start(overlay)).ok()?;
    let mut buf = Vec::new();
    f2.take(1024 * 1024).read_to_end(&mut buf).ok()?;
    if buf.starts_with(b"PK\x03\x04") {
        return Some("zip"); // zip SFX：本地头恰在 overlay 起点
    }
    for (magic, kind) in EMBED_PREFIX_MAGICS {
        for p in magic_hits(&buf, magic) {
            let ok = match *kind {
                "7z" => buf.get(p + 6).copied() == Some(0x00) && start_crc32_ok(&buf, p),
                "rar" if magic.len() == 8 => rar5_head_ok(&buf, p),
                "rar" => rar4_head_ok(&buf, p),
                _ => true,
            };
            if ok {
                return Some(kind);
            }
        }
    }
    None
}

/// 媒体/未知头文件是否其实是伪装包（封面图+压缩档 / 垃圾前缀+压缩档 / 媒体后追加压缩档）。
/// 已知缺口：封面+zip+rar 三连多形体只解得出 zip（EOCD 反推优先于尾部 rar，zip 后的
/// 小尾巴不解——内容本体在 zip 里，可接受）；尾部窗外（>64MiB）的超大追加档不参与。
fn embedded_kind(path: &Path) -> Option<&'static str> {
    let name = file_name_string(path);
    let lower = ascii_lower(name.as_bytes());
    if EMBED_EXCLUDE_EXTS
        .iter()
        .any(|e| lower.ends_with(e.as_bytes()))
    {
        return None;
    }
    sniff_embedded_zip_tail(path)
        .or_else(|| sniff_embedded_prefix(path))
        .or_else(|| embedded_tail_offset(path).map(|(_, k)| k))
        .or_else(|| zip_lfh_chain_cached(path).map(|_| "zip"))
}

/// 读文件头识别，返回 kind 字符串（"zip"/"rar"/"7z"/"gzip"/"bzip2"/"xz"/"tar"/"zstd"/"lz4"/"media"）。
/// 文件头无魔数但可能是多形体伪装包（封面图+压缩档）或垃圾前缀包时，做嵌入识别。
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
    if head.starts_with(b"MZ") {
        // SFX 自解压包（PE overlay 带压缩档结构）：按压缩档识别，7z 原生可读
        if let Some(kind) = sniff_sfx(path) {
            return Some(kind);
        }
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
        // 真封面图返回 "media"；多形体（封面+压缩档）返回压缩档 kind
        return embedded_kind(path).or(Some("media"));
    }
    // 未知头：可能是带垃圾前缀的压缩档
    embedded_kind(path)
}

/// 字符串版分类：返回 "product"/"media"/kind/None。
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

/// 路径后缀（suffix）：最后一个 `.` 起的部分（含点）；无前缀点或无点返回 ""。
pub(crate) fn suffix_with_dot(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[i..],
        _ => "",
    }
}

/// 路径主名（stem）：去掉最后一个后缀；点开头的名字整体视为 stem。
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
/// 保住 "idx>1 / idx==1" 的判定语义）。
fn parse_idx(digits: &str) -> u32 {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        0
    } else {
        trimmed.parse::<u32>().unwrap_or(u32::MAX)
    }
}

/// ASCII 大小写折叠（不改字节长度）。卷名/复合后缀模式全是 ASCII：
/// 折叠后对 ASCII 的匹配结果与 lower() 语义一致，且 UTF-8 多字节序列
/// 不可能匹配 ASCII 模式字节，所有派生索引都落在字符边界上。
pub(crate) fn ascii_lower(bytes: &[u8]) -> Vec<u8> {
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

/// 去掉尾部重命名短标记（1-4 个非 ASCII 且非点号字节，如「删」「旧」——
/// 百度/夸克下载或用户手改卷名时常见），返回去掉后的长度；无标记返回原长。
fn strip_marker(lower: &[u8]) -> usize {
    let mut n = 0;
    while n < 4 && n < lower.len() {
        let b = lower[lower.len() - 1 - n];
        if b == b'.' || b < 0x80 {
            break;
        }
        n += 1;
    }
    lower.len() - n
}

/// 匹配数字卷名尾部 `sep . 3位数字 [标记]`，或（仅 .7z）夸克形态 `.7z-<hex 6-16> . 3位数字 [标记]`。
/// 返回 (stem 字节长度, idx)。匹配一律在 ASCII 折叠后的字节上进行。
fn match_num_vol_tail(lower: &[u8], sep: &str) -> Option<(usize, u32)> {
    let end = strip_marker(lower);
    if end < 4 + 3 || !lower[end - 3..end].iter().all(|c| c.is_ascii_digit()) || lower[end - 4] != b'.'
    {
        return None;
    }
    let digits = &lower[end - 3..end];
    let head_end = end - 4; // '.' 之前
    let sep_b = sep.as_bytes();
    // 形态 A：stem + sep + "." + 数字（head_end 前正好 sep）
    if head_end >= sep_b.len() && &lower[head_end - sep_b.len()..head_end] == sep_b {
        return Some((head_end - sep_b.len(), parse_idx(std::str::from_utf8(digits).unwrap_or(""))));
    }
    // 形态 B（夸克分卷，仅 .7z）：stem + ".7z-" + <hex 6-16> + "." + 数字
    if *sep == *".7z" {
        let before = &lower[..head_end];
        if let Some(pos) = rfind_subslice(before, b".7z-") {
            let hex = &before[pos + 4..];
            if (6..=16).contains(&hex.len())
                && hex.iter().all(|c| c.is_ascii_hexdigit())
            {
                return Some((pos, parse_idx(std::str::from_utf8(digits).unwrap_or(""))));
            }
        }
    }
    None
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
    // ^(.*)\.(?:7z|zip|rar)\.\d{3}$（锚定结尾；容忍「删」类尾标与夸克 .7z-<hash> 形态）
    for (sep, _) in NUMERIC_VOL_SEPS {
        if let Some((stem_len, _)) = match_num_vol_tail(&lower, sep) {
            return name[..stem_len].to_string();
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
    // ^(.*)\.7z\.(\d{3})$ | ^(.*)\.zip\.(\d{3})$（仅这两族有数字卷；
    // 容忍「删」类尾标与夸克 .7z-<hash> 形态）
    for (sep, family) in [(".7z", VolFamily::SevenZNum), (".zip", VolFamily::ZipNum)] {
        if let Some((stem_len, idx)) = match_num_vol_tail(&n, sep) {
            return Some(VolumeRef {
                stem: String::from_utf8_lossy(&n[..stem_len]).into_owned(),
                family,
                idx,
            });
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

    // ---------- 嵌入伪装识别（多形体/垃圾前缀） ----------

    /// 最小 JPEG：JFIF 头 + EOI。封面图部分任意，魔数在 EOI 之后。
    fn minimal_jpg() -> Vec<u8> {
        let mut jpg = bytes_from_hex("ffd8ffe000104a46494600010101000100010000");
        jpg.extend_from_slice(b"\xff\xd9");
        jpg
    }

    fn bytes_from_hex(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn sniff_polyglot_cover_image_with_appended_archive() {
        let tmp = tempdir().unwrap();
        // jpg + rar（退魔士姬子 PC15732.jpg 的真实结构）
        let mut jpg_rar = minimal_jpg();
        jpg_rar.extend_from_slice(b"Rar!\x1a\x07\x01\x00rest");
        let p = write_file(tmp.path(), "cover.jpg", &jpg_rar);
        assert_eq!(sniff(&p), Some("rar"));

        // jpg + 7z
        let mut jpg_7z = minimal_jpg();
        jpg_7z.extend_from_slice(b"7z\xbc\xaf\x27\x1c\x00");
        let p = write_file(tmp.path(), "cover2.png", &jpg_7z);
        assert_eq!(sniff(&p), Some("7z"));

        // jpg + xz
        let mut jpg_xz = minimal_jpg();
        jpg_xz.extend_from_slice(b"\xfd7zXZ\x00abc");
        let p = write_file(tmp.path(), "cover3.gif", &jpg_xz);
        assert_eq!(sniff(&p), Some("xz"));

        // 纯封面图（无追加内容）→ 仍是媒体
        let p = write_file(tmp.path(), "plain.jpg", &minimal_jpg());
        assert_eq!(sniff(&p), Some("media"));

        // 未知头 + 嵌入 rar → rar（垃圾前缀包）
        let mut junk_rar = vec![0u8; 33];
        junk_rar.extend_from_slice(b"Rar!\x1a\x07\x00xx");
        let p = write_file(tmp.path(), "mystery.bin", &junk_rar);
        assert_eq!(sniff(&p), Some("rar"));
    }

    #[test]
    fn sniff_polyglot_zip_detected_by_eocd_at_tail() {
        let tmp = tempdir().unwrap();
        // 手工拼一个最小 zip：local file header + central directory + EOCD（无注释）
        let local = b"PK\x03\x04\x14\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x00\x00\x00a";
        let cd = b"PK\x01\x02\x14\x00\x14\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        let mut eocd = b"PK\x05\x06\x00\x00\x00\x00\x01\x00\x01\x00".to_vec();
        eocd.extend_from_slice(&(cd.len() as u32).to_le_bytes());
        eocd.extend_from_slice(&(local.len() as u32).to_le_bytes()); // cd_offset = 本地头之后
        eocd.extend_from_slice(b"\x00\x00"); // 注释长度 0
        let mut poly = minimal_jpg();
        poly.extend_from_slice(local);
        poly.extend_from_slice(cd);
        poly.extend_from_slice(&eocd);
        let p = write_file(tmp.path(), "cover.jpg", &poly);
        assert_eq!(sniff(&p), Some("zip"));
    }

    #[test]
    fn embedded_offset_locates_zip_after_mp4_prefix() {
        // mp4 头桩 + 追加 zip：embedded_offset 须给出 zip 起点（游戏分享复合包结构）
        let tmp = tempdir().unwrap();
        let local = b"PK\x03\x04\x14\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x00\x00\x00a";
        let cd = b"PK\x01\x02\x14\x00\x14\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        let mut eocd = b"PK\x05\x06\x00\x00\x00\x00\x01\x00\x01\x00".to_vec();
        eocd.extend_from_slice(&(cd.len() as u32).to_le_bytes());
        eocd.extend_from_slice(&(local.len() as u32).to_le_bytes()); // cd_offset = 本地头之后
        eocd.extend_from_slice(b"\x00\x00");
        let mut mp4_stub = bytes_from_hex("000000206674797069736f6d0000020069736f6d69736f32617663316d703431");
        mp4_stub.extend_from_slice(&[0u8; 77]);
        let zip_start = mp4_stub.len() as u64;
        let mut poly = mp4_stub.clone();
        poly.extend_from_slice(local);
        poly.extend_from_slice(cd);
        poly.extend_from_slice(&eocd);
        let p = write_file(tmp.path(), "game.mp4", &poly);
        assert_eq!(sniff(&p), Some("zip"), "mp4 前缀 + zip 须识别为 zip");
        assert_eq!(embedded_offset(&p, "zip"), Some(zip_start));
        assert_eq!(sniff_head(&p), None, "mp4 头无压缩档魔数");
    }

    /// 手工最小 zip（一个空文件项 a），供复合包测试拼接。
    fn handmade_zip() -> Vec<u8> {
        let local = b"PK\x03\x04\x14\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x00\x00\x00a";
        let cd = b"PK\x01\x02\x14\x00\x14\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        let mut eocd = b"PK\x05\x06\x00\x00\x00\x00\x01\x00\x01\x00".to_vec();
        eocd.extend_from_slice(&(cd.len() as u32).to_le_bytes());
        eocd.extend_from_slice(&(local.len() as u32).to_le_bytes());
        eocd.extend_from_slice(b"\x00\x00");
        let mut zip = local.to_vec();
        zip.extend_from_slice(cd);
        zip.extend_from_slice(&eocd);
        zip
    }

    #[test]
    fn zip_eocd_allows_small_trailing_junk() {
        // 二小姐.mp4 真实结构：完整 mp4 + zip(1.exe) + 尾部再拖小 rar/7z。
        // EOCD 不再要求恰好到文件尾（拖尾 ≤ ZIP_TRAIL_LIMIT），起点仍须 PK 验证。
        let tmp = tempdir().unwrap();
        let mut mp4_stub = bytes_from_hex("000000206674797069736f6d0000020069736f6d69736f32617663316d703431");
        mp4_stub.extend_from_slice(&[0u8; 77]);
        let zip_start = mp4_stub.len() as u64;
        let mut poly = mp4_stub;
        poly.extend_from_slice(&handmade_zip());
        poly.extend_from_slice(&[0u8; 64]); // 尾部补零（不得引入任何长魔数，前缀/尾窗的无校验路径会抢先）
        let p = write_file(tmp.path(), "二小姐.mp4", &poly);
        assert_eq!(sniff(&p), Some("zip"), "EOCD 带小拖尾仍须识别为 zip");
        assert_eq!(embedded_offset(&p, "zip"), Some(zip_start));
    }

    #[test]
    fn fake_eocd_in_video_data_rejected() {
        // 视频数据里的巧合 PK\x05\x06 + 貌似合理的 cd 字段不得误判为 zip：
        // EOCD 起点验证失败，且其后没有可跟随的本地头链。
        let tmp = tempdir().unwrap();
        let mut mp4_stub = bytes_from_hex("000000206674797069736f6d0000020069736f6d69736f32617663316d703431");
        mp4_stub.extend_from_slice(&[0u8; 77]);
        let mut poly = mp4_stub;
        poly.extend_from_slice(&[0u8; (ZIP_TRAIL_LIMIT + 4096) as usize]);
        // 拼一个字段自洽但指向全零区的假 EOCD（cd_offset/cd_size 指向无 zip 头区域）
        let mut fake = b"PK\x05\x06\x00\x00\x00\x00\x01\x00\x01\x00".to_vec();
        fake.extend_from_slice(&64u32.to_le_bytes()); // cd_size
        fake.extend_from_slice(&(poly.len() as u32 - 64).to_le_bytes()); // cd_offset → poly 中部零区
        fake.extend_from_slice(b"\x00\x00");
        poly.extend_from_slice(&fake);
        let p = write_file(tmp.path(), "game.mp4", &poly);
        assert_eq!(sniff(&p), Some("media"), "假 EOCD 不得误判为 zip");
        assert_eq!(embedded_offset(&p, "zip"), None);
    }

    #[test]
    fn zip_broken_eocd_found_by_lfh_chain() {
        // EOCD 被损坏（防网盘扫描手法，实测荒野独居 151/152）：真实 zip 走本地头链识别。
        let sevenz = std::path::Path::new(r"C:\Program Files\7-Zip\7z.exe");
        if !sevenz.is_file() {
            eprintln!("SKIP：本机缺少 7z.exe");
            return;
        }
        let tmp = tempdir().unwrap();
        let work = tempdir().unwrap();
        std::fs::write(work.path().join("game.txt"), "链识别").unwrap();
        std::fs::write(work.path().join("data.bin"), &[7u8; 4096]).unwrap();
        let st = std::process::Command::new(sevenz)
            .args(["a", "-y", "-tzip", "real.zip", "game.txt", "data.bin"])
            .current_dir(work.path())
            .status()
            .unwrap();
        assert!(st.success());
        let mut zip = std::fs::read(work.path().join("real.zip")).unwrap();
        // 损坏 EOCD：把最后一个 PK\x05\x06 抹成 0（链识别是唯一生路）
        let eocd_at = zip.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
        zip[eocd_at..eocd_at + 4].fill(0);
        let mut mp4_stub = bytes_from_hex("000000206674797069736f6d0000020069736f6d69736f32617663316d703431");
        mp4_stub.extend_from_slice(&[0u8; 77]);
        let zip_start = mp4_stub.len() as u64;
        let mut poly = mp4_stub;
        poly.extend_from_slice(&zip);
        poly.extend_from_slice(&[0u8; 64]); // 尾部纯零（不得引入长魔数，无校验的前缀/尾窗路径会抢先）
        let p = write_file(tmp.path(), "game.mp4", &poly);
        assert_eq!(sniff(&p), Some("zip"), "EOCD 损坏的真实 zip 须靠本地头链识别");
        assert_eq!(embedded_offset(&p, "zip"), Some(zip_start));
    }

    /// 结构校验通过的尾部 rar5（主头：head_size=5、type=1、flags=0）。
    fn valid_rar5_tail() -> Vec<u8> {
        let mut v = b"Rar!\x1a\x07\x01\x00".to_vec();
        v.extend_from_slice(&[0u8; 4]); // HEAD_CRC（校验器不验它）
        v.extend_from_slice(&[0x05, 0x01, 0x00]); // head_size=5, type=主头, flags=0
        v.extend_from_slice(&[0u8; 32]);
        v
    }

    #[test]
    fn tail_magic_validates_archive_structure() {
        // 大媒体（>前缀窗）尾部命中必须过结构校验：纯魔数（视频数据里会出现）不成立。
        let tmp = tempdir().unwrap();
        let big = (EMBED_PREFIX_WINDOW + (8 << 20)) as usize; // 前缀窗之外，逼走尾部窗路径
        let head = {
            let mut h = bytes_from_hex("000000206674797069736f6d0000020069736f6d69736f32617663316d703431");
            h.resize(big, 0);
            h
        };
        // 假 rar5 魔数（无有效头）→ 仍是媒体
        let mut fake = head.clone();
        fake.extend_from_slice(b"Rar!\x1a\x07\x01\x00");
        fake.extend_from_slice(&[0xFF; 64]); // head_size vint 越界 → 校验失败
        let p = write_file(tmp.path(), "movie.mp4", &fake);
        assert_eq!(sniff(&p), Some("media"), "假 rar5 魔数不得识别为 rar");
        // 真结构 rar5 → 识别（尾部窗命中、前缀窗未命中）
        let mut real = head;
        real.extend_from_slice(&valid_rar5_tail());
        let p = write_file(tmp.path(), "game.mp4", &real);
        assert_eq!(sniff(&p), Some("rar"), "尾部结构合法的 rar5 须识别");
        assert_eq!(embedded_offset(&p, "rar"), Some(big as u64));
        // 前缀窗命中 rar5 → 原生路径不雕（保持历史行为）
        let mut prefixed = bytes_from_hex("000000206674797069736f6d0000020069736f6d69736f32617663316d703431");
        prefixed.extend_from_slice(&[0u8; 77]);
        prefixed.extend_from_slice(&valid_rar5_tail());
        let p = write_file(tmp.path(), "cover.mp4", &prefixed);
        assert_eq!(sniff(&p), Some("rar"));
        assert_eq!(embedded_offset(&p, "rar"), None, "前缀 rar 走原生不雕");
    }

    #[test]
    fn tail_magic_validates_7z_and_xz_headers() {
        let tmp = tempdir().unwrap();
        let big = (EMBED_PREFIX_WINDOW + (8 << 20)) as usize;
        let mut head = bytes_from_hex("000000206674797069736f6d0000020069736f6d69736f32617663316d703431");
        head.resize(big, 0);
        // 假 7z：版本字节非 0 / CRC 不符 → 媒体
        let mut fake7z = head.clone();
        fake7z.extend_from_slice(b"7z\xbc\xaf\x27\x1c\x09\x09\x00\x00\x00\x00");
        fake7z.extend_from_slice(&[0u8; 32]);
        let p = write_file(tmp.path(), "a.mp4", &fake7z);
        assert_eq!(sniff(&p), Some("media"), "假 7z 魔数不得识别");
        // 真 7z 签名头（StartHeaderCRC 对 20 字节全零计算）
        let mut real7z = head;
        let zeros20 = [0u8; 20];
        let crc = crc32_ieee(&zeros20);
        real7z.extend_from_slice(b"7z\xbc\xaf\x27\x1c\x00\x04");
        real7z.extend_from_slice(&crc.to_le_bytes());
        real7z.extend_from_slice(&zeros20);
        let p = write_file(tmp.path(), "b.mp4", &real7z);
        assert_eq!(sniff(&p), Some("7z"), "尾部结构合法的 7z 须识别");
        assert_eq!(embedded_offset(&p, "7z"), Some(big as u64));
        // 假 xz（标志 CRC 不符）→ 媒体；真 xz → xz
        let mut fakexz = real7z.clone(); // 复用大前缀
        fakexz.truncate(big);
        fakexz.extend_from_slice(b"\xfd7zXZ\x00\x00\x01\xDE\xAD\xBE\xEF");
        fakexz.extend_from_slice(&[0u8; 16]);
        let p = write_file(tmp.path(), "c.mp4", &fakexz);
        assert_eq!(sniff(&p), Some("media"), "假 xz 魔数不得识别");
        let mut realxz = bytes_from_hex("000000206674797069736f6d0000020069736f6d69736f32617663316d703431");
        realxz.resize(big, 0);
        let flags = [0x00u8, 0x01];
        let fcrc = crc32_ieee(&flags);
        realxz.extend_from_slice(b"\xfd7zXZ\x00");
        realxz.extend_from_slice(&flags);
        realxz.extend_from_slice(&fcrc.to_le_bytes());
        realxz.extend_from_slice(&[0u8; 16]);
        let p = write_file(tmp.path(), "d.mp4", &realxz);
        assert_eq!(sniff(&p), Some("xz"), "尾部结构合法的 xz 须识别");
    }

    #[test]
    fn sniff_embedded_skips_executable_suffixes() {
        let tmp = tempdir().unwrap();
        // setup.exe = 垃圾前缀 + rar 内容 → 不启用嵌入识别（防 SFX 安装包误判）
        let mut sfx = vec![0u8; 33];
        sfx.extend_from_slice(b"Rar!\x1a\x07\x01\x00xx");
        let p = write_file(tmp.path(), "setup.exe", &sfx);
        assert_eq!(sniff(&p), None);
        // 改个非可执行后缀即可识别
        let p = write_file(tmp.path(), "setup.bin", &sfx);
        assert_eq!(sniff(&p), Some("rar"));
        // 文档/脚本后缀同理禁用（内容可能真的包含魔数字符串，误判会被移入失败目录）
        for name in ["notes.txt", "readme.md", "fmt.log", "x.json", "run.bat"] {
            let p = write_file(tmp.path(), name, &sfx);
            assert_eq!(sniff(&p), None, "{name}");
        }
        // .pdf 不排除：伪装成 pdf 的压缩包必须照常识别
        let p = write_file(tmp.path(), "cover.pdf", &sfx);
        assert_eq!(sniff(&p), Some("rar"));
    }

    #[test]
    fn sfx_exe_with_archive_overlay_detected() {
        // 真实 SFX 结构（对齐 1.exe）：7z.sfx + 包裹 7z 分卷的 wrapped.7z。
        let sevenz = std::path::Path::new(r"C:\Program Files\7-Zip\7z.exe");
        let sfx_mod = sevenz.parent().unwrap().join("7z.sfx");
        if !sevenz.is_file() || !sfx_mod.is_file() {
            eprintln!("SKIP：本机缺少 7z.exe/7z.sfx");
            return;
        }
        let tmp = tempdir().unwrap();
        let work = tempdir().unwrap();
        // xorshift 伪随机填充（可压缩数据造不出多卷）
        let mut game = Vec::with_capacity(200 * 1024);
        let mut x = 0x9E3779B97F4A7C15u64;
        while game.len() < 200 * 1024 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            game.extend_from_slice(&x.to_le_bytes());
        }
        fs::write(work.path().join("game.bin"), &game).unwrap();
        let st = |args: &[&str]| {
            assert!(std::process::Command::new(sevenz)
                .args(args)
                .current_dir(work.path())
                .status()
                .unwrap()
                .success());
        };
        st(&["a", "-y", "-t7z", "-v64k", "inner.7z", "game.bin"]);
        let vols: Vec<String> = fs::read_dir(work.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("inner.7z.0"))
            .collect();
        assert!(vols.len() >= 2, "200KB @64KiB 卷应产生多个分卷");
        let mut wrap: Vec<&str> = vec!["a", "-y", "-t7z", "wrapped.7z"];
        for v in &vols {
            wrap.push(v);
        }
        st(&wrap);
        // SFX = 7z.sfx + wrapped.7z
        let mut sfx = fs::read(&sfx_mod).unwrap();
        sfx.extend_from_slice(&fs::read(work.path().join("wrapped.7z")).unwrap());
        let p = write_file(tmp.path(), "1.exe", &sfx);
        assert_eq!(sniff_head(&p), Some("7z"), "SFX 头判定");
        assert_eq!(sniff(&p), Some("7z"), "SFX 应识别为 7z");
        // carve_offset 先比 sniff_head（此时已判 7z），不再走 embedded_offset——
        // 后者扫前缀窗当然能找到 overlay 里的 7z，属预期，不作断言。
        // 阴性对照：纯 PE（无 overlay 压缩档）与 7z.sfx 裸 stub 都不得识别
        let plain = write_file(tmp.path(), "game.exe", &fs::read(&sfx_mod).unwrap());
        assert_eq!(sniff(&plain), None, "纯 PE 不得误判为压缩档");
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
        // part0 是合法卷名（idx 0；扫描时既非 idx>1 也非首卷）
        let v = parse_volume_name("a.part0.rar").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("a", VolFamily::RarPart, 0));
    }

    #[test]
    fn parse_volume_name_rename_marks() {
        // 「删」类尾标（百度/夸克下载或手改卷名常见）
        let v = parse_volume_name("深渊迷宮.7z.001删").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("深渊迷宮", VolFamily::SevenZNum, 1));
        let v = parse_volume_name("a.7z.002旧").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("a", VolFamily::SevenZNum, 2));
        // 夸克分卷：.7z-<hex>.###（hex 段 6-16 位）
        let v = parse_volume_name("a.7z-15eab3e6d4fd.001").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("a", VolFamily::SevenZNum, 1));
        let v = parse_volume_name("a.7z-abcdef.003").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("a", VolFamily::SevenZNum, 3));
        // 无标记/短哈希不误判
        assert!(parse_volume_name("a.7z-abc.001").is_none()); // hex < 6
        assert!(parse_volume_name("a.7z-.001").is_none());
        assert!(parse_volume_name("a.zip-15eab3e6d4fd.001").is_none()); // 哈希形态仅 .7z
        // 主名同步剥离
        assert_eq!(archive_stem("深渊迷宮.7z.001删"), "深渊迷宮");
        assert_eq!(archive_stem("a.7z-15eab3e6d4fd.001"), "a");
        assert_eq!(archive_stem("a.zip.002旧"), "a");
        assert_eq!(archive_stem("game"), "game"); // 纯 CJK/普通名不受影响
        assert_eq!(archive_stem("某游戏"), "某游戏");
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
        let pdf = "游戏集合 - 副本 - 副本 - 副本.pdf";
        assert!(parse_volume_name(pdf).is_none());
        assert_eq!(archive_stem(pdf), "游戏集合 - 副本 - 副本 - 副本");
        // 中文 + 卷名后缀
        let v = parse_volume_name("副本.7z.002").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("副本", VolFamily::SevenZNum, 2));
        let v = parse_volume_name("某游戏.PART02.RAR").unwrap();
        assert_eq!((v.stem.as_str(), v.family, v.idx), ("某游戏", VolFamily::RarPart, 2));
        assert_eq!(archive_stem("副本.tar.gz"), "副本");
        // 长数字不是 r/z 族卷名（\d{2} 严格两位）
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
