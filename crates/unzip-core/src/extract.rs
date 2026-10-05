//! 解压执行后端：7z/unrar 子进程、密码循环、内置 lz4、捆绑工具回退
//! 子进程结果约定与文案风格固定（错误消息为回归断言锚点）。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::{exe_dir, Config};
use crate::types::ArchiveKind;

/// 子进程结果：退出码（None=启动失败）与合并输出（UTF-8 lossy）。
#[derive(Debug)]
pub struct ProcResult {
    pub code: Option<i32>,
    pub text: String,
}

/// 捆绑工具目录探测：环境变量 UNZIP_BUNDLED_DIR → exe 旁 bundled/。
/// 供上层（CLI/Tauri）与 Extractor 回退链共用。
pub fn bundled_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("UNZIP_BUNDLED_DIR") {
        let p = PathBuf::from(d);
        if p.is_dir() {
            return Some(p);
        }
    }
    let d = exe_dir().join("bundled");
    if d.is_dir() {
        Some(d)
    } else {
        None
    }
}

/// 解压器：持有配置与捆绑工具探测，7z 缺失时回退捆绑 7za.exe。
pub struct Extractor {
    seven_zip: Option<PathBuf>,
    unrar: Option<PathBuf>,
    lz4: Option<PathBuf>,
    lz4_cfg: String, // 配置原文（trim 后），仅用于错误文案
}

/// 跑外部工具：stdin 置空、stdout+stderr 合并 lossy、CREATE_NO_WINDOW。
/// exe 不存在或 spawn 失败返回 None。cwd：指定子进程工作目录（打包场景需要）。
fn run_exe(exe: &Path, cwd: Option<&Path>, tail: &[&str], args: &[String]) -> Option<ProcResult> {
    if !exe.exists() {
        return None;
    }
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .args(tail)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    let mut bytes = out.stdout;
    bytes.extend_from_slice(&out.stderr);
    Some(ProcResult {
        code: out.status.code(),
        text: String::from_utf8_lossy(&bytes).into_owned(),
    })
}

/// 输出按行取最后一行、截 120 字符（对齐 " ".join(text.strip().splitlines()[-1:])[:120]）。
fn tail_line(text: &str) -> String {
    text.trim().lines().last().unwrap_or("").chars().take(120).collect()
}

fn ok_le1(r: &ProcResult) -> bool {
    r.code.is_some_and(|c| c <= 1)
}

impl Extractor {
    /// bundled_dir：7za.exe / UnRAR.exe 旁挂目录（None = 仅探测 exe 旁 bundled/ 与环境变量）
    pub fn new(cfg: &Config, bundled_dir: Option<&Path>) -> Self {
        let bundled = bundled_dir
            .map(|p| p.to_path_buf())
            .or_else(crate::extract::bundled_dir);
        let pick = |cfg_path: &str, bundled_names: &[&str]| -> Option<PathBuf> {
            let p = cfg_path.trim();
            if !p.is_empty() && Path::new(p).exists() {
                return Some(PathBuf::from(p));
            }
            bundled.as_ref().and_then(|d| {
                bundled_names
                    .iter()
                    .map(|n| d.join(n))
                    .find(|p| p.exists())
            })
        };
        let lz4_cfg = cfg.lz4.trim().to_string();
        let lz4 = if !lz4_cfg.is_empty() && Path::new(&lz4_cfg).exists() {
            Some(PathBuf::from(&lz4_cfg))
        } else {
            None
        };
        Extractor {
            seven_zip: pick(&cfg.seven_zip, &["7za.exe", "7z.exe"]),
            unrar: pick(&cfg.unrar, &["UnRAR.exe"]),
            lz4,
            lz4_cfg,
        }
    }

    /// 跑 7z（自动附加 -y -bd -sccUTF-8，stdin 置空，CREATE_NO_WINDOW）。
    /// exe 缺失返回 None。
    pub fn run_7z(&self, args: &[String]) -> Option<ProcResult> {
        run_exe(self.seven_zip.as_deref()?, None, &["-y", "-bd", "-sccUTF-8"], args)
    }

    /// 跑 7z 并指定工作目录（打包场景：cd 进暂存目录后打包 "."，保证压缩包根结构）。
    pub fn run_7z_in(&self, dir: &Path, args: &[String]) -> Option<ProcResult> {
        run_exe(
            self.seven_zip.as_deref()?,
            Some(dir),
            &["-y", "-bd", "-sccUTF-8"],
            args,
        )
    }

    /// 7z 是否可用（用户配置路径或捆绑回退探测到即为 true）。
    pub fn has_7z(&self) -> bool {
        self.seven_zip.is_some()
    }

    /// 跑 unrar（自动附加 -y）。
    pub fn run_unrar(&self, args: &[String]) -> Option<ProcResult> {
        run_exe(self.unrar.as_deref()?, None, &["-y"], args)
    }

