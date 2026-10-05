//! 纯 Rust LZ4 帧解码器（帧格式/lz4 块格式，支持 legacy 链）。
//! 支持标准帧/legacy 帧（lz4 -l）/块依赖（-BD）/多帧拼接/跳帧/未压缩块；
//! 不校验 xxhash，解码错误以 Lz4Error 抛出。

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum Lz4Error {
    #[error("{0}")]
    Msg(String),
}

impl From<std::io::Error> for Lz4Error {
    fn from(e: std::io::Error) -> Self {
        Lz4Error::Msg(e.to_string())
    }
}

const MAGIC: u32 = 0x184D2204;
const LEGACY_MAGIC: u32 = 0x184C2102;
const SKIPPABLE_MIN: u32 = 0x184D2A50;
const SKIPPABLE_MAX: u32 = 0x184D2A5F;
const WINDOW: usize = 64 << 10; // lz4 最大匹配距离 64KB
const LEGACY_BLOCK_MAX: usize = 8 << 20; // legacy 格式固定 8MB 块
const IO_BUF: usize = 1 << 20; // 文件 I/O 缓冲（1MB）

/// 块大小上限表（BD 字段索引，默认 4MiB）。
fn block_max(bd: u8) -> usize {
    match (bd >> 4) & 7 {
        4 => 64 << 10,
        5 => 256 << 10,
        6 => 1 << 20,
        _ => 4 << 20,
    }
}

fn err(msg: &str) -> Lz4Error {
    Lz4Error::Msg(msg.to_string())
}

