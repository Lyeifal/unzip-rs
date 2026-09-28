//! 共享数据结构（与 Python 版 unzip_core.py 的 dataclass 一一对应）。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// 卷族：rar-part | 7z-num | zip-num | rar-r | zip-z
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum VolFamily {
    RarPart,
    SevenZNum,
    ZipNum,
    RarR,
    ZipZ,
}

impl VolFamily {
    pub fn as_str(&self) -> &'static str {
        match self {
            VolFamily::RarPart => "rar-part",
            VolFamily::SevenZNum => "7z-num",
            VolFamily::ZipNum => "zip-num",
            VolFamily::RarR => "rar-r",
            VolFamily::ZipZ => "zip-z",
        }
    }

    /// 族是否属于 rar 体系（决定组装/探针走 unrar 还是 7z）。
    pub fn is_rar(&self) -> bool {
        matches!(self, VolFamily::RarPart | VolFamily::RarR)
    }
}

/// 7z 能解的压缩档种类（对应 Python SEVENZ_KINDS）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArchiveKind {
    Zip,
    Rar,
    SevenZ,
    Gzip,
    Bzip2,
    Xz,
    Tar,
    Zstd,
}

impl ArchiveKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ArchiveKind::Zip => "zip",
            ArchiveKind::Rar => "rar",
            ArchiveKind::SevenZ => "7z",
            ArchiveKind::Gzip => "gzip",
            ArchiveKind::Bzip2 => "bzip2",
            ArchiveKind::Xz => "xz",
            ArchiveKind::Tar => "tar",
            ArchiveKind::Zstd => "zstd",
        }
    }

    pub fn from_kind_str(s: &str) -> Option<Self> {
        match s {
            "zip" => Some(ArchiveKind::Zip),
            "rar" => Some(ArchiveKind::Rar),
            "7z" => Some(ArchiveKind::SevenZ),
            "gzip" => Some(ArchiveKind::Gzip),
            "bzip2" => Some(ArchiveKind::Bzip2),
            "xz" => Some(ArchiveKind::Xz),
            "tar" => Some(ArchiveKind::Tar),
            "zstd" => Some(ArchiveKind::Zstd),
            _ => None,
        }
    }
}

/// 包种类：压缩档 / lz4 套娃 / 最终产物（apk 族）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PkgKind {
    Archive(ArchiveKind),
    Lz4,
    Product,
}

impl PkgKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PkgKind::Archive(k) => k.as_str(),
            PkgKind::Lz4 => "lz4",
            PkgKind::Product => "product",
        }
    }

    pub fn from_kind_str(s: &str) -> Option<Self> {
        match s {
            "lz4" => Some(PkgKind::Lz4),
            "product" => Some(PkgKind::Product),
            other => ArchiveKind::from_kind_str(other).map(PkgKind::Archive),
        }
    }

    /// 是否属于 7z 可解档（对应 SEVENZ_KINDS 成员判断）。
    pub fn is_sevenz_kind(&self) -> bool {
        matches!(self, PkgKind::Archive(_))
    }
}

#[derive(Clone, Debug)]
pub struct VolumeRef {
    pub stem: String,     // 小写规范 stem（可能因改名而不准）
    pub family: VolFamily,
    pub idx: u32,         // 1 起
}

#[derive(Clone, Debug)]
pub struct Package {
    pub name: String,                       // 展示/目录名
    pub first: PathBuf,                     // 首卷（独立包即自身）
    pub kind: PkgKind,
    pub rel: PathBuf,                       // 首卷相对源根
    pub root: PathBuf,                      // 所属源根
    pub vol_family: Option<VolFamily>,      // 卷族（无分卷为 None）
    pub vol_stem: String,                   // 卷族 stem（小写）
    pub volumes: BTreeMap<u32, PathBuf>,    // idx -> Path（不含 idx1=first）
    pub dest_base: Option<PathBuf>,         // run 内算好的目标目录（不含重名后缀）
}

impl Package {
    /// 组键：最外层目录名；根目录散包用包名（对应 _group_key）。
    pub fn group_key(&self) -> String {
        if self.rel.components().count() > 1 {
            self.rel
                .components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .unwrap_or_else(|| self.name.clone())
        } else {
            self.name.clone()
        }
    }
}

#[derive(Default)]
pub struct ScanResult {
    pub archives: Vec<(PathBuf, PkgKind)>,                         // (path, kind)
    pub products: Vec<PathBuf>,
    pub volumes: HashMap<(VolFamily, String), BTreeMap<u32, PathBuf>>, // (family, stem) -> {idx: path}
    pub vol_firsts: HashMap<(VolFamily, String), PathBuf>,         // (family, stem) -> part1/001
    pub rar_orphans: Vec<PathBuf>,                                 // rar 魔数但名字无卷模式
    pub orphans: Vec<PathBuf>,                                     // 无魔数未知文件
    pub adopted: Vec<PathBuf>,                                     // 已被某卷集领走
    pub skips: Vec<(PathBuf, String)>,                             // (path, reason)
    pub warns: Vec<String>,
}

#[derive(Default)]
pub struct Summary {
    pub ok: Vec<(String, String)>,       // (name, note)
    pub failed: Vec<(String, String)>,   // (name, reason)
    pub skipped: Vec<(String, String)>,  // (name, reason)
    pub warns: Vec<String>,
}

/// 日志级别，as_str() 与 Python 版 level 字符串完全一致（前端按此着色）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogLevel {
    Ok,
    Error,
    Warn,
    Skip,
    Info,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Ok => "ok",
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Skip => "skip",
            LogLevel::Info => "info",
        }
    }
}

/// run() 的回调：日志、进度、取消。CLI/Tauri 各自实现。
pub trait RunCallback: Send + Sync {
    fn on_log(&self, msg: &str, level: LogLevel);
    fn on_progress(&self, _done: usize, _total: usize, _name: &str) {}
    fn should_cancel(&self) -> bool {
        false
    }
}

/// 便捷空实现：静默丢弃日志。
pub struct NullCallback;

impl RunCallback for NullCallback {
    fn on_log(&self, _msg: &str, _level: LogLevel) {}
}

/// 词法归一化：去掉 . 和 .. 分量、统一分隔符，不要求路径存在
///（对齐 Python Path.resolve(strict=False) 的语义）。
pub(crate) fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub(crate) fn is_under(path: &Path, parent: &Path) -> bool {
    let p = normalize(parent);
    let target = normalize(path);
    target == p || target.starts_with(&p)
}