    /// 7z t 按序试密码（空密码在前），命中后正式解压。
    /// 返回 Ok(Some(命中密码)) / Ok(None=空密码命中) / Err(原因)。
    pub fn extract_with_7z(
        &self,
        archive: &Path,
        dest: &Path,
        passwords: &[String],
    ) -> Result<Option<String>, String> {
        let mut candidates: Vec<String> = vec![String::new()];
        for p in passwords {
            if !candidates.contains(p) {
                candidates.push(p.clone());
            }
        }
        let mut last_text = String::new();
        let mut tried = 0usize;
        for pw in &candidates {
            tried += 1;
            let Some(r) = self.run_7z(&[
                "t".to_string(),
                archive.to_string_lossy().into_owned(),
                format!("-p{pw}"),
            ]) else {
                return Err("找不到 7z.exe，请检查 config.json 中的 seven_zip 路径".to_string());
            };
            last_text = r.text.clone();
            if ok_le1(&r) {
                let Some(r2) = self.run_7z(&[
                    "x".to_string(),
                    archive.to_string_lossy().into_owned(),
                    format!("-o{}", dest.to_string_lossy()),
                    format!("-p{pw}"),
                ]) else {
                    return Err("找不到 7z.exe".to_string());
                };
                if ok_le1(&r2) {
                    return Ok(if pw.is_empty() { None } else { Some(pw.clone()) });
                }
                return Err(format!("测试通过但解压失败：{}", tail_line(&r2.text)));
            }
            let low = r.text.to_lowercase();
            if low.contains("wrong password") || low.contains("data error") {
                continue;
            }
            break;
        }
        Err(format!("尝试了 {tried} 个密码均失败。{}", tail_line(&last_text)))
    }

    /// 7z 不可用时的 rar 回退（unrar 裸 -p 会交互询问，空密码必须先试无 -p）。
    pub fn extract_rar_fallback(
        &self,
        archive: &Path,
        dest: &Path,
        passwords: &[String],
    ) -> Result<Option<String>, String> {
        let arch = archive.to_string_lossy().into_owned();
        let Some(r) = self.run_unrar(&["t".to_string(), arch.clone()]) else {
            return Err("7z 不可用，且找不到 UnRAR.exe，请检查 config.json 中的路径".to_string());
        };
        if r.code == Some(0) {
            let r2 = self.run_unrar(&["x".to_string(), arch, dest.to_string_lossy().into_owned()]);
            if r2.as_ref().is_some_and(|r| r.code == Some(0)) {
                return Ok(None); // 空密码命中
            }
            return Err("测试通过但解压失败".to_string());
        }
        let mut last_text = r.text;
        let mut seen: Vec<String> = Vec::new();
        for pw in passwords {
            if pw.is_empty() || seen.contains(pw) {
                continue;
            }
            seen.push(pw.clone());
            let Some(r) = self.run_unrar(&["t".to_string(), format!("-p{pw}"), arch.clone()]) else {
                return Err("找不到 UnRAR.exe".to_string());
            };
            last_text = r.text.clone();
            if r.code == Some(0) {
                let r2 = self.run_unrar(&[
                    "x".to_string(),
                    format!("-p{pw}"),
                    arch.clone(),
                    dest.to_string_lossy().into_owned(),
                ]);
                if r2.as_ref().is_some_and(|r| r.code == Some(0)) {
                    return Ok(Some(pw.clone()));
                }
                return Err("测试通过但解压失败".to_string());
            }
        }
        Err(format!("UnRAR 尝试所有密码均失败。{}", tail_line(&last_text)))
    }

    /// kind=rar 时优先 7z（探测 -p 防加密头卡交互），7z 不可用走 unrar 回退。
    pub fn extract_archive(
        &self,
        archive: &Path,
        dest: &Path,
        passwords: &[String],
        kind: ArchiveKind,
    ) -> Result<Option<String>, String> {
        if kind == ArchiveKind::Rar {
            let r7 = self.run_7z(&[
                "t".to_string(),
                archive.to_string_lossy().into_owned(),
                "-p".to_string(),
            ]);
            if r7.is_none() {
                return self.extract_rar_fallback(archive, dest, passwords);
            }
        }
        self.extract_with_7z(archive, dest, passwords)
    }

    /// lz4 解码：内置 lz4mini 优先，外部 lz4.exe 回退。None=成功，Some=错误描述。
    pub fn decode_lz4(&self, src: &Path, dst: &Path) -> Option<String> {
        let mut errors: Vec<String> = Vec::new();
        if let Err(e) = crate::lz4mini::decompress_file(src, dst) {
            errors.push(format!("内置解码失败：{e}"));
        } else {
            return None;
        }
        match &self.lz4 {
            Some(exe) => {
                let mut cmd = Command::new(exe);
                cmd.args(["-d", "-f", "-q"])
                    .arg(src)
                    .arg(dst)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
                }
                match cmd.output() {
                    Ok(out) => {
                        if out.status.code() == Some(0) && dst.exists() {
                            return None;
                        }
                        let mut bytes = out.stdout;
                        bytes.extend_from_slice(&out.stderr);
                        let text = String::from_utf8_lossy(&bytes);
                        let stripped = text.trim();
                        let msg = if stripped.is_empty() {
                            format!("退出码 {}", out.status.code().unwrap_or(-1))
                        } else {
                            tail_line(&text)
                        };
                        errors.push(format!("lz4.exe 解码失败：{msg}"));
                    }
                    Err(e) => errors.push(format!("无法运行 lz4.exe：{e}")),
                }
            }
            None => errors.push(format!(
                "lz4.exe 未配置或不存在：{}",
                if self.lz4_cfg.is_empty() {
                    "（空）"
                } else {
                    &self.lz4_cfg
                }
            )),
        }
        Some(errors.join("；"))
    }
}

/// 先 hard_link（目标已存在先删），失败回退 copy。
pub(crate) fn link_or_copy(src: &Path, dst: &Path) {
    if dst.exists() {
        let _ = std::fs::remove_file(dst);
    }
    if std::fs::hard_link(src, dst).is_err() {
        let _ = std::fs::copy(src, dst);
    }
}
