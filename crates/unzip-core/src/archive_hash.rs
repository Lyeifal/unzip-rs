//! Hash compressed output bytes using bounded memory, including each individual volume.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::types::RunCallback;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchiveHash {
    pub file_name: String,
    pub size_bytes: u64,
    pub sha256: String,
}

pub fn hash_file(path: &Path, cb: &dyn RunCallback) -> io::Result<ArchiveHash> {
    let mut file = File::open(path)?;
    let before = file.metadata()?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    let mut size = 0u64;
    loop {
        if cb.should_cancel() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "已取消哈希计算"));
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    let after = file.metadata()?;
    if size != before.len()
        || size != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err(io::Error::other("文件在哈希计算期间发生变化"));
    }
    Ok(ArchiveHash {
        file_name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        size_bytes: size,
        sha256: format!("{:x}", hasher.finalize()),
    })
}
