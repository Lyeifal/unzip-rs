//! Publish staged Explorer extraction without overwriting existing user files.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

fn candidate(base: &Path, index: usize, directory: bool) -> PathBuf {
    if index == 1 {
        return base.to_path_buf();
    }
    if directory {
        base.with_file_name(format!(
            "{} ({index})",
            base.file_name().unwrap_or_default().to_string_lossy()
        ))
    } else {
        let stem = base.file_stem().unwrap_or_default().to_string_lossy();
        let ext = base
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        base.with_file_name(format!("{stem} ({index}){ext}"))
    }
}

fn reserve_dir(base: &Path) -> io::Result<PathBuf> {
    for index in 1..=100_000 {
        let path = candidate(base, index, true);
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                // An existing empty output directory is not a content collision.
                let metadata = fs::symlink_metadata(&path)?;
                if metadata.is_dir() && !metadata.file_type().is_symlink()
                    && fs::read_dir(&path)?.next().is_none() { return Ok(path); }
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "无法分配不重名的输出目录",
    ))
}

fn publish_entry(source: &Path, base: &Path) -> io::Result<PathBuf> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "解压内容含符号链接，已保留在暂存目录，请手动检查",
        ));
    }
    if metadata.is_dir() {
        let target = reserve_dir(base)?;
        publish_contents(source, &target)?;
        fs::remove_dir(source)?;
        return Ok(target);
    }
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "不支持的解压产物类型",
        ));
    }
    for index in 1..=100_000 {
        let target = candidate(base, index, false);
        // hard_link is atomic and never replaces an existing destination.
        match fs::hard_link(source, &target) {
            Ok(()) => {
                fs::remove_file(source)?;
                return Ok(target);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => {}
        }
        let mut out = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(out) => out,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let copied = fs::File::open(source).and_then(|mut input| io::copy(&mut input, &mut out));
        drop(out);
        if let Err(e) = copied {
            let _ = fs::remove_file(&target);
            return Err(e);
        }
        fs::remove_file(source)?;
        return Ok(target);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "无法分配不重名的输出文件",
    ))
}

fn publish_contents(source: &Path, target: &Path) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        publish_entry(&entry.path(), &target.join(entry.file_name()))?;
    }
    Ok(())
}

pub(crate) fn publish(source: &Path, parent: &Path, named_folder: bool) -> io::Result<PathBuf> {
    if named_folder {
        publish_entry(source, &parent.join(source.file_name().unwrap_or_default()))
    } else {
        publish_contents(source, parent)?;
        Ok(parent.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collisions_never_replace_existing_files_or_directories() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("stage");
        let target = temp.path().join("out");
        fs::create_dir_all(source.join("folder")).unwrap();
        fs::create_dir_all(target.join("folder")).unwrap();
        fs::write(source.join("file.txt"), b"new").unwrap();
        fs::write(source.join("folder/new.txt"), b"nested").unwrap();
        fs::write(target.join("file.txt"), b"original").unwrap();
        fs::write(target.join("folder/keep.txt"), b"keep").unwrap();
        publish(&source, &target, false).unwrap();
        assert_eq!(fs::read(target.join("file.txt")).unwrap(), b"original");
        assert_eq!(fs::read(target.join("file (2).txt")).unwrap(), b"new");
        assert_eq!(fs::read(target.join("folder/keep.txt")).unwrap(), b"keep");
        assert_eq!(
            fs::read(target.join("folder (2)/new.txt")).unwrap(),
            b"nested"
        );
    }
    #[test]
    fn named_folder_reuses_existing_empty_directory_without_counter() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("stage/archive");
        let target = temp.path().join("out");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(target.join("archive")).unwrap();
        fs::write(source.join("file.txt"), b"new").unwrap();
        assert_eq!(
            publish(&source, &target, true).unwrap(),
            target.join("archive")
        );
        assert_eq!(fs::read(target.join("archive/file.txt")).unwrap(), b"new");
        assert!(!target.join("archive (2)").exists());
    }
}