/// 读到 n 字节或 EOF 为止（仅 EOF 才短读）。
fn read_up_to<R: Read>(fin: &mut R, n: usize) -> Result<Vec<u8>, Lz4Error> {
    let mut buf = vec![0u8; n];
    let mut filled = 0usize;
    while filled < n {
        match fin.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(m) => filled += m,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    buf.truncate(filled);
    Ok(buf)
}

/// 解一个 LZ4 块。hist 为前一块的窗口（块依赖模式），最多取末尾 64KB。
fn decompress_block(data: &[u8], hist: &[u8]) -> Result<Vec<u8>, Lz4Error> {
    let hist = if hist.len() > WINDOW {
        &hist[hist.len() - WINDOW..]
    } else {
        hist
    };
    let hlen = hist.len();
    let mut dst: Vec<u8> = Vec::new();
    let end = data.len();
    let mut i = 0usize;
    while i < end {
        let token = data[i];
        i += 1;
        let mut lit_len = (token >> 4) as usize;
        if lit_len == 15 {
            loop {
                if i >= end {
                    return Err(err("字面量长度被截断"));
                }
                let b = data[i];
                i += 1;
                lit_len += b as usize;
                if b != 255 {
                    break;
                }
            }
        }
        if i + lit_len > end {
            return Err(err("字面量被截断"));
        }
        dst.extend_from_slice(&data[i..i + lit_len]);
        i += lit_len;
        if i >= end {
            break; // 最后一个序列只有字面量
        }
        if i + 2 > end {
            return Err(err("匹配偏移被截断"));
        }
        let offset = (data[i] as usize) | ((data[i + 1] as usize) << 8);
        i += 2;
        if offset == 0 {
            return Err(err("偏移为 0"));
        }
        let mut match_len = ((token & 0x0F) as usize) + 4;
        if (token & 0x0F) == 15 {
            loop {
                if i >= end {
                    return Err(err("匹配长度被截断"));
                }
                let b = data[i];
                i += 1;
                match_len += b as usize;
                if b != 255 {
                    break;
                }
            }
        }
        let total = hlen + dst.len();
        if offset > total {
            return Err(err("偏移超出历史窗口"));
        }
        let start = total - offset;
        if start < hlen {
            // 头部落在历史窗口
            let k = std::cmp::min(match_len, hlen - start);
            dst.extend_from_slice(&hist[start..start + k]);
            match_len -= k;
        }
        if match_len > 0 {
            // 自重复：单位 = 虚拟流末尾 offset 字节
            let ustart = hlen + dst.len() - offset;
            let unit: Vec<u8> = if ustart >= hlen {
                dst[ustart - hlen..].to_vec()
            } else {
                let k = hlen - ustart;
                let mut u = Vec::with_capacity(offset);
                u.extend_from_slice(&hist[ustart..]);
                u.extend_from_slice(&dst[..offset - k]);
                u
            };
            dst.reserve(match_len);
            let mut remaining = match_len;
            while remaining > 0 {
                let n = remaining.min(unit.len());
                dst.extend_from_slice(&unit[..n]);
                remaining -= n;
            }
        }
    }
    Ok(dst)
}

/// 从二进制可读流解到二进制可写流，支持多帧拼接。
pub fn decompress_stream<R: Read, W: Write>(fin: &mut R, fout: &mut W) -> Result<(), Lz4Error> {
    loop {
        let header = read_up_to(fin, 4)?;
        if header.is_empty() {
            return Ok(());
        }
        if header.len() < 4 {
            return Err(err("截断的帧头"));
        }
        let magic = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        if (SKIPPABLE_MIN..=SKIPPABLE_MAX).contains(&magic) {
            let size_b = read_up_to(fin, 4)?;
            if size_b.len() < 4 {
                return Err(err("截断的跳帧"));
            }
            let size =
                u32::from_le_bytes([size_b[0], size_b[1], size_b[2], size_b[3]]) as usize;
            let _ = read_up_to(fin, size)?; // payload 只跳过（短读不报错）
            continue;
        }
        if magic == LEGACY_MAGIC {
            read_legacy(fin, fout)?;
            continue;
        }
        if magic != MAGIC {
            return Err(Lz4Error::Msg(format!("不是 lz4 帧（magic={:#010x}）", magic)));
        }
        read_frame(fin, fout)?;
    }
}

/// legacy 帧（lz4 -l，Linux 内核早期格式）：magic 后连续 [u32 压缩大小|块数据]。
fn read_legacy<R: Read, W: Write>(fin: &mut R, fout: &mut W) -> Result<(), Lz4Error> {
    loop {
        let size_b = read_up_to(fin, 4)?;
        if size_b.is_empty() {
            return Ok(());
        }
        if size_b.len() < 4 {
            return Err(err("截断的 legacy 块头"));
        }
        let csize = u32::from_le_bytes([size_b[0], size_b[1], size_b[2], size_b[3]]) as usize;
        if csize == 0 {
            return Ok(());
        }
        let data = read_up_to(fin, csize)?;
        if data.len() < csize {
            return Err(err("截断的 legacy 块数据"));
        }
        let out = decompress_block(&data, b"")?;
        if out.len() > LEGACY_BLOCK_MAX {
            return Err(err("legacy 块解压结果超过 8MB"));
        }
        fout.write_all(&out)?;
    }
}

fn read_frame<R: Read, W: Write>(fin: &mut R, fout: &mut W) -> Result<(), Lz4Error> {
    let flg_b = read_up_to(fin, 1)?;
    let bd_b = read_up_to(fin, 1)?;
    if flg_b.is_empty() || bd_b.is_empty() {
        return Err(err("截断的帧描述符"));
    }
    let flg = flg_b[0];
    let bd = bd_b[0];
    if (flg >> 6) != 0b01 {
        return Err(Lz4Error::Msg(format!("未知帧版本: {}", (flg >> 6) & 3)));
    }
    let block_checksum = (flg >> 4) & 1 != 0;
    let has_content_size = (flg >> 3) & 1 != 0;
    let content_checksum = (flg >> 2) & 1 != 0;
    let has_dict_id = flg & 1 != 0;
    if has_content_size && read_up_to(fin, 8)?.len() < 8 {
        return Err(err("截断的内容长度"));
    }
    if has_dict_id && read_up_to(fin, 4)?.len() < 4 {
        return Err(err("截断的词典 ID"));
    }
    if read_up_to(fin, 1)?.is_empty() {
        // HC（不校验）
        return Err(err("截断的帧头校验"));
    }
    let block_max = block_max(bd);
    let mut window: Vec<u8> = Vec::new(); // 块依赖模式的历史
    loop {
        let size_b = read_up_to(fin, 4)?;
        if size_b.len() < 4 {
            return Err(err("截断的块头"));
        }
        let bsz_raw = u32::from_le_bytes([size_b[0], size_b[1], size_b[2], size_b[3]]);
        if bsz_raw == 0 {
            break; // EndMark
        }
        let uncompressed = bsz_raw & 0x8000_0000 != 0;
        let bsz = (bsz_raw & 0x7FFF_FFFF) as usize;
        let data = read_up_to(fin, bsz)?;
        if data.len() < bsz {
            return Err(err("截断的块数据"));
        }
        let out = if uncompressed {
            data
        } else {
            decompress_block(&data, &window)?
        };
        if out.len() > block_max {
            return Err(err("块解压结果超出帧声明的上限"));
        }
        fout.write_all(&out)?;
        window.extend_from_slice(&out);
        if window.len() > WINDOW {
            window.drain(..window.len() - WINDOW); // window = (window + out)[-WINDOW:]
        }
        if block_checksum && read_up_to(fin, 4)?.len() < 4 {
            return Err(err("截断的块校验"));
        }
    }
    if content_checksum && read_up_to(fin, 4)?.len() < 4 {
        return Err(err("截断的流校验"));
    }
    Ok(())
}

/// 解码内存中的 lz4 帧，返回原始字节。
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, Lz4Error> {
    let mut src = std::io::Cursor::new(data);
    let mut dst = Vec::new();
    decompress_stream(&mut src, &mut dst)?;
    Ok(dst)
}

/// 解码文件，返回输出字节数。
pub fn decompress_file(src: &Path, dst: &Path) -> Result<u64, Lz4Error> {
    let fin = File::open(src)?;
    let fout = File::create(dst)?;
    let mut reader = BufReader::with_capacity(IO_BUF, fin);
    let mut writer = BufWriter::with_capacity(IO_BUF, fout);
    decompress_stream(&mut reader, &mut writer)?;
    writer.flush()?;
    Ok(writer.get_ref().metadata()?.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- 辅助 ----------

    fn unhex(s: &str) -> Vec<u8> {
        assert!(s.len() % 2 == 0, "hex 长度必须为偶数");
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    /// 与参考实现的样例数据逐字节一致（同一 LCG 常数）。
    fn sample_data() -> Vec<u8> {
        sample_impl(150, 8192)
    }

    /// FIX_STD_BX_CS 对应的小样本（lz4.exe -B4 -BX --content-size）。
    fn sample_small() -> Vec<u8> {
        sample_impl(20, 512)
    }

    fn sample_impl(n_lines: u32, n_random: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let line = "自动解压工具 lz4 帧解码器测试样本 ".repeat(8);
        for i in 0..n_lines {
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(format!("line {:05}\n", i).as_bytes());
        }
        let mut state: u64 = 0x9E3779B97F4A7C15;
        for _ in 0..n_random {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            out.push((state >> 33) as u8);
        }
        for i in n_lines..n_lines * 2 {
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(format!("line {:05}\n", i).as_bytes());
        }
        out
    }

    /// 手工构造标准帧：magic + flg + bd + hc(0x00) + chunks（每个已是 [u32 头|数据]）+ EndMark。
    fn frame(flg: u8, bd: u8, chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&MAGIC.to_le_bytes());
        v.push(flg);
        v.push(bd);
        v.push(0x00); // HC（不校验）
        for c in chunks {
            v.extend_from_slice(c);
        }
        v.extend_from_slice(&0u32.to_le_bytes()); // EndMark
        v
    }

    /// 压缩块 chunk：[压缩大小|块数据]。
    fn compressed_chunk(block: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(block.len() + 4);
        v.extend_from_slice(&(block.len() as u32).to_le_bytes());
        v.extend_from_slice(block);
        v
    }

    /// 未压缩块 chunk（高位置位）：[大小|0x80000000|原始数据]。
    fn uncompressed_chunk(payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(payload.len() + 4);
        v.extend_from_slice(&((payload.len() as u32) | 0x8000_0000).to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    /// 跳帧 chunk：[magic|u32 大小|payload]。
    fn skippable(payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(payload.len() + 8);
        v.extend_from_slice(&SKIPPABLE_MIN.to_le_bytes());
        v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    /// 构造压缩块：1 字节字面量 'A' + 偏移 1 的自重复匹配，共输出 1+match_total 字节。
    fn rle_block(match_total: usize) -> Vec<u8> {
        assert!(match_total >= 19); // 4 + 15 + 扩展
        let mut v = vec![0x1F, b'A', 0x01, 0x00];
        let mut sum = match_total - 19;
        while sum >= 255 {
            v.push(255);
            sum -= 255;
        }
        v.push(sum as u8);
        v
    }

    fn expect_err(data: &[u8], msg: &str) {
        let e = decompress(data).unwrap_err();
        assert_eq!(e.to_string(), msg, "data={:02x?}", data);
    }

    // ---------- 内嵌固件 round-trip（lz4.exe v1.10.0 生成，见文件底部常量） ----------

    #[test]
    fn fixture_standard_frame() {
        // lz4.exe -B4：标准帧、块独立（flg=0x64）、默认带流校验、2 个压缩块。
        assert_eq!(decompress(&unhex(FIX_STD_B4)).unwrap(), sample_data());
    }

    #[test]
    fn fixture_block_dependency() {
        // lz4.exe -B4 -BD：块依赖（flg=0x44），含 75 处跨块匹配（头部落历史窗口）。
        assert_eq!(decompress(&unhex(FIX_BD_B4)).unwrap(), sample_data());
    }

    #[test]
    fn fixture_legacy() {
        // lz4.exe -l：legacy 帧（magic=0x184C2102）。
        assert_eq!(decompress(&unhex(FIX_LEGACY)).unwrap(), sample_data());
    }

    #[test]
    fn fixture_block_checksum_and_content_size() {
        // lz4.exe -B4 -BX --content-size：块校验 + 内容长度字段只跳过不校验。
        assert_eq!(decompress(&unhex(FIX_STD_BX_CS)).unwrap(), sample_small());
    }

    #[test]
    fn fixture_via_stream_and_file() {
        let blob = unhex(FIX_BD_B4);
        let mut out = Vec::new();
        let mut cur = std::io::Cursor::new(&blob);
        decompress_stream(&mut cur, &mut out).unwrap();
        assert_eq!(out, sample_data());
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.lz4");
        let dst = dir.path().join("out.bin");
        std::fs::write(&src, &blob).unwrap();
        let n = decompress_file(&src, &dst).unwrap();
        let got = std::fs::read(&dst).unwrap();
        assert_eq!(n, got.len() as u64);
        assert_eq!(got, sample_data());
    }

    // ---------- 手工构造：正常路径 ----------

    #[test]
    fn empty_input_ok() {
        assert_eq!(decompress(b"").unwrap(), b"");
    }

    #[test]
    fn handcrafted_uncompressed_block() {
        // 未压缩块（高位置位）原样输出。
        let payload = b"raw payload, not compressible at all 0123456789";
        let mut data = frame(0x40, 0x40, &[uncompressed_chunk(payload)]);
        data.extend_from_slice(&frame(0x40, 0x40, &[uncompressed_chunk(b"")]));
        assert_eq!(decompress(&data).unwrap(), payload);
    }

    #[test]
    fn skippable_frames_skipped() {
        // 只有跳帧 → 空输出；跳帧 + 标准帧 → 正常解码；多个跳帧 + 跳帧大小 0。
        assert_eq!(decompress(&skippable(b"whatever")).unwrap(), b"");
        let mut data = skippable(b"junk");
        data.extend_from_slice(&skippable(b""));
        data.extend_from_slice(&frame(0x40, 0x70, &[uncompressed_chunk(b"real")]));
        data.extend_from_slice(&skippable(&[0xAB; 3]));
        assert_eq!(decompress(&data).unwrap(), b"real");
    }

    #[test]
    fn multi_frame_concat() {
        // 多帧拼接：两个标准帧 + 夹在其间的跳帧。
        let mut data = frame(0x40, 0x70, &[uncompressed_chunk(b"hello-")]);
        data.extend_from_slice(&skippable(b"mid"));
        data.extend_from_slice(&frame(0x40, 0x70, &[uncompressed_chunk(b"world")]));
        assert_eq!(decompress(&data).unwrap(), b"hello-world");
    }

    #[test]
    fn self_repeat_within_block() {
        // 自重复：offset=2、len=10，unit="ab"，整除。
        let block = [0x26, b'a', b'b', 0x02, 0x00];
        assert_eq!(decompress_block(&block, b"").unwrap(), b"abababababab");
        // 自重复：offset=3、len=7，unit="def"，非整除。
        let block2 = [0x63, b'a', b'b', b'c', b'd', b'e', b'f', 0x03, 0x00];
        assert_eq!(decompress_block(&block2, b"").unwrap(), b"abcdefdefdefd");
    }

    #[test]
    fn match_spanning_history_and_self_repeat() {
        // 块依赖帧：块1 输出 "abcd"；块2 的匹配 offset=6、len=10，
        // 头部落在历史窗口（先拷 hist 的 "abcd"），剩余 6 字节由整段 dst 作为 unit 自重复。
        // 期望块2输出 "xyabcdxyabcd"。
        let f = frame(
            0x40, // 块依赖（bit5=0）
            0x40,
            &[
                compressed_chunk(&[0x40, b'a', b'b', b'c', b'd']), // 纯字面量 "abcd"
                compressed_chunk(&[0x26, b'x', b'y', 0x06, 0x00]), // lit "xy" + match(off=6,len=10)
            ],
        );
        assert_eq!(decompress(&f).unwrap(), b"abcdxyabcdxyabcd");
    }

    #[test]
    fn block_max_ids_accept() {
        // BD 块大小 id=5（256KB）：200KB 输出必须接受（区分 64KB 默认）。
        let f5 = frame(0x40, 0x50, &[compressed_chunk(&rle_block(200_000))]);
        assert_eq!(decompress(&f5).unwrap(), vec![b'A'; 200_001]);
        // BD 块大小 id=6（1MB）：1MB 输出必须接受。
        let f6 = frame(0x40, 0x60, &[compressed_chunk(&rle_block((1 << 20) - 1))]);
        assert_eq!(decompress(&f6).unwrap(), vec![b'A'; 1 << 20]);
        // 保留 id（0-3）按默认 4MB 处理。
        let f0 = frame(0x40, 0x00, &[compressed_chunk(&rle_block(200_000))]);
        assert_eq!(decompress(&f0).unwrap(), vec![b'A'; 200_001]);
    }

    // ---------- 手工构造：错误路径（消息措辞为回归断言） ----------

    #[test]
    fn err_not_lz4_frame() {
        expect_err(&[0x44, 0x33, 0x22, 0x11], "不是 lz4 帧（magic=0x11223344）");
        expect_err(&[0x00, 0x00, 0x00, 0x00], "不是 lz4 帧（magic=0x00000000）");
    }

    #[test]
    fn err_truncated_frame_header() {
        expect_err(&[0x04], "截断的帧头");
        expect_err(&[0x04, 0x22, 0x4d], "截断的帧头");
    }

    #[test]
    fn err_truncated_frame_descriptor() {
        let mut d = MAGIC.to_le_bytes().to_vec();
        d.push(0x40); // 只有 flg，没有 bd
        expect_err(&d, "截断的帧描述符");
    }

    #[test]
    fn err_unknown_version() {
        let f = |flg: u8| frame(flg, 0x40, &[]);
        expect_err(&f(0x00), "未知帧版本: 0");
        expect_err(&f(0x80), "未知帧版本: 2");
        expect_err(&f(0xC0), "未知帧版本: 3");
    }

    #[test]
    fn err_truncated_frame_optional_fields() {
        // 内容长度字段只给 3 字节。
        let mut d = MAGIC.to_le_bytes().to_vec();
        d.extend_from_slice(&[0x48, 0x40]); // flg: 版本01 + content size
        d.extend_from_slice(&[1, 2, 3]);
        expect_err(&d, "截断的内容长度");
        // 词典 ID 字段只给 2 字节。
        let mut d = MAGIC.to_le_bytes().to_vec();
        d.extend_from_slice(&[0x41, 0x40]); // flg: 版本01 + dict id
        d.extend_from_slice(&[1, 2]);
        expect_err(&d, "截断的词典 ID");
        // 缺 HC 字节。
        let mut d = MAGIC.to_le_bytes().to_vec();
        d.extend_from_slice(&[0x40, 0x40]);
        expect_err(&d, "截断的帧头校验");
    }

    #[test]
    fn err_truncated_block_header() {
        let mut d = frame(0x40, 0x40, &[]);
        d.truncate(d.len() - 6); // 去掉 EndMark(4) 之外再多去 2 字节，留一个半块头
        d.extend_from_slice(&[0x01, 0x00]); // 只有 2 字节的块头
        expect_err(&d, "截断的块头");
    }

    #[test]
    fn err_truncated_block_data() {
        let mut d = MAGIC.to_le_bytes().to_vec();
        d.extend_from_slice(&[0x40, 0x40, 0x00]); // flg bd hc
        d.extend_from_slice(&100u32.to_le_bytes()); // 声明 100 字节
        d.extend_from_slice(&[0xAA; 10]); // 实际只有 10 字节
        expect_err(&d, "截断的块数据");
    }

    #[test]
    fn err_truncated_block_checksum() {
        // flg 带块校验位：块结束后没有 4 字节校验。
        let mut d = MAGIC.to_le_bytes().to_vec();
        d.extend_from_slice(&[0x50, 0x40, 0x00]); // flg: 版本01 + block checksum
        d.extend_from_slice(&compressed_chunk(&[0x00])); // 输出为空的块
        expect_err(&d, "截断的块校验");
    }

    #[test]
    fn err_truncated_content_checksum() {
        // flg 带流校验位：块 + EndMark 之后没有 4 字节校验。
        let mut d = MAGIC.to_le_bytes().to_vec();
        d.extend_from_slice(&[0x44, 0x40, 0x00]); // flg: 版本01 + content checksum
        d.extend_from_slice(&compressed_chunk(&[0x00]));
        d.extend_from_slice(&0u32.to_le_bytes()); // EndMark
        expect_err(&d, "截断的流校验");
    }

    #[test]
    fn err_truncated_skippable() {
        let mut d = SKIPPABLE_MIN.to_le_bytes().to_vec();
        d.extend_from_slice(&[0x01, 0x00]); // 只有 2 字节的大小字段
        expect_err(&d, "截断的跳帧");
    }

    #[test]
    fn err_truncated_legacy() {
        let mut d = LEGACY_MAGIC.to_le_bytes().to_vec();
        d.extend_from_slice(&[0x01, 0x00]); // legacy 块头只有 2 字节
        expect_err(&d, "截断的 legacy 块头");
        let mut d = LEGACY_MAGIC.to_le_bytes().to_vec();
        d.extend_from_slice(&100u32.to_le_bytes());
        d.extend_from_slice(&[0xAA; 10]);
        expect_err(&d, "截断的 legacy 块数据");
    }

    #[test]
    fn err_block_over_declared_max() {
        // BD id=4（64KB）：输出 65537 字节 > 上限。
        let f = frame(0x40, 0x40, &[compressed_chunk(&rle_block(65_536))]);
        expect_err(&f, "块解压结果超出帧声明的上限");
        // BD id=6（1MB）：输出 1MB+1 字节 > 上限。
        let f = frame(0x40, 0x60, &[compressed_chunk(&rle_block(1 << 20))]);
        expect_err(&f, "块解压结果超出帧声明的上限");
    }

    #[test]
    fn err_legacy_over_8mb() {
        // legacy 固定 8MB 块上限：输出 8MB+1 字节。
        let block = rle_block(8 << 20);
        let mut d = LEGACY_MAGIC.to_le_bytes().to_vec();
        d.extend_from_slice(&(block.len() as u32).to_le_bytes());
        d.extend_from_slice(&block);
        d.extend_from_slice(&0u32.to_le_bytes());
        expect_err(&d, "legacy 块解压结果超过 8MB");
    }

    #[test]
    fn err_block_level() {
        // 字面量长度扩展被截断。
        let f = frame(0x40, 0x40, &[compressed_chunk(&[0xF0])]);
        expect_err(&f, "字面量长度被截断");
        let f = frame(0x40, 0x40, &[compressed_chunk(&[0xF0, 0xFF])]);
        expect_err(&f, "字面量长度被截断");
        // 字面量越过块尾。
        let f = frame(0x40, 0x40, &[compressed_chunk(&[0x50, b'a'])]);
        expect_err(&f, "字面量被截断");
        // 匹配偏移字段不足 2 字节。
        let f = frame(0x40, 0x40, &[compressed_chunk(&[0x10, b'a', 0x01])]);
        expect_err(&f, "匹配偏移被截断");
        // 偏移为 0。
        let f = frame(0x40, 0x40, &[compressed_chunk(&[0x10, b'a', 0x00, 0x00])]);
        expect_err(&f, "偏移为 0");
        // 匹配长度扩展被截断。
        let f = frame(0x40, 0x40, &[compressed_chunk(&[0x1F, b'a', 0x01, 0x00])]);
        expect_err(&f, "匹配长度被截断");
        // 偏移超出历史窗口（无历史，输出仅 1 字节，offset=5）。
        let f = frame(0x40, 0x40, &[compressed_chunk(&[0x10, b'a', 0x05, 0x00])]);
        expect_err(&f, "偏移超出历史窗口");
    }

    // ---------- 可选活测：本机有官方 lz4.exe 时做 round-trip ----------

    #[test]
    fn live_roundtrip_if_lz4_available() {
        let lz4 = r"D:\APP\解压工具\解压工具\lz4.exe";
        if !Path::new(lz4).exists() {
            eprintln!("skip: 未找到 {lz4}");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("sample.bin");
        std::fs::write(&src, sample_data()).unwrap();
        let modes: [&[&str]; 4] = [&["-B4"], &["-B4", "-BD"], &["-B4", "-BX", "--content-size"], &["-l"]];
        for (i, mode) in modes.iter().enumerate() {
            let comp = dir.path().join(format!("s{i}.lz4"));
            let out = dir.path().join(format!("s{i}.out"));
            let st = std::process::Command::new(lz4)
                .args(["-f", "-q"])
                .args(*mode)
                .arg(&src)
                .arg(&comp)
                .status()
                .unwrap();
            assert!(st.success(), "lz4 压缩失败: {mode:?}");
            let n = decompress_file(&comp, &out).unwrap();
            let got = std::fs::read(&out).unwrap();
            assert_eq!(n, got.len() as u64);
            assert_eq!(got, sample_data(), "mode {mode:?}");
        }
    }

    // ---------- 内嵌固件（lz4.exe v1.10.0 生成；hex 编码，unhex 后使用） ----------
const FIX_STD_B4: &str = concat!(
    "04224d186440a7781c0000ff21e887aae58aa8e8a7a3e58e8be5b7a5e585b7206c7a3420e5b8a7e8a7a3e7a081e599a8",
    "e6b58be8af95e6a0b7e69cac203000ff3e606c696e65203001001f0a5b01ff3e0f50011d058b011f318b01ff781f328b",
    "01ff781f338b01ff781f348b01ff781f358b01ff781f368b01ff781f378b01ff781f388b01ff781f398b01ff772f3130",
    "8b01ff780f6e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f316e",
    "0fff781f316e0fff781f326e0fff781f326e0fff781f326e0fff781f326e0fff781f326e0fff781f326e0fff781f326e",
    "0fff781f326e0fff781f326e0fff781f326e0fff781f336e0fff781f336e0fff781f336e0fff781f336e0fff781f336e",
    "0fff781f336e0fff781f336e0fff781f336e0fff781f336e0fff781f336e0fff781f346e0fff781f346e0fff781f346e",
    "0fff781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f356e",
    "0fff781f356e0fff781f356e0fff781f356e0fff781f356e0fff781f356e0fff781f356e0fff781f356e0fff781f356e",
    "0fff781f356e0fff781f366e0fff781f366e0fff781f366e0fff781f366e0fff781f366e0fff781f366e0fff781f366e",
    "0fff781f366e0fff781f366e0fff781f366e0fff781f37945cff781f376e0fff781f376e0fff781f376e0fff781f376e",
    "0fff781f376e0fff781f376e0fff781f376e0fff781f376e0fff781f376e0fff781f386e0fff781f386e0fff781f386e",
    "0fff781f386e0fff781f386e0fff781f386e0fff781f386e0fff781f386e0fff781f386e0fff781f386e0fff781f396e",
    "0fff781f396e0fff781f396e0fff781f396e0fff781f396e0fff781f396e0fff781f396e0fff781f396e0fff781f396e",
    "0fff781f396e0fff771f314c9aff782f31306e0fff772f31306e0fff772f31306e0fff772f31306e0fff772f31306e0f",
    "ff772f31306e0fff772f31306e0fff772f31306e0fff772f31306e0fff780f4c9aff782f31316e0fff780f4c9aff781f",
    "314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f",
    "314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f",
    "314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f",
    "314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f",
    "314c9aff781f314c9aff781f314c9aff781f314c9aff78f0ffffffffffffffffffffffffffffffffffffffffffffffff",
    "9b3134390a10a625a1eaef065248d7e1900a4df3391f04b7bfd7ee70dfe7537dc29c2188378509eb5db269c1d9bcbcdd",
    "f031bc951603661fb1fdfe183490f41c4a608ad7c30c1f4c76ca7ead6af35ffd792bc8fa6d48079f48b90a216fd4a758",
    "77da750703e5ec5d104e01b4ce10720da0735d06090df58ae1273eec61868d0a8945ad445329b6e5e9882c1f2f1801d5",
    "1f17a3c4284d71a7d4c0266ce832a4217a68c55e3b6d0b8b40edb09a8b4092ec024f320112c234a4a291fc64a4a422ad",
    "496f85a747eb3e7c3f8d45adb3a8c4012d8ec26a304d2bfc99ec7a2034e6596fc03ce04f6624699fe50abbb9c2a973ab",
    "661c5b2cabd8bf6c009e2ab6ebce15e8c3426044bd05ef406a0d8c77159557b4de2a060a7ebe8ddabfa83bc30ffb83de",
    "12d11a8b058d7d393039ecf6c3fc25e63961000312acafc2876fcaa38a5f8b7399f61f1e5ef488a824a25b129c73325a",
    "18016965570b79297e72bd342b45b3aa31c66b4db04b5210acc2b4789ad89243256d767560e1bb0266840b1159d08678",
    "f23857937ba6691210edc220e017b83b93462282743781224478944d4e056a51f47b8cfd38beaa9060474d482e709a13",
    "33fc5d88b1fe56a88e5e6eb2d34e0c349dc770fd2c15bd62de38abf4e13a511bf2e6c14a1c7603edcf35b57d72823938",
    "67b71ad0c5a11b7eea62d5eb63293af2ead5c0f44111d1efd31ee09e386444982c18c35471e98ea56615f4332d1393d5",
    "e72b552f47d74a3f4f158f6fe5afe442ed43b669f8b03193a443730c3937a568716c35c78c4d7a6f0923dff7ac9596e3",
    "1af0bfcc5317f0aacbfb9372fffd5b03535883c4bcdaaffc851435a469c87f765e8b22760a448c20c3547714ed426a84",
    "aa7ffd056daabbc029ac978858fdca4ce70a057604831aab9bebb7d55d15ee966550af623417b6d8e75d771753f28e9f",
    "2b416651ebf103ae74d374cb15038e8256b8234441883d186b79066387f22a9a38b589dffc9d87e6eb0de4b439d71b7b",
    "b82e16c676db33f4bceb05daafdd29ea77f7eba3652cbf9a007966fdc8e4b269364ea150034904ed6b691684caa6a2c7",
    "fa73b2b01d031a427e51153692c45b3a77eaf2b37cca65803b298dbb5a6117853fc79dff3ae761b9e816d314b4a32eaa",
    "27a177c971f894914818c06f16c0dc9f7f997951cf2338e9de0ddef091fdf0947df6968f8c7b195ab190dcde2820f143",
    "73ea0d87412f1ef70930e2bd48eb35c045e0d4c425e40ad7c1862fd2e30568619fef8e8424d4eef082a1898caee30428",
    "6ee391055f1cc9b1994a005ffe2741371b618d878ed1e0f6bba58884cbfef2d10ba3316bbd4245b058b1db214ef2d25f",
    "da546f09f6ff0ceee9143662ccbcca0ce1fbcfa73f13bca1c3d164f5f98ca15a758d541a89f867aeec4d9b75824ba84e",
    "708f719df6ccfdca7134aa3b375ccb9f7457913e053948a5bbb2ff1c594c9f7379e4b487239626d4708df48e860dde25",
    "15d59bc814cb670e4995fcbfcf16d5930af3038ccb63e93573ed15066413437697dc7bba1d645e9befb21252707c2949",
    "074042dfd456492374773af189b315340442be22a90faa9e5622b34f4e1350852f6c025f9babddfce69840129e828c2e",
    "75bae6f42f502cbde0d1d730f9f578d5aa4c32ae0c18923655b87f5e7971d4e7e2235c6979c72d1992720c74f9086936",
    "108052d341b1eacb94722037d24c78a5655edfdd755adaff78f40a11c6c1245af1821e559454be246847ec2d81c33bfe",
    "06a7782a90d546139679be7d456c0780df43c4d93b867f8babd69a3532ee7de80466e8869110f0fc49c8e121b8ed6db5",
    "f737953c64dcf513fa8ca76ea1501b3f9c8499deef96be953544045c4309cda9e9f23436c7e08606dadfa156535ecfd9",
    "554412b8adc87f95ae6027fcda275efb853a466fc674eed564fef98ccc851410c692e68bb185f1c09e97c33fc1983603",
    "bd9aa82401bb87827008560b4eac84cde1de249d9c4b3c91efda604f835dec203478193d73780a8d36c366e90b3cba1a",
    "bc6d4b6327ddf46a768da43a6a68b78845ae72f108f9ca6581cc2c8fe2a3b42bd92fb85af86c9f3f5cf7e57778631190",
    "8a9951dcb6787a4d5052d47e9859b1ebee159966fd35afc301393adde2f5db7de9b14fa118026fd3034503859066970b",
    "32e45baf44ad08156fc30923078af6d11b9fac0586db5cb40310a7810466ce8f208ca0fe5521ffee3a471ddbeb906a15",
    "b3d0868caf639750eccc4a99be11a6dac7fdac9b0bdedc4dee3133f22e460726aa8a8497b07cd99134f5e1fd523b373a",
    "8e7957a6ebd4d7a5f79a0da884fa830866840cffac6e7e5d579c201fd75cbef77c6f7ff8ffbca28a08bc0313b4cf1d2b",
    "46f5ee34eb4d49807a2346761c9683df07c8f68030bd5fdc07ea12170805bf3ea7309dda6cdca01a9e6d73b468373953",
    "57c2f81e1c912a75750470e74dbb9a776fb4dae403f5508d3d927689adc0dcd413451800712a1459556a9afd4b364f66",
    "17c5e533ee6cd2e67f1720de446b1511a7b3b281c5d7a75d9974e79ebba4883c317b31b3f17201d1d8a4138e262d1a73",
    "1043dc83f0e7e75b066499cfc2641e9c765e96f5ef7f69ff3a3414aaa23d0f2a21d1d951e4d3d1d9a1dd6dd9e9c6bae5",
    "4876fc5100b5fa17b8cedc329b19e8b8503705f508dc5f3997d7ad429955f104c1e2917f0ec15621b2ba69503d0ac704",
    "fe1e4b1b0e380666a517bf4b119cecba3bee79b4eb4688757c34cf1c3529e2e13e59166c4eb0b9fe011e39ecdd65856f",
    "56a50044eabf492689a28ad072e7de1e1fb3bd6a9ec96906c5a37056d684e57c83e947b6fbe4c2f214494c8c520eb510",
    "784778cdac66070dd78889e6941caff90e057a6c418fcca947835090644715a42c58d263d8f302e44f4a0aad6b609408",
    "a4c67db62c21af21e5702efba0213fe8f6a0976987014ec066fdec66e1d97fa5610c1f75126a68a0743325031c936e18",
    "b12c456a0674def0d8ba2ef5ab302ff352e2cc36480763c7e39cf2b94704b02f22a9009cda20aa05d49cde64236a5d05",
    "850fbbd0bb4fc302b9691f41a4ce4dee404100be9ef52f825f2cb46140b7592f8e33a37f0ab41f75b7efd391ff44a76f",
    "fee985725aa06d05425d1d3a15ae2cd39051e6831da6c166045a26ad2a36982c933b441c5baf6cf86ffece58c01a3ba9",
    "c8ed4a3d3efc6e2cd95bed6435f8569344ca5b40913b3bbbedaed240ee376594949d23c9b5d7a19ea45e109024e3d40b",
    "aaf4c15d620771553d56200ecc7fa8e7fe1f0621de094efbb54158ce2b5ea089803c394a0529309d4816aa7183e93e38",
    "c2ecfdb2c3f5215d6888c1be67634897e919bf165e2fa1e6c5c3ea2a31af3eaadb53358035fa44a0ca124dc2168139e3",
    "38507b6052c1fc3323d3fa086297c2ba89042fc166559bd5c54e4f3450621e4d3cc93339a5c902ddf2db206309b3828e",
    "da2f72f600858628c7ef432c41d3c47405de3d7d4d1604c1c97be11ff8a8f8bfb513bc8fbf332654ee241cb5df447dbf",
    "3437ecb5518260f0b4da8f21da7602a96214b2018a94178daf49139128d1f9f1ba1d826c219983dc1c9a6053acb5ff36",
    "925413690c51b7eafc1efc6fd7c6b130294ca61669b86b012a47fa1cb665762deac165a4cee8e558f57f12ae9ea2be95",
    "7b4c4e6274b8a714ba558461022730fb0ab02df1cd95410308914f97e09f453659433423fa12d9b3ba8e3afd5a0edf05",
    "16ee980898b6ed44a080351ae9b44cb9e74bc7a28281b68d1b0a7ec5bbda3b72c359b1bee3add10c4caf260ca71f265b",
    "09499b823629871435a363fd51c2a778d9ef101aa44a58d5686e30e3e3634c7d303f591a403c2aa4b2ecc75df2ba4d2b",
    "436c0dfbacabb30d472c62f4fcd1c2b8a12232539d209336f6a2dc871233c2c793556f2bbfa59c02d93598301b90f761",
    "3f30d0057c12d08a0aa73f084065277f0e959e072aa69251d8dbd160f21a289cd3c3c6d5134c85d3026f85d51e13cbcc",
    "338c5392e31e9ed06f34fee1f2483be8d4a87f8dfba8eee8eafe4e46d9cd4835c6a8ca171d5d93fe624d52e704d62828",
    "b4f5b30205ca65eb2955d59324b828a0567974485f01cef0b8da1430c063d337a459586699c15c7d716c02d4aed2051c",
    "4a4723d524d37abbe774f14a2cfc73efe1f60d31892ae46524a6fb7c1fb828326622c48eee314b657745cb4cef1f6eb8",
    "7ab412726ddca8c03cc432558ce9adadd87f86fde0f1c64d2e420b130d3ebc979c16b6b67c0776aeb75dfbfe758c22ed",
    "be449286dcc5fe1ebde173001ec7cab65890485ee4d9377e7dd199e83cb6a9ae2e6739021324d637e0496d340aa7d778",
    "ef5b8782a4a8765bcebdd1d51a2d9859c3faa7db26abc19a100cdc4f404ed8fe97c89646e4935f7925f3fbd0a1af9ace",
    "a5c511abbc02055aa6596fa86d39d73dc81b6ac1cfa642c5a3ee92afadb752b50d5a69599a45798688b27212bbf1d881",
    "03d9be51eb83900dfcc5440edfbe7a4755a88cb33de4ca984d221d12849334841ca27f7df2835fafd8a387cb9807859d",
    "7d14f691f30b385fedef659e8cd67df20184c2522e606bc8c4b8a60671dcbd7d350bf26b7689db6fe4d44f5dbe8ce27d",
    "06d0343d4c5396df97b35399335beba765174a7b002d5e0eddb0bf6061ad0b66ab6f096bbbbfe8135cb4b91a52b570a1",
    "39757857e9b6489fdbbfcc60d2d67c8ce7b76fa4825315c6b8c10d57e2f5ec04a62b5a11bd2eb78ee3508d7cbe5076f6",
    "f1c27ea99ab275cbdaba9e501646594b76995ed1d7d6a3c225f36d76da9a59e5873fa710c4928f17dbd367b634b6b41f",
    "ce9437f477879e86a139fa6f1b63845dbacaaca5e76814e6af821eeb0f7d16213af50ea7619917f557de3725721cb431",
    "33c0073fe78a5f6e8afac77804c15c3d37b32e06ed4819ebe8876eb003f3eb970196e425fad17b0dc91cc5116cdb4172",
    "36733dbaa9968d70c8ef80bddd5cc92cdfa98be889c2a1ef5aed6b0d9a1c04202ca2e7136bb5fca4dda4b44e35157486",
    "fb9a58bf82353843ab8e0b68551289d797080ea4f0e7cd2db1340cf420aff441441c2563327258e20aab8839cb4ef6a2",
    "06da8266efe8092515ea138fc78b188c2f4d4070a4dfcb7f95776bb611a7d9d527552c4fb8ce9a834efb2f8a2568d232",
    "028cd83f711d8c56a21b6cad16ffc5594bcdbf7248710d0eb94273953f5f202e9ccbf44520b4cb52a3af8a83218d757b",
    "7147ed9ce351d4c6fb71ece7db7063b1bf64dcedfc336bc295ac99aec89a6d35d08a156f32f605d2a4c679f7bb8349b4",
    "d87110f76dd80481e7e353b665cc1a02dcb77500d3dd9bec594215bd54ed2603489eb957d2b366a3e1f2eab018e97422",
    "d65bd9f77cd33850815eab680a75deca2e81968cd94d97a790391839371c1d6fcd08d0258be368276a3d6aa4e6dc3da1",
    "ae6c6e855acb561d2d4cb1f74bbb07a52e6659aa24b75e75f1648f50d1dcde1cbec206f7ad931ad706eeb3938c888d31",
    "c9ca178acb7c3588b0edbcbf4cb489d56acf8b3d84739698e980d838d37e1d7e594b00ce794ab7e0a33fc673a92816fa",
    "a01c88b54646b73c0bf8a48f180c53c191588e21380597a0523e0959c408ce5b7a40539ee71428687258fd3a6ae39948",
    "a2c972eb39c7357b7b06358b3e3348f7023c03733ec050b5dda233d769354e444579c4f272c0e01e391ca2863121cf05",
    "8249dcc8e223d4622e4b98743b7c62194457b170b2a09420ae221eed70e14388473344a883ba997951328e950ac17a28",
    "83ffb1b93d763f6e2d1142c13aa47454ee293173bec34584ab1417acfa5d8d2089ca3750e31c7f38e3e9412476bf0aa4",
    "2a161b3a7de93db5f983eb17a24209c78969dc889c15e25dfce937906a30e21e0f726e99ca642c94c357019b08bc7449",
    "e9f31aa09cf9ac5a5342fe96f8a4f066d99b79192ba1fc2e3fa8a76715ba89054f807563f15213b6944d7c1f50fba12a",
    "3aa8e30b7d645fbdbc4011899290ded9283b2e3e9a0801edea4275213250baab1cbf95e44772cadd898e67f2a03dfff0",
    "aff976e51f4458e22569dce4a078bad2eee6321b99b0f93062205befb231170f7a4b1e6aadd3afc566d1a5bb180dbcb3",
    "6d60068560ccf1945595352701900a628016c6dcaa0c96884385123f50f1d7a4ed79733444607cc72e136a178cf918c7",
    "a1a59268d43655c576438a126d50fec020e1e8d4fe9a309b5d3aa5128dca019eb54357edd56d2e2f01abe616bc2b63fe",
    "62804f82305ceca0629b5fbb73e5a1150e484e235e061e7edcfd422eec5657b77dc8fb34c1e6db50b4adee035af516f3",
    "83b8452fc2731edc18445179c504abb4fa8f218ab4f8f4c02b4215a705335ceafd4e50c80cabe3d28d14291062c590bf",
    "e260c0358278fcc9e879132956ab725b7a19f8ce970c21b4ffb99941ed13fbaa0c5019cff29604b5bf393a554a01faaf",
    "9b8cf06b2bc44b8fcdedf950d14f83e2e4559c3672757071201ffb356db75919a91643ad95afd297131077af7c4bc378",
    "b41c5b70f14871327ff97793d9f058eb223405a4bdc5e81357dbecc08f4d3bae6e5af80039ff0caa3fb794f5a8b7475c",
    "be09840041f7c4c8b0942e0c9f96b207f3a41bc9c36995b016e3891cf4c39cdcfb72fc2e90985ceb73c3dc09666415d1",
    "e0c25f6818d7b76eff93f4fd4abc13dc249ac3f38044ab875669b44c8579d935d289c313a145ef0f99df673fa9ff521e",
    "dc10008b663e72701eb6624eb521ddbe32279ff308fb90ec18d97a3df4e5097e245ad644b35c77b0c639cea883b3c970",
    "7e010f040bb54437a7fe5d7af2879244e910199f0d6a476e2899f3cb8d9bf24a1104ea70da460a2a592be8a8c20513ef",
    "07687ce8e10c766125c92d4432dbb65a6b782e6debbfbcb48a162e1cd6451d80d6534556862862bb50bd0874d513634a",
    "ff58f59a57120ca5cc3f8bd16d40d049310d7ea6cdc67196179f94ce8d0394697529e2dcb238065b536736e388c17b3a",
    "0335a23d2178d6e06a8535946793123a774917bb61d613f1d784548c68adbcaf485fd3bc1c40d0c9e6a2f51816553883",
    "fbc7a13e796176e7023887a085c620b89f2c87289ff168b07b025072338fd1d708bb6b4811c5525757c2fe8c06f249ef",
    "4ddebc7c7118b7929ab88fc2f1b879a30416b2792289269a9d770ad3b9ff87bac76ba6cf515ea5f1da968deb5a8581be",
    "77faf184d973d079c5b6290892e90222bf16e4eb926f2849216412d107694e745c8e531376ad11da565a9f4e9b9b5519",
    "a7802c6e365f09f54f929e15559c3812e154a81da26e8aed54b67d55753caf4bbe408264838e22a4686dbd5425a1eff5",
    "b7fbd1cb48183dd8a9003dd34dea786a98f6e367200d1f3e37d6d6e00e465a17d7bab5cfefe491e2114fc98856076df9",
    "26e07938aa84c0577f767e021c30f719cd1eae419102d0277d0c14b4bffb469c41f14de0d0949c1a9f7246b204dcbed5",
    "81637f01f8402cbd02e724193479d4e4b264754aeac25db6bba311dbe12380647e48c08e93303965414da874cd43a19a",
    "b9d3c96316d6844a6647df0e6b43cbb4bd66d268d1cbf6b84a5b097f8d71148f19c910b3cac3ac53ce3923c7af5f478d",
    "ef024fe9f99749d30f56f7685c3511e8a1d9b27ff40a48995fa4902a49889a214062fba5834b0185439f7ed27c2018e9",
    "3333ec608fa4e39df03a6e392141c81caaa53646f4f659fbcb25955282e7e8464eba7d67bc5fe5846ce868999c83152c",
    "b416ebe732a3f3eda35e2f6bd5a9a2ea1696d8b45dcad185fa0adeca5b426210c20003f45f7d644c45bccc099eb2634c",
    "de4cd9952e9707d6a521b5e571751625da0655873675286f999e8d834ac06e353250ee255b7f0518870aaadc21957575",
    "eef41842dd682cbe55c7bf127bcdc610601dd2659cab264671ae23310ea88941a719d1ad43c1cee1ee57f2d4884c52b8",
    "7bc9b4dece93e722143d7d3afdc4800eac17c917f8a1596cfd3a874472f57fc1ef19eeb90969a61eabdedfd80a118c36",
    "6a3cf9ed84830c0e281fc0395b045d4b8017375f3bfae1ce21e58db85f5445ef5f527f9b4360973b8aef556486054449",
    "d72750964415fad3be8db40f71ee8cdee4fb8be7bb551d5ba2bf7f4cc811f5518c9b351d8a6f77474e24c1d3b765e81f",
    "7281d4cf78c8b775b4488fcb879b37ebafc9d3c91209c2aacac82c81e36669d56f2b86e8630f705fb7d9fd0a31affb59",
    "cea034211e136c53809acf509b4e0c377515b6349c8fce6abbcff50133b8ffe77fbe307f4354ea57bf6eb1f4b42a941a",
    "1a814f89d067ca8dd787707075c9eaba6d00a3a9000cddfbfc075cdbe841f6023db18765189359138bd6bd5e515df017",
    "db9e19eeda5bcc9d7fc9abe70e162eadca37cc514d9066caaffa97190d9ff337b1bd01c7f42469238aed1dacdaebac1f",
    "07cd3abae18258aed915b3add52c248e06815dee29774fea0037a32f0dcc2a335baa87523cdb1115461cd5e4325f239d",
    "22a7f401a26f4730942bf421341015a1965478e9917a705e444f5cc6010ba13c1096049cf29a060069c25e9ce05c6b92",
    "befacdcd3a5650201b2d5690f4cb7f66a2f870f5afe16ea445a114615e2f1ba95245baa48da10dd176eb123582bf738a",
    "faca74fb89e9528d32c70395eedee7931aaececd3a6e7ef74a662e5b64d7255494f7d3f1e3f9b3c1adcd1ffd8220c610",
    "6a526e3920d17ed7439928d59091d2fcd0668b90ef5f83cc489037bcef11c382ee4bbecba78be79ca89575a4ac466d13",
    "f896049c4457e627db6f448eafb661fdf38115339d2c1809c8f1087110e93ed1cdaec407f86df32a27f83e89100277ce",
    "2e00f24c86b7f1a5e2c36d8c345007dd8d13998e3964f47501256554f85c988d07cc64fa80c656678e135267b3f4a0a2",
    "7c80544d0b0000ffffffffffffffff9ad1679a2ae0fe091bed13a5eca7633a0483933432e3a3cf9d94be3b1c14ee97f1",
    "f5a9ed09e7960d2fdb94b8986ee8c060718e3ef7f7a644fd43173d6a03d3f94dc00e21fe97d09db1f36c779aa2cd4df1",
    "ee61279bf9ae10f63d8a0370e5b8d756351cc5adfa6e65419300492e811d1e859bf3611f64cd957863e553b623580ebb",
    "b0f096b30590fee25e6521ac54831038f77d2f8c04fff3eda973f17fe8a2b41e84d765e507e32948791a06eb79937adf",
    "3563cb77a38a7aca96f3666b6b4033c41c627e17924484ae4a3c141ef6917dfc988b0aff36eea0a03c11439cbfa3dac8",
    "90038a54e9f0b820b145033d7dbedc2f583a844f8d4c73afba62e8551f8cf5cbc826f7a7f0253854ca422bdd031a5224",
    "f56834a1794d93f437d8c717b911602217c8da5997526df5268b0b98c6b761081b9da2bb828ea3bc5fc02b4afd2c9d8d",
    "5a9055afc4bb72758bfe4be7db77b078fe4885f40c8741b460377d576cbf6a61916c7d30c7a745612fb6438c3f5dd7f2",
    "3398e9b209f5797b6627064cdf1fd342fea4be643f047b2e7448016b5e55bcc512d7d46f25c2b9b39abb37fc5b1ec849",
    "c678c2148e937a8d2877ccee257862858f416b34717049909f5d6da1589135be0536d20cc28e2a3c417027e88eda40fa",
    "9a5c979f92fe3deb8b2c37ff935d934575d3be5b01d62a5318825cf3abd2188ffbd428636d57efd36bf41afe57fc0090",
    "830843177a99871b29577d5133c24848aed27e465636ec9d39add4d34c8ad38ff1e6ec99d079ef5a4eaee750925ea330",
    "c1d6aba7b98f6f765e6b269dc44aae21f1f07b44063867277d944e28747cc447b20e197b4b7c5772abdf138488b11949",
    "c2b8cbc0eee1833a0219395dd155e2fa4834afcfb5a39520d94cac6124f091d7cef239bf127013bb43910fa07d51270f",
    "f5df75cac1202a9986ff4dd7b4fb22a24512823e29f96098f443973131538515b064bc2c0aec9511b04d69f934137932",
    "3c623042f957df51dcacffbf1c7e095b5529be5322cac9e8b406cb6a48d079f44a9c7b1dcf4d624a0d3161c7ed7eb25a",
    "7b83c8b747aba83dc8715dfc8da851eca304ac306537d1a0a259c6796256c5aada0adced8a9ceefdfbc76ad365f711e3",
    "b7ff07284f2b6074fe8aaf1753a1a2742376d72b802bb173b3276608438dbf1c4f2d2cbfecaa40c1863e15d64162195f",
    "5cf715446e534c5da61435c478b3ec802703fd02d2c3d5aae1bdf33f624b4005c10c982cf4e2da77596ae8e783b4c654",
    "10e2040eb6be6f4cb256470e2f84c7df1ddeacf73a662c931eda0b25dae8b46588b15950e247790fd91a9a996ff5cebe",
    "a9190a5fa39136248fe165aa383964bfdbf70149191333762b1102abc51036b26b4583bdf0250f538d423e396b3463d9",
    "bd72dcd0090ce573b2f6aae6bf1a7e850f9c1b91f7605d8cbbfe23cda38833476a3102cd38f992346a71d4a65ef30fa4",
    "4469b880c8dd4f967dd02ebc3917dcec402eae806ea830787ecb61622a5e18949fdc43d46004101a7522c854037a06ac",
    "dd669e40a6955d5c052ed48bb7cedee3f0624a80d1f1bdbe7b88f0021b0e8c9cbc70fcbb761592af3c56d9eebaaa9697",
    "2585299ef4dcd8ae23b8fdec317890af559ac1b703fcdac149d24697911ab521a43db172920141b3b205e81953b310ee",
    "b47f995667cc03b570b8a42c4f4ec8cd29ce44ea110fa6c0a0560b0b3f96fc279a234cd4a1565a54dadf33d74e46c7b2",
    "222085a0a6077b0590a16de82ddfcd181c879dc702eada5ac8986f9d381eee288e96755afbb06f7fd2e27f121cb99328",
    "bac6b5e51cfde14958e7124a94d60db0586a1c0a6176620b6093684fa1c9949500af05402b506cbbbb3e9fc6d797606e",
    "3683ad527cdcd6f256aac56831b35923a4d9ab08039dc231f9b26100ec84e31402d12a836d5ce601f20cf349f027444f",
    "1f4256ce80beb06427b9f44a42fa26579d248a1e84d9bf15138bcda0ff65ab01cf62af4471553d1a2c5d483a5f61a6dd",
    "47855ed56a818e2d335bf906474b66ba8ec9387e02f7dd71f5fa270fe26f9db07e10f60dcc38fbfde60ca795c5dce648",
    "d436b8b1b0d425b53976398b1bf55b2d5c4c07df8e8519fbd408d8125c48cef5532f1a78aee006adb4f88a5c11241777",
    "7005cd9672f5dff53a83248a1c750038131bd591d97d7b70e2238ff6115c476721a5b2b448071b1ee4fafe72120c3cea",
    "0dddd02220a2c2b23eceaaf9bc6d76fe860b73729b94e9a1ab2e794c96fb2bef4ee8c97674861098ff3c300f93699554",
    "c5e6c7bedcbaa4b172958dc41585fc6e8edd354133c6c50854d9ed4e03e4d40bef907ec012445f1aa55adb5361c67581",
    "4fa1ac5d1714247a226d2a1bf6c5f33e61602dda0491d85432d5f65f7cc087637bf9c813a158834841dc3287d98a1bf6",
    "998c4f26e9ffee11078122dde5de6723819f8ed10760078136575633321131149c7aeb7b93d5955013779d7368051fd4",
    "f1dd4e76a105d139771d72aea4e4aac9cd9eb7ed1aa94bedbe6664146517a64097a7110ad7d6b9591eaef55c8201e485",
    "4ad3c2ca0f4b0fb4e8107828a5fe73171d3273058b3e81f1318862d0da20f564cf2f9223232ed3ef683b9e289f3493a1",
    "250f100d0932890b565c55b1028106260c62d2c5585172790533adc14ad03eeee5c36637735607f715cb0c1aa6312299",
    "848ce887aae58aa8e8a7a3e58e8be5b7a5e585b7206c7a3420e5b8a7e8a7a3e7a081e599a8e6b58be8af95e6a0b7e69c",
    "ac203000ff3ebf6c696e652030303135300a5b01ff3e0f50011d058b011f318b01ff781f328b01ff781f338b01ff781f",
    "348b01ff781f358b01ff781f368b01ff781f378b01ff781f388b01ff781f398b01ff771f366e0fff781f366e0fff781f",
    "366e0fff781f366e0fff781f366e0fff781f366e0fff781f366e0fff781f366e0fff781f366e0fff781f366e0fff781f",
    "376e0fff781f376e0fff781f376e0fff781f376e0fff781f376e0fff781f376e0fff781f376e0fff781f376e0fff781f",
    "376e0fff781f376e0fff781f386e0fff781f386e0fff781f386e0fff781f386e0fff781f386e0fff781f386e0fff781f",
    "386e0fff781f386e0fff781f386e0fff781f386e0fff781f396e0fff781f396e0fff781f396e0fff781f396e0fff781f",
    "396e0fff781f396e0fff781f396e0fff781f396e0fff781f396e0fff781f396e0fff772f32306e0fff772f32306e0fff",
    "772f32306e0fff772f32306e0fff772f32306e0fff772f32306e0fff772f32306e0fff772f32306e0fff772f32306e0f",
    "ff772f32306e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f316e",
    "0fff781f316e0fff781f316e0fff781f316e0fff781f326e0fff781f326e0fff781f326e0fff781f326e0fff781f326e",
    "0fff781f326e0fff781f326e0fff781f326e0fff781f326e0fff781f326e0fff781f336e0fff781f336e0fff781f336e",
    "0fff781f336e0fff781f336e0fff781f336e0fff781f336e0fff781f336e0fff781f336e0fff781f336e0fff781f346e",
    "0fff781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f346e",
    "0fff781f346e0fff780f4c9aff782f32356e0fff781f356e0fff781f356e0fff781f356e0fff781f356e0fff781f356e",
    "0fff781f356e0fff781f356e0fff781f356e0fff780f4c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9a",
    "ff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9a",
    "ff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9a",
    "ff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9a",
    "ff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9a",
    "ff781f324c9aff781f324c9aff7750303239390a00000000eaa446b2",
);

const FIX_BD_B4: &str = concat!(
    "04224d1844405e7a1c0000ff21e887aae58aa8e8a7a3e58e8be5b7a5e585b7206c7a3420e5b8a7e8a7a3e7a081e599a8",
    "e6b58be8af95e6a0b7e69cac203000ff3ebf6c696e652030303030300a5b01ff3e0f50011d058b011f318b01ff781f32",
    "8b01ff781f338b01ff781f348b01ff781f358b01ff781f368b01ff781f378b01ff781f388b01ff781f398b01ff771f31",
    "6e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f31",
    "6e0fff781f316e0fff781f326e0fff781f326e0fff781f326e0fff781f326e0fff781f326e0fff781f326e0fff781f32",
    "6e0fff781f326e0fff781f326e0fff781f326e0fff781f336e0fff781f336e0fff781f336e0fff781f336e0fff781f33",
    "6e0fff781f336e0fff781f336e0fff781f336e0fff781f336e0fff781f336e0fff781f346e0fff781f346e0fff781f34",
    "6e0fff781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f35",
    "6e0fff781f356e0fff781f356e0fff781f356e0fff781f356e0fff781f356e0fff781f356e0fff781f356e0fff781f35",
    "6e0fff781f356e0fff781f366e0fff781f366e0fff781f366e0fff781f366e0fff781f366e0fff781f366e0fff781f36",
    "6e0fff781f366e0fff781f366e0fff781f366e0fff781f376e0fff781f376e0fff781f376e0fff781f376e0fff781f37",
    "6e0fff781f376e0fff781f376e0fff781f376e0fff781f376e0fff781f376e0fff781f386e0fff781f386e0fff781f38",
    "6e0fff781f386e0fff781f386e0fff781f386e0fff781f386e0fff781f386e0fff781f386e0fff781f386e0fff781f39",
    "6e0fff781f396e0fff781f396e0fff781f396e0fff781f396e0fff781f396e0fff781f396e0fff781f396e0fff781f39",
    "6e0fff781f396e0fff772f31306e0fff772f31306e0fff772f31306e0fff772f31306e0fff772f31306e0fff772f3130",
    "6e0fff772f31306e0fff772f31306e0fff772f31306e0fff772f31306e0fff780f4c9aff781f314c9aff781f314c9aff",
    "781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff",
    "781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff",
    "781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff",
    "781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff",
    "781f314c9aff781f314c9aff781f314c9aff781f314c9aff78f0ffffffffffffffffffffffffffffffffffffffffffff",
    "ffff9b3134390a10a625a1eaef065248d7e1900a4df3391f04b7bfd7ee70dfe7537dc29c2188378509eb5db269c1d9bc",
    "bcddf031bc951603661fb1fdfe183490f41c4a608ad7c30c1f4c76ca7ead6af35ffd792bc8fa6d48079f48b90a216fd4",
    "a75877da750703e5ec5d104e01b4ce10720da0735d06090df58ae1273eec61868d0a8945ad445329b6e5e9882c1f2f18",
    "01d51f17a3c4284d71a7d4c0266ce832a4217a68c55e3b6d0b8b40edb09a8b4092ec024f320112c234a4a291fc64a4a4",
    "22ad496f85a747eb3e7c3f8d45adb3a8c4012d8ec26a304d2bfc99ec7a2034e6596fc03ce04f6624699fe50abbb9c2a9",
    "73ab661c5b2cabd8bf6c009e2ab6ebce15e8c3426044bd05ef406a0d8c77159557b4de2a060a7ebe8ddabfa83bc30ffb",
    "83de12d11a8b058d7d393039ecf6c3fc25e63961000312acafc2876fcaa38a5f8b7399f61f1e5ef488a824a25b129c73",
    "325a18016965570b79297e72bd342b45b3aa31c66b4db04b5210acc2b4789ad89243256d767560e1bb0266840b1159d0",
    "8678f23857937ba6691210edc220e017b83b93462282743781224478944d4e056a51f47b8cfd38beaa9060474d482e70",
    "9a1333fc5d88b1fe56a88e5e6eb2d34e0c349dc770fd2c15bd62de38abf4e13a511bf2e6c14a1c7603edcf35b57d7282",
    "393867b71ad0c5a11b7eea62d5eb63293af2ead5c0f44111d1efd31ee09e386444982c18c35471e98ea56615f4332d13",
    "93d5e72b552f47d74a3f4f158f6fe5afe442ed43b669f8b03193a443730c3937a568716c35c78c4d7a6f0923dff7ac95",
    "96e31af0bfcc5317f0aacbfb9372fffd5b03535883c4bcdaaffc851435a469c87f765e8b22760a448c20c3547714ed42",
    "6a84aa7ffd056daabbc029ac978858fdca4ce70a057604831aab9bebb7d55d15ee966550af623417b6d8e75d771753f2",
    "8e9f2b416651ebf103ae74d374cb15038e8256b8234441883d186b79066387f22a9a38b589dffc9d87e6eb0de4b439d7",
    "1b7bb82e16c676db33f4bceb05daafdd29ea77f7eba3652cbf9a007966fdc8e4b269364ea150034904ed6b691684caa6",
    "a2c7fa73b2b01d031a427e51153692c45b3a77eaf2b37cca65803b298dbb5a6117853fc79dff3ae761b9e816d314b4a3",
    "2eaa27a177c971f894914818c06f16c0dc9f7f997951cf2338e9de0ddef091fdf0947df6968f8c7b195ab190dcde2820",
    "f14373ea0d87412f1ef70930e2bd48eb35c045e0d4c425e40ad7c1862fd2e30568619fef8e8424d4eef082a1898caee3",
    "04286ee391055f1cc9b1994a005ffe2741371b618d878ed1e0f6bba58884cbfef2d10ba3316bbd4245b058b1db214ef2",
    "d25fda546f09f6ff0ceee9143662ccbcca0ce1fbcfa73f13bca1c3d164f5f98ca15a758d541a89f867aeec4d9b75824b",
    "a84e708f719df6ccfdca7134aa3b375ccb9f7457913e053948a5bbb2ff1c594c9f7379e4b487239626d4708df48e860d",
    "de2515d59bc814cb670e4995fcbfcf16d5930af3038ccb63e93573ed15066413437697dc7bba1d645e9befb21252707c",
    "2949074042dfd456492374773af189b315340442be22a90faa9e5622b34f4e1350852f6c025f9babddfce69840129e82",
    "8c2e75bae6f42f502cbde0d1d730f9f578d5aa4c32ae0c18923655b87f5e7971d4e7e2235c6979c72d1992720c74f908",
    "6936108052d341b1eacb94722037d24c78a5655edfdd755adaff78f40a11c6c1245af1821e559454be246847ec2d81c3",
    "3bfe06a7782a90d546139679be7d456c0780df43c4d93b867f8babd69a3532ee7de80466e8869110f0fc49c8e121b8ed",
    "6db5f737953c64dcf513fa8ca76ea1501b3f9c8499deef96be953544045c4309cda9e9f23436c7e08606dadfa156535e",
    "cfd9554412b8adc87f95ae6027fcda275efb853a466fc674eed564fef98ccc851410c692e68bb185f1c09e97c33fc198",
    "3603bd9aa82401bb87827008560b4eac84cde1de249d9c4b3c91efda604f835dec203478193d73780a8d36c366e90b3c",
    "ba1abc6d4b6327ddf46a768da43a6a68b78845ae72f108f9ca6581cc2c8fe2a3b42bd92fb85af86c9f3f5cf7e5777863",
    "11908a9951dcb6787a4d5052d47e9859b1ebee159966fd35afc301393adde2f5db7de9b14fa118026fd3034503859066",
    "970b32e45baf44ad08156fc30923078af6d11b9fac0586db5cb40310a7810466ce8f208ca0fe5521ffee3a471ddbeb90",
    "6a15b3d0868caf639750eccc4a99be11a6dac7fdac9b0bdedc4dee3133f22e460726aa8a8497b07cd99134f5e1fd523b",
    "373a8e7957a6ebd4d7a5f79a0da884fa830866840cffac6e7e5d579c201fd75cbef77c6f7ff8ffbca28a08bc0313b4cf",
    "1d2b46f5ee34eb4d49807a2346761c9683df07c8f68030bd5fdc07ea12170805bf3ea7309dda6cdca01a9e6d73b46837",
    "395357c2f81e1c912a75750470e74dbb9a776fb4dae403f5508d3d927689adc0dcd413451800712a1459556a9afd4b36",
    "4f6617c5e533ee6cd2e67f1720de446b1511a7b3b281c5d7a75d9974e79ebba4883c317b31b3f17201d1d8a4138e262d",
    "1a731043dc83f0e7e75b066499cfc2641e9c765e96f5ef7f69ff3a3414aaa23d0f2a21d1d951e4d3d1d9a1dd6dd9e9c6",
    "bae54876fc5100b5fa17b8cedc329b19e8b8503705f508dc5f3997d7ad429955f104c1e2917f0ec15621b2ba69503d0a",
    "c704fe1e4b1b0e380666a517bf4b119cecba3bee79b4eb4688757c34cf1c3529e2e13e59166c4eb0b9fe011e39ecdd65",
    "856f56a50044eabf492689a28ad072e7de1e1fb3bd6a9ec96906c5a37056d684e57c83e947b6fbe4c2f214494c8c520e",
    "b510784778cdac66070dd78889e6941caff90e057a6c418fcca947835090644715a42c58d263d8f302e44f4a0aad6b60",
    "9408a4c67db62c21af21e5702efba0213fe8f6a0976987014ec066fdec66e1d97fa5610c1f75126a68a0743325031c93",
    "6e18b12c456a0674def0d8ba2ef5ab302ff352e2cc36480763c7e39cf2b94704b02f22a9009cda20aa05d49cde64236a",
    "5d05850fbbd0bb4fc302b9691f41a4ce4dee404100be9ef52f825f2cb46140b7592f8e33a37f0ab41f75b7efd391ff44",
    "a76ffee985725aa06d05425d1d3a15ae2cd39051e6831da6c166045a26ad2a36982c933b441c5baf6cf86ffece58c01a",
    "3ba9c8ed4a3d3efc6e2cd95bed6435f8569344ca5b40913b3bbbedaed240ee376594949d23c9b5d7a19ea45e109024e3",
    "d40baaf4c15d620771553d56200ecc7fa8e7fe1f0621de094efbb54158ce2b5ea089803c394a0529309d4816aa7183e9",
    "3e38c2ecfdb2c3f5215d6888c1be67634897e919bf165e2fa1e6c5c3ea2a31af3eaadb53358035fa44a0ca124dc21681",
    "39e338507b6052c1fc3323d3fa086297c2ba89042fc166559bd5c54e4f3450621e4d3cc93339a5c902ddf2db206309b3",
    "828eda2f72f600858628c7ef432c41d3c47405de3d7d4d1604c1c97be11ff8a8f8bfb513bc8fbf332654ee241cb5df44",
    "7dbf3437ecb5518260f0b4da8f21da7602a96214b2018a94178daf49139128d1f9f1ba1d826c219983dc1c9a6053acb5",
    "ff36925413690c51b7eafc1efc6fd7c6b130294ca61669b86b012a47fa1cb665762deac165a4cee8e558f57f12ae9ea2",
    "be957b4c4e6274b8a714ba558461022730fb0ab02df1cd95410308914f97e09f453659433423fa12d9b3ba8e3afd5a0e",
    "df0516ee980898b6ed44a080351ae9b44cb9e74bc7a28281b68d1b0a7ec5bbda3b72c359b1bee3add10c4caf260ca71f",
    "265b09499b823629871435a363fd51c2a778d9ef101aa44a58d5686e30e3e3634c7d303f591a403c2aa4b2ecc75df2ba",
    "4d2b436c0dfbacabb30d472c62f4fcd1c2b8a12232539d209336f6a2dc871233c2c793556f2bbfa59c02d93598301b90",
    "f7613f30d0057c12d08a0aa73f084065277f0e959e072aa69251d8dbd160f21a289cd3c3c6d5134c85d3026f85d51e13",
    "cbcc338c5392e31e9ed06f34fee1f2483be8d4a87f8dfba8eee8eafe4e46d9cd4835c6a8ca171d5d93fe624d52e704d6",
    "2828b4f5b30205ca65eb2955d59324b828a0567974485f01cef0b8da1430c063d337a459586699c15c7d716c02d4aed2",
    "051c4a4723d524d37abbe774f14a2cfc73efe1f60d31892ae46524a6fb7c1fb828326622c48eee314b657745cb4cef1f",
    "6eb87ab412726ddca8c03cc432558ce9adadd87f86fde0f1c64d2e420b130d3ebc979c16b6b67c0776aeb75dfbfe758c",
    "22edbe449286dcc5fe1ebde173001ec7cab65890485ee4d9377e7dd199e83cb6a9ae2e6739021324d637e0496d340aa7",
    "d778ef5b8782a4a8765bcebdd1d51a2d9859c3faa7db26abc19a100cdc4f404ed8fe97c89646e4935f7925f3fbd0a1af",
    "9acea5c511abbc02055aa6596fa86d39d73dc81b6ac1cfa642c5a3ee92afadb752b50d5a69599a45798688b27212bbf1",
    "d88103d9be51eb83900dfcc5440edfbe7a4755a88cb33de4ca984d221d12849334841ca27f7df2835fafd8a387cb9807",
    "859d7d14f691f30b385fedef659e8cd67df20184c2522e606bc8c4b8a60671dcbd7d350bf26b7689db6fe4d44f5dbe8c",
    "e27d06d0343d4c5396df97b35399335beba765174a7b002d5e0eddb0bf6061ad0b66ab6f096bbbbfe8135cb4b91a52b5",
    "70a139757857e9b6489fdbbfcc60d2d67c8ce7b76fa4825315c6b8c10d57e2f5ec04a62b5a11bd2eb78ee3508d7cbe50",
    "76f6f1c27ea99ab275cbdaba9e501646594b76995ed1d7d6a3c225f36d76da9a59e5873fa710c4928f17dbd367b634b6",
    "b41fce9437f477879e86a139fa6f1b63845dbacaaca5e76814e6af821eeb0f7d16213af50ea7619917f557de3725721c",
    "b43133c0073fe78a5f6e8afac77804c15c3d37b32e06ed4819ebe8876eb003f3eb970196e425fad17b0dc91cc5116cdb",
    "417236733dbaa9968d70c8ef80bddd5cc92cdfa98be889c2a1ef5aed6b0d9a1c04202ca2e7136bb5fca4dda4b44e3515",
    "7486fb9a58bf82353843ab8e0b68551289d797080ea4f0e7cd2db1340cf420aff441441c2563327258e20aab8839cb4e",
    "f6a206da8266efe8092515ea138fc78b188c2f4d4070a4dfcb7f95776bb611a7d9d527552c4fb8ce9a834efb2f8a2568",
    "d232028cd83f711d8c56a21b6cad16ffc5594bcdbf7248710d0eb94273953f5f202e9ccbf44520b4cb52a3af8a83218d",
    "757b7147ed9ce351d4c6fb71ece7db7063b1bf64dcedfc336bc295ac99aec89a6d35d08a156f32f605d2a4c679f7bb83",
    "49b4d87110f76dd80481e7e353b665cc1a02dcb77500d3dd9bec594215bd54ed2603489eb957d2b366a3e1f2eab018e9",
    "7422d65bd9f77cd33850815eab680a75deca2e81968cd94d97a790391839371c1d6fcd08d0258be368276a3d6aa4e6dc",
    "3da1ae6c6e855acb561d2d4cb1f74bbb07a52e6659aa24b75e75f1648f50d1dcde1cbec206f7ad931ad706eeb3938c88",
    "8d31c9ca178acb7c3588b0edbcbf4cb489d56acf8b3d84739698e980d838d37e1d7e594b00ce794ab7e0a33fc673a928",
    "16faa01c88b54646b73c0bf8a48f180c53c191588e21380597a0523e0959c408ce5b7a40539ee71428687258fd3a6ae3",
    "9948a2c972eb39c7357b7b06358b3e3348f7023c03733ec050b5dda233d769354e444579c4f272c0e01e391ca2863121",
    "cf058249dcc8e223d4622e4b98743b7c62194457b170b2a09420ae221eed70e14388473344a883ba997951328e950ac1",
    "7a2883ffb1b93d763f6e2d1142c13aa47454ee293173bec34584ab1417acfa5d8d2089ca3750e31c7f38e3e9412476bf",
    "0aa42a161b3a7de93db5f983eb17a24209c78969dc889c15e25dfce937906a30e21e0f726e99ca642c94c357019b08bc",
    "7449e9f31aa09cf9ac5a5342fe96f8a4f066d99b79192ba1fc2e3fa8a76715ba89054f807563f15213b6944d7c1f50fb",
    "a12a3aa8e30b7d645fbdbc4011899290ded9283b2e3e9a0801edea4275213250baab1cbf95e44772cadd898e67f2a03d",
    "fff0aff976e51f4458e22569dce4a078bad2eee6321b99b0f93062205befb231170f7a4b1e6aadd3afc566d1a5bb180d",
    "bcb36d60068560ccf1945595352701900a628016c6dcaa0c96884385123f50f1d7a4ed79733444607cc72e136a178cf9",
    "18c7a1a59268d43655c576438a126d50fec020e1e8d4fe9a309b5d3aa5128dca019eb54357edd56d2e2f01abe616bc2b",
    "63fe62804f82305ceca0629b5fbb73e5a1150e484e235e061e7edcfd422eec5657b77dc8fb34c1e6db50b4adee035af5",
    "16f383b8452fc2731edc18445179c504abb4fa8f218ab4f8f4c02b4215a705335ceafd4e50c80cabe3d28d14291062c5",
    "90bfe260c0358278fcc9e879132956ab725b7a19f8ce970c21b4ffb99941ed13fbaa0c5019cff29604b5bf393a554a01",
    "faaf9b8cf06b2bc44b8fcdedf950d14f83e2e4559c3672757071201ffb356db75919a91643ad95afd297131077af7c4b",
    "c378b41c5b70f14871327ff97793d9f058eb223405a4bdc5e81357dbecc08f4d3bae6e5af80039ff0caa3fb794f5a8b7",
    "475cbe09840041f7c4c8b0942e0c9f96b207f3a41bc9c36995b016e3891cf4c39cdcfb72fc2e90985ceb73c3dc096664",
    "15d1e0c25f6818d7b76eff93f4fd4abc13dc249ac3f38044ab875669b44c8579d935d289c313a145ef0f99df673fa9ff",
    "521edc10008b663e72701eb6624eb521ddbe32279ff308fb90ec18d97a3df4e5097e245ad644b35c77b0c639cea883b3",
    "c9707e010f040bb54437a7fe5d7af2879244e910199f0d6a476e2899f3cb8d9bf24a1104ea70da460a2a592be8a8c205",
    "13ef07687ce8e10c766125c92d4432dbb65a6b782e6debbfbcb48a162e1cd6451d80d6534556862862bb50bd0874d513",
    "634aff58f59a57120ca5cc3f8bd16d40d049310d7ea6cdc67196179f94ce8d0394697529e2dcb238065b536736e388c1",
    "7b3a0335a23d2178d6e06a8535946793123a774917bb61d613f1d784548c68adbcaf485fd3bc1c40d0c9e6a2f5181655",
    "3883fbc7a13e796176e7023887a085c620b89f2c87289ff168b07b025072338fd1d708bb6b4811c5525757c2fe8c06f2",
    "49ef4ddebc7c7118b7929ab88fc2f1b879a30416b2792289269a9d770ad3b9ff87bac76ba6cf515ea5f1da968deb5a85",
    "81be77faf184d973d079c5b6290892e90222bf16e4eb926f2849216412d107694e745c8e531376ad11da565a9f4e9b9b",
    "5519a7802c6e365f09f54f929e15559c3812e154a81da26e8aed54b67d55753caf4bbe408264838e22a4686dbd5425a1",
    "eff5b7fbd1cb48183dd8a9003dd34dea786a98f6e367200d1f3e37d6d6e00e465a17d7bab5cfefe491e2114fc9885607",
    "6df926e07938aa84c0577f767e021c30f719cd1eae419102d0277d0c14b4bffb469c41f14de0d0949c1a9f7246b204dc",
    "bed581637f01f8402cbd02e724193479d4e4b264754aeac25db6bba311dbe12380647e48c08e93303965414da874cd43",
    "a19ab9d3c96316d6844a6647df0e6b43cbb4bd66d268d1cbf6b84a5b097f8d71148f19c910b3cac3ac53ce3923c7af5f",
    "478def024fe9f99749d30f56f7685c3511e8a1d9b27ff40a48995fa4902a49889a214062fba5834b0185439f7ed27c20",
    "18e93333ec608fa4e39df03a6e392141c81caaa53646f4f659fbcb25955282e7e8464eba7d67bc5fe5846ce868999c83",
    "152cb416ebe732a3f3eda35e2f6bd5a9a2ea1696d8b45dcad185fa0adeca5b426210c20003f45f7d644c45bccc099eb2",
    "634cde4cd9952e9707d6a521b5e571751625da0655873675286f999e8d834ac06e353250ee255b7f0518870aaadc2195",
    "7575eef41842dd682cbe55c7bf127bcdc610601dd2659cab264671ae23310ea88941a719d1ad43c1cee1ee57f2d4884c",
    "52b87bc9b4dece93e722143d7d3afdc4800eac17c917f8a1596cfd3a874472f57fc1ef19eeb90969a61eabdedfd80a11",
    "8c366a3cf9ed84830c0e281fc0395b045d4b8017375f3bfae1ce21e58db85f5445ef5f527f9b4360973b8aef55648605",
    "4449d72750964415fad3be8db40f71ee8cdee4fb8be7bb551d5ba2bf7f4cc811f5518c9b351d8a6f77474e24c1d3b765",
    "e81f7281d4cf78c8b775b4488fcb879b37ebafc9d3c91209c2aacac82c81e36669d56f2b86e8630f705fb7d9fd0a31af",
    "fb59cea034211e136c53809acf509b4e0c377515b6349c8fce6abbcff50133b8ffe77fbe307f4354ea57bf6eb1f4b42a",
    "941a1a814f89d067ca8dd787707075c9eaba6d00a3a9000cddfbfc075cdbe841f6023db18765189359138bd6bd5e515d",
    "f017db9e19eeda5bcc9d7fc9abe70e162eadca37cc514d9066caaffa97190d9ff337b1bd01c7f42469238aed1dacdaeb",
    "ac1f07cd3abae18258aed915b3add52c248e06815dee29774fea0037a32f0dcc2a335baa87523cdb1115461cd5e4325f",
    "239d22a7f401a26f4730942bf421341015a1965478e9917a705e444f5cc6010ba13c1096049cf29a060069c25e9ce05c",
    "6b92befacdcd3a5650201b2d5690f4cb7f66a2f870f5afe16ea445a114615e2f1ba95245baa48da10dd176eb123582bf",
    "738afaca74fb89e9528d32c70395eedee7931aaececd3a6e7ef74a662e5b64d7255494f7d3f1e3f9b3c1adcd1ffd8220",
    "c6106a526e3920d17ed7439928d59091d2fcd0668b90ef5f83cc489037bcef11c382ee4bbecba78be79ca89575a4ac46",
    "6d13f896049c4457e627db6f448eafb661fdf38115339d2c1809c8f1087110e93ed1cdaec407f86df32a27f83e891002",
    "77ce2e00f24c86b7f1a5e2c36d8c345007dd8d13998e3964f47501256554f85c988d07cc64fa80c656678e135267b3f4",
    "a0a27c80544f0b0000ffffffffffffffff9ad1679a2ae0fe091bed13a5eca7633a0483933432e3a3cf9d94be3b1c14ee",
    "97f1f5a9ed09e7960d2fdb94b8986ee8c060718e3ef7f7a644fd43173d6a03d3f94dc00e21fe97d09db1f36c779aa2cd",
    "4df1ee61279bf9ae10f63d8a0370e5b8d756351cc5adfa6e65419300492e811d1e859bf3611f64cd957863e553b62358",
    "0ebbb0f096b30590fee25e6521ac54831038f77d2f8c04fff3eda973f17fe8a2b41e84d765e507e32948791a06eb7993",
    "7adf3563cb77a38a7aca96f3666b6b4033c41c627e17924484ae4a3c141ef6917dfc988b0aff36eea0a03c11439cbfa3",
    "dac890038a54e9f0b820b145033d7dbedc2f583a844f8d4c73afba62e8551f8cf5cbc826f7a7f0253854ca422bdd031a",
    "5224f56834a1794d93f437d8c717b911602217c8da5997526df5268b0b98c6b761081b9da2bb828ea3bc5fc02b4afd2c",
    "9d8d5a9055afc4bb72758bfe4be7db77b078fe4885f40c8741b460377d576cbf6a61916c7d30c7a745612fb6438c3f5d",
    "d7f23398e9b209f5797b6627064cdf1fd342fea4be643f047b2e7448016b5e55bcc512d7d46f25c2b9b39abb37fc5b1e",
    "c849c678c2148e937a8d2877ccee257862858f416b34717049909f5d6da1589135be0536d20cc28e2a3c417027e88eda",
    "40fa9a5c979f92fe3deb8b2c37ff935d934575d3be5b01d62a5318825cf3abd2188ffbd428636d57efd36bf41afe57fc",
    "0090830843177a99871b29577d5133c24848aed27e465636ec9d39add4d34c8ad38ff1e6ec99d079ef5a4eaee750925e",
    "a330c1d6aba7b98f6f765e6b269dc44aae21f1f07b44063867277d944e28747cc447b20e197b4b7c5772abdf138488b1",
    "1949c2b8cbc0eee1833a0219395dd155e2fa4834afcfb5a39520d94cac6124f091d7cef239bf127013bb43910fa07d51",
    "270ff5df75cac1202a9986ff4dd7b4fb22a24512823e29f96098f443973131538515b064bc2c0aec9511b04d69f93413",
    "79323c623042f957df51dcacffbf1c7e095b5529be5322cac9e8b406cb6a48d079f44a9c7b1dcf4d624a0d3161c7ed7e",
    "b25a7b83c8b747aba83dc8715dfc8da851eca304ac306537d1a0a259c6796256c5aada0adced8a9ceefdfbc76ad365f7",
    "11e3b7ff07284f2b6074fe8aaf1753a1a2742376d72b802bb173b3276608438dbf1c4f2d2cbfecaa40c1863e15d64162",
    "195f5cf715446e534c5da61435c478b3ec802703fd02d2c3d5aae1bdf33f624b4005c10c982cf4e2da77596ae8e783b4",
    "c65410e2040eb6be6f4cb256470e2f84c7df1ddeacf73a662c931eda0b25dae8b46588b15950e247790fd91a9a996ff5",
    "cebea9190a5fa39136248fe165aa383964bfdbf70149191333762b1102abc51036b26b4583bdf0250f538d423e396b34",
    "63d9bd72dcd0090ce573b2f6aae6bf1a7e850f9c1b91f7605d8cbbfe23cda38833476a3102cd38f992346a71d4a65ef3",
    "0fa44469b880c8dd4f967dd02ebc3917dcec402eae806ea830787ecb61622a5e18949fdc43d46004101a7522c854037a",
    "06acdd669e40a6955d5c052ed48bb7cedee3f0624a80d1f1bdbe7b88f0021b0e8c9cbc70fcbb761592af3c56d9eebaaa",
    "96972585299ef4dcd8ae23b8fdec317890af559ac1b703fcdac149d24697911ab521a43db172920141b3b205e81953b3",
    "10eeb47f995667cc03b570b8a42c4f4ec8cd29ce44ea110fa6c0a0560b0b3f96fc279a234cd4a1565a54dadf33d74e46",
    "c7b2222085a0a6077b0590a16de82ddfcd181c879dc702eada5ac8986f9d381eee288e96755afbb06f7fd2e27f121cb9",
    "9328bac6b5e51cfde14958e7124a94d60db0586a1c0a6176620b6093684fa1c9949500af05402b506cbbbb3e9fc6d797",
    "606e3683ad527cdcd6f256aac56831b35923a4d9ab08039dc231f9b26100ec84e31402d12a836d5ce601f20cf349f027",
    "444f1f4256ce80beb06427b9f44a42fa26579d248a1e84d9bf15138bcda0ff65ab01cf62af4471553d1a2c5d483a5f61",
    "a6dd47855ed56a818e2d335bf906474b66ba8ec9387e02f7dd71f5fa270fe26f9db07e10f60dcc38fbfde60ca795c5dc",
    "e648d436b8b1b0d425b53976398b1bf55b2d5c4c07df8e8519fbd408d8125c48cef5532f1a78aee006adb4f88a5c1124",
    "17777005cd9672f5dff53a83248a1c750038131bd591d97d7b70e2238ff6115c476721a5b2b448071b1ee4fafe72120c",
    "3cea0dddd02220a2c2b23eceaaf9bc6d76fe860b73729b94e9a1ab2e794c96fb2bef4ee8c97674861098ff3c300f9369",
    "9554c5e6c7bedcbaa4b172958dc41585fc6e8edd354133c6c50854d9ed4e03e4d40bef907ec012445f1aa55adb5361c6",
    "75814fa1ac5d1714247a226d2a1bf6c5f33e61602dda0491d85432d5f65f7cc087637bf9c813a158834841dc3287d98a",
    "1bf6998c4f26e9ffee11078122dde5de6723819f8ed10760078136575633321131149c7aeb7b93d5955013779d736805",
    "1fd4f1dd4e76a105d139771d72aea4e4aac9cd9eb7ed1aa94bedbe6664146517a64097a7110ad7d6b9591eaef55c8201",
    "e4854ad3c2ca0f4b0fb4e8107828a5fe73171d3273058b3e81f1318862d0da20f564cf2f9223232ed3ef683b9e289f34",
    "93a1250f100d0932890b565c55b1028106260c62d2c5585172790533adc14ad03eeee5c36637735607f715cb0c1aa631",
    "2299848ce887aae58aa8e8a7a3e58e8be5b7a5e585b7206c7a3420e5b8a7e8a7a3e7a081e599a8e6b58be8af95e6a0b7",
    "e69cac203000ff3e8f6c696e65203030314cbaff781f314cbaff781f314cbaff781f314cbaff781f314cbaff782f3135",
    "266dff783f35360a9d0aff3e0f50011d05cd0a0f266dff781f35266dff780f4cbaff781f314cbaff781f314cbaff781f",
    "314cbaff782f3136947cff780f4cbaff782f31366e0fff780f4cbaff781f314cbaff781f314cbaff781f314cbaff781f",
    "314cbaff781f314cbaff782f3137028cff780f4cbaff781f314cbaff782f31376e0fff780f4cbaff781f314cbaff781f",
    "314cbaff781f314cbaff781f314cbaff783f3138319326ff771f386e0fff780f4cbaff781f314cbaff781f314cbaff78",
    "1f314cbaff782f31384a2eff781f384a2eff782f3839580cff770f4cbaff781f314cbaff781f314cbaff781f314cbaff",
    "781f314cbaff781f314cbaff782f3139b83dff781f396e0fff780f4cbaff781f314cbaff781f324cbaff781f324cbaff",
    "781f324cbaff781f324cbaff781f324cbaff781f324cbaff781f324cbaff781f324cbaff781f324cbaff781f324cbaff",
    "781f324cbaff781f324cbaff782f32314a2eff771f324cbaff781f324cbaff782f3231b83dff771f324cbaff781f324c",
    "baff781f324cbaff781f324cbaff782f323228d9ff772f3232b83dff771f324cbaff781f324cbaff783f3232340136ff",
    "761f324cbaff782f32324a2eff771f324cbaff781f324cbaff781f324cbaff782f32336e0fff780f4cbaff781f324cba",
    "ff781f324cbaff781f324cbaff782f3233dc1eff780f4cbaff782f3233b83dff772f3233264dff772f3233264dff772f",
    "32346e0fff780f4cbaff781f324cbaff782f3234707bff771f324cbaff782f32346e0fff780f4cbaff781f324cbaff78",
    "2f32346e0fff781f346e0fff780f4c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff78",
    "1f324c9aff782f3235dc1eff780f4c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff78",
    "1f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff78",
    "1f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff78",
    "1f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff78",
    "1f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff78",
    "1f324c9aff781f324c9aff781f324c9aff7750303239390a00000000eaa446b2",
);

const FIX_LEGACY: &str = concat!(
    "02214c18c4270000ff21e887aae58aa8e8a7a3e58e8be5b7a5e585b7206c7a3420e5b8a7e8a7a3e7a081e599a8e6b58b",
    "e8af95e6a0b7e69cac203000ff3ebf6c696e652030303030300a5b01ff3e0f50011d058b011f318b01ff781f328b01ff",
    "781f338b01ff781f348b01ff781f358b01ff781f368b01ff781f378b01ff781f388b01ff781f398b01ff771f316e0fff",
    "781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff",
    "781f316e0fff781f326e0fff781f326e0fff781f326e0fff781f326e0fff781f326e0fff781f326e0fff781f326e0fff",
    "781f326e0fff781f326e0fff781f326e0fff781f336e0fff781f336e0fff781f336e0fff781f336e0fff781f336e0fff",
    "781f336e0fff781f336e0fff781f336e0fff781f336e0fff781f336e0fff781f346e0fff781f346e0fff781f346e0fff",
    "781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f346e0fff781f356e0fff",
    "781f356e0fff781f356e0fff781f356e0fff781f356e0fff781f356e0fff781f356e0fff781f356e0fff781f356e0fff",
    "781f356e0fff781f366e0fff781f366e0fff781f366e0fff781f366e0fff781f366e0fff781f366e0fff781f366e0fff",
    "781f366e0fff781f366e0fff781f366e0fff781f376e0fff781f376e0fff781f376e0fff781f376e0fff781f376e0fff",
    "781f376e0fff781f376e0fff781f376e0fff781f376e0fff781f376e0fff781f386e0fff781f386e0fff781f386e0fff",
    "781f386e0fff781f386e0fff781f386e0fff781f386e0fff781f386e0fff781f386e0fff781f386e0fff781f396e0fff",
    "781f396e0fff781f396e0fff781f396e0fff781f396e0fff781f396e0fff781f396e0fff781f396e0fff781f396e0fff",
    "781f396e0fff772f31306e0fff772f31306e0fff772f31306e0fff772f31306e0fff772f31306e0fff772f31306e0fff",
    "772f31306e0fff772f31306e0fff772f31306e0fff772f31306e0fff780f4c9aff781f314c9aff781f314c9aff781f31",
    "4c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f31",
    "4c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f31",
    "4c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f31",
    "4c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f314c9aff781f31",
    "4c9aff781f314c9aff781f314c9aff781f314c9aff78ffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ffffffffffffff453134390a10a625a1eaef065248d7e1900a4df3391f04b7bfd7ee70dfe7537dc29c2188378509eb5d",
    "b269c1d9bcbcddf031bc951603661fb1fdfe183490f41c4a608ad7c30c1f4c76ca7ead6af35ffd792bc8fa6d48079f48",
    "b90a216fd4a75877da750703e5ec5d104e01b4ce10720da0735d06090df58ae1273eec61868d0a8945ad445329b6e5e9",
    "882c1f2f1801d51f17a3c4284d71a7d4c0266ce832a4217a68c55e3b6d0b8b40edb09a8b4092ec024f320112c234a4a2",
    "91fc64a4a422ad496f85a747eb3e7c3f8d45adb3a8c4012d8ec26a304d2bfc99ec7a2034e6596fc03ce04f6624699fe5",
    "0abbb9c2a973ab661c5b2cabd8bf6c009e2ab6ebce15e8c3426044bd05ef406a0d8c77159557b4de2a060a7ebe8ddabf",
    "a83bc30ffb83de12d11a8b058d7d393039ecf6c3fc25e63961000312acafc2876fcaa38a5f8b7399f61f1e5ef488a824",
    "a25b129c73325a18016965570b79297e72bd342b45b3aa31c66b4db04b5210acc2b4789ad89243256d767560e1bb0266",
    "840b1159d08678f23857937ba6691210edc220e017b83b93462282743781224478944d4e056a51f47b8cfd38beaa9060",
    "474d482e709a1333fc5d88b1fe56a88e5e6eb2d34e0c349dc770fd2c15bd62de38abf4e13a511bf2e6c14a1c7603edcf",
    "35b57d7282393867b71ad0c5a11b7eea62d5eb63293af2ead5c0f44111d1efd31ee09e386444982c18c35471e98ea566",
    "15f4332d1393d5e72b552f47d74a3f4f158f6fe5afe442ed43b669f8b03193a443730c3937a568716c35c78c4d7a6f09",
    "23dff7ac9596e31af0bfcc5317f0aacbfb9372fffd5b03535883c4bcdaaffc851435a469c87f765e8b22760a448c20c3",
    "547714ed426a84aa7ffd056daabbc029ac978858fdca4ce70a057604831aab9bebb7d55d15ee966550af623417b6d8e7",
    "5d771753f28e9f2b416651ebf103ae74d374cb15038e8256b8234441883d186b79066387f22a9a38b589dffc9d87e6eb",
    "0de4b439d71b7bb82e16c676db33f4bceb05daafdd29ea77f7eba3652cbf9a007966fdc8e4b269364ea150034904ed6b",
    "691684caa6a2c7fa73b2b01d031a427e51153692c45b3a77eaf2b37cca65803b298dbb5a6117853fc79dff3ae761b9e8",
    "16d314b4a32eaa27a177c971f894914818c06f16c0dc9f7f997951cf2338e9de0ddef091fdf0947df6968f8c7b195ab1",
    "90dcde2820f14373ea0d87412f1ef70930e2bd48eb35c045e0d4c425e40ad7c1862fd2e30568619fef8e8424d4eef082",
    "a1898caee304286ee391055f1cc9b1994a005ffe2741371b618d878ed1e0f6bba58884cbfef2d10ba3316bbd4245b058",
    "b1db214ef2d25fda546f09f6ff0ceee9143662ccbcca0ce1fbcfa73f13bca1c3d164f5f98ca15a758d541a89f867aeec",
    "4d9b75824ba84e708f719df6ccfdca7134aa3b375ccb9f7457913e053948a5bbb2ff1c594c9f7379e4b487239626d470",
    "8df48e860dde2515d59bc814cb670e4995fcbfcf16d5930af3038ccb63e93573ed15066413437697dc7bba1d645e9bef",
    "b21252707c2949074042dfd456492374773af189b315340442be22a90faa9e5622b34f4e1350852f6c025f9babddfce6",
    "9840129e828c2e75bae6f42f502cbde0d1d730f9f578d5aa4c32ae0c18923655b87f5e7971d4e7e2235c6979c72d1992",
    "720c74f9086936108052d341b1eacb94722037d24c78a5655edfdd755adaff78f40a11c6c1245af1821e559454be2468",
    "47ec2d81c33bfe06a7782a90d546139679be7d456c0780df43c4d93b867f8babd69a3532ee7de80466e8869110f0fc49",
    "c8e121b8ed6db5f737953c64dcf513fa8ca76ea1501b3f9c8499deef96be953544045c4309cda9e9f23436c7e08606da",
    "dfa156535ecfd9554412b8adc87f95ae6027fcda275efb853a466fc674eed564fef98ccc851410c692e68bb185f1c09e",
    "97c33fc1983603bd9aa82401bb87827008560b4eac84cde1de249d9c4b3c91efda604f835dec203478193d73780a8d36",
    "c366e90b3cba1abc6d4b6327ddf46a768da43a6a68b78845ae72f108f9ca6581cc2c8fe2a3b42bd92fb85af86c9f3f5c",
    "f7e577786311908a9951dcb6787a4d5052d47e9859b1ebee159966fd35afc301393adde2f5db7de9b14fa118026fd303",
    "4503859066970b32e45baf44ad08156fc30923078af6d11b9fac0586db5cb40310a7810466ce8f208ca0fe5521ffee3a",
    "471ddbeb906a15b3d0868caf639750eccc4a99be11a6dac7fdac9b0bdedc4dee3133f22e460726aa8a8497b07cd99134",
    "f5e1fd523b373a8e7957a6ebd4d7a5f79a0da884fa830866840cffac6e7e5d579c201fd75cbef77c6f7ff8ffbca28a08",
    "bc0313b4cf1d2b46f5ee34eb4d49807a2346761c9683df07c8f68030bd5fdc07ea12170805bf3ea7309dda6cdca01a9e",
    "6d73b46837395357c2f81e1c912a75750470e74dbb9a776fb4dae403f5508d3d927689adc0dcd413451800712a145955",
    "6a9afd4b364f6617c5e533ee6cd2e67f1720de446b1511a7b3b281c5d7a75d9974e79ebba4883c317b31b3f17201d1d8",
    "a4138e262d1a731043dc83f0e7e75b066499cfc2641e9c765e96f5ef7f69ff3a3414aaa23d0f2a21d1d951e4d3d1d9a1",
    "dd6dd9e9c6bae54876fc5100b5fa17b8cedc329b19e8b8503705f508dc5f3997d7ad429955f104c1e2917f0ec15621b2",
    "ba69503d0ac704fe1e4b1b0e380666a517bf4b119cecba3bee79b4eb4688757c34cf1c3529e2e13e59166c4eb0b9fe01",
    "1e39ecdd65856f56a50044eabf492689a28ad072e7de1e1fb3bd6a9ec96906c5a37056d684e57c83e947b6fbe4c2f214",
    "494c8c520eb510784778cdac66070dd78889e6941caff90e057a6c418fcca947835090644715a42c58d263d8f302e44f",
    "4a0aad6b609408a4c67db62c21af21e5702efba0213fe8f6a0976987014ec066fdec66e1d97fa5610c1f75126a68a074",
    "3325031c936e18b12c456a0674def0d8ba2ef5ab302ff352e2cc36480763c7e39cf2b94704b02f22a9009cda20aa05d4",
    "9cde64236a5d05850fbbd0bb4fc302b9691f41a4ce4dee404100be9ef52f825f2cb46140b7592f8e33a37f0ab41f75b7",
    "efd391ff44a76ffee985725aa06d05425d1d3a15ae2cd39051e6831da6c166045a26ad2a36982c933b441c5baf6cf86f",
    "fece58c01a3ba9c8ed4a3d3efc6e2cd95bed6435f8569344ca5b40913b3bbbedaed240ee376594949d23c9b5d7a19ea4",
    "5e109024e3d40baaf4c15d620771553d56200ecc7fa8e7fe1f0621de094efbb54158ce2b5ea089803c394a0529309d48",
    "16aa7183e93e38c2ecfdb2c3f5215d6888c1be67634897e919bf165e2fa1e6c5c3ea2a31af3eaadb53358035fa44a0ca",
    "124dc2168139e338507b6052c1fc3323d3fa086297c2ba89042fc166559bd5c54e4f3450621e4d3cc93339a5c902ddf2",
    "db206309b3828eda2f72f600858628c7ef432c41d3c47405de3d7d4d1604c1c97be11ff8a8f8bfb513bc8fbf332654ee",
    "241cb5df447dbf3437ecb5518260f0b4da8f21da7602a96214b2018a94178daf49139128d1f9f1ba1d826c219983dc1c",
    "9a6053acb5ff36925413690c51b7eafc1efc6fd7c6b130294ca61669b86b012a47fa1cb665762deac165a4cee8e558f5",
    "7f12ae9ea2be957b4c4e6274b8a714ba558461022730fb0ab02df1cd95410308914f97e09f453659433423fa12d9b3ba",
    "8e3afd5a0edf0516ee980898b6ed44a080351ae9b44cb9e74bc7a28281b68d1b0a7ec5bbda3b72c359b1bee3add10c4c",
    "af260ca71f265b09499b823629871435a363fd51c2a778d9ef101aa44a58d5686e30e3e3634c7d303f591a403c2aa4b2",
    "ecc75df2ba4d2b436c0dfbacabb30d472c62f4fcd1c2b8a12232539d209336f6a2dc871233c2c793556f2bbfa59c02d9",
    "3598301b90f7613f30d0057c12d08a0aa73f084065277f0e959e072aa69251d8dbd160f21a289cd3c3c6d5134c85d302",
    "6f85d51e13cbcc338c5392e31e9ed06f34fee1f2483be8d4a87f8dfba8eee8eafe4e46d9cd4835c6a8ca171d5d93fe62",
    "4d52e704d62828b4f5b30205ca65eb2955d59324b828a0567974485f01cef0b8da1430c063d337a459586699c15c7d71",
    "6c02d4aed2051c4a4723d524d37abbe774f14a2cfc73efe1f60d31892ae46524a6fb7c1fb828326622c48eee314b6577",
    "45cb4cef1f6eb87ab412726ddca8c03cc432558ce9adadd87f86fde0f1c64d2e420b130d3ebc979c16b6b67c0776aeb7",
    "5dfbfe758c22edbe449286dcc5fe1ebde173001ec7cab65890485ee4d9377e7dd199e83cb6a9ae2e6739021324d637e0",
    "496d340aa7d778ef5b8782a4a8765bcebdd1d51a2d9859c3faa7db26abc19a100cdc4f404ed8fe97c89646e4935f7925",
    "f3fbd0a1af9acea5c511abbc02055aa6596fa86d39d73dc81b6ac1cfa642c5a3ee92afadb752b50d5a69599a45798688",
    "b27212bbf1d88103d9be51eb83900dfcc5440edfbe7a4755a88cb33de4ca984d221d12849334841ca27f7df2835fafd8",
    "a387cb9807859d7d14f691f30b385fedef659e8cd67df20184c2522e606bc8c4b8a60671dcbd7d350bf26b7689db6fe4",
    "d44f5dbe8ce27d06d0343d4c5396df97b35399335beba765174a7b002d5e0eddb0bf6061ad0b66ab6f096bbbbfe8135c",
    "b4b91a52b570a139757857e9b6489fdbbfcc60d2d67c8ce7b76fa4825315c6b8c10d57e2f5ec04a62b5a11bd2eb78ee3",
    "508d7cbe5076f6f1c27ea99ab275cbdaba9e501646594b76995ed1d7d6a3c225f36d76da9a59e5873fa710c4928f17db",
    "d367b634b6b41fce9437f477879e86a139fa6f1b63845dbacaaca5e76814e6af821eeb0f7d16213af50ea7619917f557",
    "de3725721cb43133c0073fe78a5f6e8afac77804c15c3d37b32e06ed4819ebe8876eb003f3eb970196e425fad17b0dc9",
    "1cc5116cdb417236733dbaa9968d70c8ef80bddd5cc92cdfa98be889c2a1ef5aed6b0d9a1c04202ca2e7136bb5fca4dd",
    "a4b44e35157486fb9a58bf82353843ab8e0b68551289d797080ea4f0e7cd2db1340cf420aff441441c2563327258e20a",
    "ab8839cb4ef6a206da8266efe8092515ea138fc78b188c2f4d4070a4dfcb7f95776bb611a7d9d527552c4fb8ce9a834e",
    "fb2f8a2568d232028cd83f711d8c56a21b6cad16ffc5594bcdbf7248710d0eb94273953f5f202e9ccbf44520b4cb52a3",
    "af8a83218d757b7147ed9ce351d4c6fb71ece7db7063b1bf64dcedfc336bc295ac99aec89a6d35d08a156f32f605d2a4",
    "c679f7bb8349b4d87110f76dd80481e7e353b665cc1a02dcb77500d3dd9bec594215bd54ed2603489eb957d2b366a3e1",
    "f2eab018e97422d65bd9f77cd33850815eab680a75deca2e81968cd94d97a790391839371c1d6fcd08d0258be368276a",
    "3d6aa4e6dc3da1ae6c6e855acb561d2d4cb1f74bbb07a52e6659aa24b75e75f1648f50d1dcde1cbec206f7ad931ad706",
    "eeb3938c888d31c9ca178acb7c3588b0edbcbf4cb489d56acf8b3d84739698e980d838d37e1d7e594b00ce794ab7e0a3",
    "3fc673a92816faa01c88b54646b73c0bf8a48f180c53c191588e21380597a0523e0959c408ce5b7a40539ee714286872",
    "58fd3a6ae39948a2c972eb39c7357b7b06358b3e3348f7023c03733ec050b5dda233d769354e444579c4f272c0e01e39",
    "1ca2863121cf058249dcc8e223d4622e4b98743b7c62194457b170b2a09420ae221eed70e14388473344a883ba997951",
    "328e950ac17a2883ffb1b93d763f6e2d1142c13aa47454ee293173bec34584ab1417acfa5d8d2089ca3750e31c7f38e3",
    "e9412476bf0aa42a161b3a7de93db5f983eb17a24209c78969dc889c15e25dfce937906a30e21e0f726e99ca642c94c3",
    "57019b08bc7449e9f31aa09cf9ac5a5342fe96f8a4f066d99b79192ba1fc2e3fa8a76715ba89054f807563f15213b694",
    "4d7c1f50fba12a3aa8e30b7d645fbdbc4011899290ded9283b2e3e9a0801edea4275213250baab1cbf95e44772cadd89",
    "8e67f2a03dfff0aff976e51f4458e22569dce4a078bad2eee6321b99b0f93062205befb231170f7a4b1e6aadd3afc566",
    "d1a5bb180dbcb36d60068560ccf1945595352701900a628016c6dcaa0c96884385123f50f1d7a4ed79733444607cc72e",
    "136a178cf918c7a1a59268d43655c576438a126d50fec020e1e8d4fe9a309b5d3aa5128dca019eb54357edd56d2e2f01",
    "abe616bc2b63fe62804f82305ceca0629b5fbb73e5a1150e484e235e061e7edcfd422eec5657b77dc8fb34c1e6db50b4",
    "adee035af516f383b8452fc2731edc18445179c504abb4fa8f218ab4f8f4c02b4215a705335ceafd4e50c80cabe3d28d",
    "14291062c590bfe260c0358278fcc9e879132956ab725b7a19f8ce970c21b4ffb99941ed13fbaa0c5019cff29604b5bf",
    "393a554a01faaf9b8cf06b2bc44b8fcdedf950d14f83e2e4559c3672757071201ffb356db75919a91643ad95afd29713",
    "1077af7c4bc378b41c5b70f14871327ff97793d9f058eb223405a4bdc5e81357dbecc08f4d3bae6e5af80039ff0caa3f",
    "b794f5a8b7475cbe09840041f7c4c8b0942e0c9f96b207f3a41bc9c36995b016e3891cf4c39cdcfb72fc2e90985ceb73",
    "c3dc09666415d1e0c25f6818d7b76eff93f4fd4abc13dc249ac3f38044ab875669b44c8579d935d289c313a145ef0f99",
    "df673fa9ff521edc10008b663e72701eb6624eb521ddbe32279ff308fb90ec18d97a3df4e5097e245ad644b35c77b0c6",
    "39cea883b3c9707e010f040bb54437a7fe5d7af2879244e910199f0d6a476e2899f3cb8d9bf24a1104ea70da460a2a59",
    "2be8a8c20513ef07687ce8e10c766125c92d4432dbb65a6b782e6debbfbcb48a162e1cd6451d80d6534556862862bb50",
    "bd0874d513634aff58f59a57120ca5cc3f8bd16d40d049310d7ea6cdc67196179f94ce8d0394697529e2dcb238065b53",
    "6736e388c17b3a0335a23d2178d6e06a8535946793123a774917bb61d613f1d784548c68adbcaf485fd3bc1c40d0c9e6",
    "a2f51816553883fbc7a13e796176e7023887a085c620b89f2c87289ff168b07b025072338fd1d708bb6b4811c5525757",
    "c2fe8c06f249ef4ddebc7c7118b7929ab88fc2f1b879a30416b2792289269a9d770ad3b9ff87bac76ba6cf515ea5f1da",
    "968deb5a8581be77faf184d973d079c5b6290892e90222bf16e4eb926f2849216412d107694e745c8e531376ad11da56",
    "5a9f4e9b9b5519a7802c6e365f09f54f929e15559c3812e154a81da26e8aed54b67d55753caf4bbe408264838e22a468",
    "6dbd5425a1eff5b7fbd1cb48183dd8a9003dd34dea786a98f6e367200d1f3e37d6d6e00e465a17d7bab5cfefe491e211",
    "4fc98856076df926e07938aa84c0577f767e021c30f719cd1eae419102d0277d0c14b4bffb469c41f14de0d0949c1a9f",
    "7246b204dcbed581637f01f8402cbd02e724193479d4e4b264754aeac25db6bba311dbe12380647e48c08e9330396541",
    "4da874cd43a19ab9d3c96316d6844a6647df0e6b43cbb4bd66d268d1cbf6b84a5b097f8d71148f19c910b3cac3ac53ce",
    "3923c7af5f478def024fe9f99749d30f56f7685c3511e8a1d9b27ff40a48995fa4902a49889a214062fba5834b018543",
    "9f7ed27c2018e93333ec608fa4e39df03a6e392141c81caaa53646f4f659fbcb25955282e7e8464eba7d67bc5fe5846c",
    "e868999c83152cb416ebe732a3f3eda35e2f6bd5a9a2ea1696d8b45dcad185fa0adeca5b426210c20003f45f7d644c45",
    "bccc099eb2634cde4cd9952e9707d6a521b5e571751625da0655873675286f999e8d834ac06e353250ee255b7f051887",
    "0aaadc21957575eef41842dd682cbe55c7bf127bcdc610601dd2659cab264671ae23310ea88941a719d1ad43c1cee1ee",
    "57f2d4884c52b87bc9b4dece93e722143d7d3afdc4800eac17c917f8a1596cfd3a874472f57fc1ef19eeb90969a61eab",
    "dedfd80a118c366a3cf9ed84830c0e281fc0395b045d4b8017375f3bfae1ce21e58db85f5445ef5f527f9b4360973b8a",
    "ef556486054449d72750964415fad3be8db40f71ee8cdee4fb8be7bb551d5ba2bf7f4cc811f5518c9b351d8a6f77474e",
    "24c1d3b765e81f7281d4cf78c8b775b4488fcb879b37ebafc9d3c91209c2aacac82c81e36669d56f2b86e8630f705fb7",
    "d9fd0a31affb59cea034211e136c53809acf509b4e0c377515b6349c8fce6abbcff50133b8ffe77fbe307f4354ea57bf",
    "6eb1f4b42a941a1a814f89d067ca8dd787707075c9eaba6d00a3a9000cddfbfc075cdbe841f6023db18765189359138b",
    "d6bd5e515df017db9e19eeda5bcc9d7fc9abe70e162eadca37cc514d9066caaffa97190d9ff337b1bd01c7f42469238a",
    "ed1dacdaebac1f07cd3abae18258aed915b3add52c248e06815dee29774fea0037a32f0dcc2a335baa87523cdb111546",
    "1cd5e4325f239d22a7f401a26f4730942bf421341015a1965478e9917a705e444f5cc6010ba13c1096049cf29a060069",
    "c25e9ce05c6b92befacdcd3a5650201b2d5690f4cb7f66a2f870f5afe16ea445a114615e2f1ba95245baa48da10dd176",
    "eb123582bf738afaca74fb89e9528d32c70395eedee7931aaececd3a6e7ef74a662e5b64d7255494f7d3f1e3f9b3c1ad",
    "cd1ffd8220c6106a526e3920d17ed7439928d59091d2fcd0668b90ef5f83cc489037bcef11c382ee4bbecba78be79ca8",
    "9575a4ac466d13f896049c4457e627db6f448eafb661fdf38115339d2c1809c8f1087110e93ed1cdaec407f86df32a27",
    "f83e89100277ce2e00f24c86b7f1a5e2c36d8c345007dd8d13998e3964f47501256554f85c988d07cc64fa80c656678e",
    "135267b3f4a0a27c8054d1679a2ae0fe091bed13a5eca7633a0483933432e3a3cf9d94be3b1c14ee97f1f5a9ed09e796",
    "0d2fdb94b8986ee8c060718e3ef7f7a644fd43173d6a03d3f94dc00e21fe97d09db1f36c779aa2cd4df1ee61279bf9ae",
    "10f63d8a0370e5b8d756351cc5adfa6e65419300492e811d1e859bf3611f64cd957863e553b623580ebbb0f096b30590",
    "fee25e6521ac54831038f77d2f8c04fff3eda973f17fe8a2b41e84d765e507e32948791a06eb79937adf3563cb77a38a",
    "7aca96f3666b6b4033c41c627e17924484ae4a3c141ef6917dfc988b0aff36eea0a03c11439cbfa3dac890038a54e9f0",
    "b820b145033d7dbedc2f583a844f8d4c73afba62e8551f8cf5cbc826f7a7f0253854ca422bdd031a5224f56834a1794d",
    "93f437d8c717b911602217c8da5997526df5268b0b98c6b761081b9da2bb828ea3bc5fc02b4afd2c9d8d5a9055afc4bb",
    "72758bfe4be7db77b078fe4885f40c8741b460377d576cbf6a61916c7d30c7a745612fb6438c3f5dd7f23398e9b209f5",
    "797b6627064cdf1fd342fea4be643f047b2e7448016b5e55bcc512d7d46f25c2b9b39abb37fc5b1ec849c678c2148e93",
    "7a8d2877ccee257862858f416b34717049909f5d6da1589135be0536d20cc28e2a3c417027e88eda40fa9a5c979f92fe",
    "3deb8b2c37ff935d934575d3be5b01d62a5318825cf3abd2188ffbd428636d57efd36bf41afe57fc0090830843177a99",
    "871b29577d5133c24848aed27e465636ec9d39add4d34c8ad38ff1e6ec99d079ef5a4eaee750925ea330c1d6aba7b98f",
    "6f765e6b269dc44aae21f1f07b44063867277d944e28747cc447b20e197b4b7c5772abdf138488b11949c2b8cbc0eee1",
    "833a0219395dd155e2fa4834afcfb5a39520d94cac6124f091d7cef239bf127013bb43910fa07d51270ff5df75cac120",
    "2a9986ff4dd7b4fb22a24512823e29f96098f443973131538515b064bc2c0aec9511b04d69f9341379323c623042f957",
    "df51dcacffbf1c7e095b5529be5322cac9e8b406cb6a48d079f44a9c7b1dcf4d624a0d3161c7ed7eb25a7b83c8b747ab",
    "a83dc8715dfc8da851eca304ac306537d1a0a259c6796256c5aada0adced8a9ceefdfbc76ad365f711e3b7ff07284f2b",
    "6074fe8aaf1753a1a2742376d72b802bb173b3276608438dbf1c4f2d2cbfecaa40c1863e15d64162195f5cf715446e53",
    "4c5da61435c478b3ec802703fd02d2c3d5aae1bdf33f624b4005c10c982cf4e2da77596ae8e783b4c65410e2040eb6be",
    "6f4cb256470e2f84c7df1ddeacf73a662c931eda0b25dae8b46588b15950e247790fd91a9a996ff5cebea9190a5fa391",
    "36248fe165aa383964bfdbf70149191333762b1102abc51036b26b4583bdf0250f538d423e396b3463d9bd72dcd0090c",
    "e573b2f6aae6bf1a7e850f9c1b91f7605d8cbbfe23cda38833476a3102cd38f992346a71d4a65ef30fa44469b880c8dd",
    "4f967dd02ebc3917dcec402eae806ea830787ecb61622a5e18949fdc43d46004101a7522c854037a06acdd669e40a695",
    "5d5c052ed48bb7cedee3f0624a80d1f1bdbe7b88f0021b0e8c9cbc70fcbb761592af3c56d9eebaaa96972585299ef4dc",
    "d8ae23b8fdec317890af559ac1b703fcdac149d24697911ab521a43db172920141b3b205e81953b310eeb47f995667cc",
    "03b570b8a42c4f4ec8cd29ce44ea110fa6c0a0560b0b3f96fc279a234cd4a1565a54dadf33d74e46c7b2222085a0a607",
    "7b0590a16de82ddfcd181c879dc702eada5ac8986f9d381eee288e96755afbb06f7fd2e27f121cb99328bac6b5e51cfd",
    "e14958e7124a94d60db0586a1c0a6176620b6093684fa1c9949500af05402b506cbbbb3e9fc6d797606e3683ad527cdc",
    "d6f256aac56831b35923a4d9ab08039dc231f9b26100ec84e31402d12a836d5ce601f20cf349f027444f1f4256ce80be",
    "b06427b9f44a42fa26579d248a1e84d9bf15138bcda0ff65ab01cf62af4471553d1a2c5d483a5f61a6dd47855ed56a81",
    "8e2d335bf906474b66ba8ec9387e02f7dd71f5fa270fe26f9db07e10f60dcc38fbfde60ca795c5dce648d436b8b1b0d4",
    "25b53976398b1bf55b2d5c4c07df8e8519fbd408d8125c48cef5532f1a78aee006adb4f88a5c112417777005cd9672f5",
    "dff53a83248a1c750038131bd591d97d7b70e2238ff6115c476721a5b2b448071b1ee4fafe72120c3cea0dddd02220a2",
    "c2b23eceaaf9bc6d76fe860b73729b94e9a1ab2e794c96fb2bef4ee8c97674861098ff3c300f93699554c5e6c7bedcba",
    "a4b172958dc41585fc6e8edd354133c6c50854d9ed4e03e4d40bef907ec012445f1aa55adb5361c675814fa1ac5d1714",
    "247a226d2a1bf6c5f33e61602dda0491d85432d5f65f7cc087637bf9c813a158834841dc3287d98a1bf6998c4f26e9ff",
    "ee11078122dde5de6723819f8ed10760078136575633321131149c7aeb7b93d5955013779d7368051fd4f1dd4e76a105",
    "d139771d72aea4e4aac9cd9eb7ed1aa94bedbe6664146517a64097a7110ad7d6b9591eaef55c8201e4854ad3c2ca0f4b",
    "0fb4e8107828a5fe73171d3273058b3e81f1318862d0da20f564cf2f9223232ed3ef683b9e289f3493a1250f100d0932",
    "890b565c55b1028106260c62d2c5585172790533adc14ad03eeee5c36637735607f715cb0c1aa6312299848ce887aae5",
    "8aa8e8a7a3e58e8be5b7a5e585b7206c7a3420e5b8a7e8a7a3e7a081e599a8e6b58be8af95e6a0b7e69cac203000ff3e",
    "8f6c696e65203030314cbaff781f314cbaff781f314cbaff781f314cbaff781f314cbaff782f3135266dff781f35266d",
    "ff780f4cbaff781f314cbaff781f314cbaff781f314cbaff781f314cbaff781f314cbaff782f3136947cff780f4cbaff",
    "782f31366e0fff780f4cbaff781f314cbaff781f314cbaff781f314cbaff781f314cbaff781f314cbaff782f3137028c",
    "ff780f4cbaff781f314cbaff782f31376e0fff780f4cbaff781f314cbaff781f314cbaff781f314cbaff781f314cbaff",
    "784f3138310a3031ff3e0f50011d0460310f4cbaff781f314cbaff781f314cbaff781f314cbaff782f31384a2eff781f",
    "38709bff781f38709bff781f38709bff780f4cbaff781f314cbaff781f314cbaff781f314cbaff781f314cbaff781f31",
    "4cbaff782f31396e0fff781f396e0fff780f4cbaff781f314cbaff781f324cbaff781f324cbaff781f324cbaff781f32",
    "4cbaff781f324cbaff781f324cbaff781f324cbaff781f324cbaff782f3230dc1eff771f324cbaff781f324cbaff782f",
    "32314a2eff772f3231b83dff771f324cbaff781f324cbaff781f324cbaff781f324cbaff781f324cbaff781f324cbaff",
    "781f324cbaff782f323228d9ff771f324cbaff781f324cbaff781f324cbaff781f324cbaff781f324cbaff782f32324a",
    "2eff771f324cbaff781f324cbaff781f324cbaff782f32336e0fff780f4cbaff781f324cbaff781f324cbaff781f324c",
    "baff782f3233945cff771f324cbaff781f324cbaff782f32334a2eff780f4cbaff782f32346e0fff780f4cbaff781f32",
    "4cbaff782f3234707bff771f324cbaff782f32346e0fff780f4cbaff781f324cbaff781f324cbaff782f3234945cff77",
    "1f324c9aff781f324c9aff782f3235b83dff780f4c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff78",
    "1f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff78",
    "1f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff78",
    "1f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff78",
    "1f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff78",
    "1f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff781f324c9aff78",
    "1f324c9aff7750303239390a",
);

const FIX_STD_BX_CS: &str = concat!(
    "04224d187c40b83f000000000000503a030000ff21e887aae58aa8e8a7a3e58e8be5b7a5e585b7206c7a3420e5b8a7e8",
    "a7a3e7a081e599a8e6b58be8af95e6a0b7e69cac203000ff3e606c696e65203001001f0a5b01ff3e0f50011d058b011f",
    "318b01ff781f328b01ff781f338b01ff781f348b01ff781f358b01ff781f368b01ff781f378b01ff781f388b01ff781f",
    "398b01ff772f31308b01ff780f6e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f316e0fff781f",
    "316e0fff781f316e0fff78fffff531390a10a625a1eaef065248d7e1900a4df3391f04b7bfd7ee70dfe7537dc29c2188",
    "378509eb5db269c1d9bcbcddf031bc951603661fb1fdfe183490f41c4a608ad7c30c1f4c76ca7ead6af35ffd792bc8fa",
    "6d48079f48b90a216fd4a75877da750703e5ec5d104e01b4ce10720da0735d06090df58ae1273eec61868d0a8945ad44",
    "5329b6e5e9882c1f2f1801d51f17a3c4284d71a7d4c0266ce832a4217a68c55e3b6d0b8b40edb09a8b4092ec024f3201",
    "12c234a4a291fc64a4a422ad496f85a747eb3e7c3f8d45adb3a8c4012d8ec26a304d2bfc99ec7a2034e6596fc03ce04f",
    "6624699fe50abbb9c2a973ab661c5b2cabd8bf6c009e2ab6ebce15e8c3426044bd05ef406a0d8c77159557b4de2a060a",
    "7ebe8ddabfa83bc30ffb83de12d11a8b058d7d393039ecf6c3fc25e63961000312acafc2876fcaa38a5f8b7399f61f1e",
    "5ef488a824a25b129c73325a18016965570b79297e72bd342b45b3aa31c66b4db04b5210acc2b4789ad89243256d7675",
    "60e1bb0266840b1159d08678f23857937ba6691210edc220e017b83b93462282743781224478944d4e056a51f47b8cfd",
    "38beaa9060474d482e709a1333fc5d88b1fe56a88e5e6eb2d34e0c349dc770fd2c15bd62de38abf4e13a511bf2e6c14a",
    "1c7603edcf35b57d7282393867b71ad0c5a11b7eea62d5eb63293af2ead5c0f44111d1efd31ee09e386444982c18c354",
    "71dc20ff761f326e11ff781f326e11ff781f326e11ff781f326e11ff781f326e11ff781f326e11ff781f326e11ff781f",
    "326e11ff781f326e11ff781f32dc20ff781f336e0fff781f336e0fff781f336e0fff781f336e0fff781f336e0fff781f",
    "336e0fff781f336e0fff781f336e0fff781f336e0fff7650303033390a8738c86100000000bb6eec28",
);

}
