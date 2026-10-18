//! Non-destructive migration from the legacy application data directories.

use std::fs;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

pub const APP_DIR_NAME: &str = "unzip-rs";
pub const LEGACY_APP_DIR_NAME: &str = "com.unzip.app";

/// Copy a legacy directory only when the new directory is absent or empty.
/// Publish the complete copy with one rename; errors leave the source untouched.
/// Roaming data uses SQLite's snapshot API so committed WAL records are included.
pub fn migrate_directory(source: &Path, destination: &Path, snapshot_catalog: bool) -> Result<(), String> {
    if source == destination { return Ok(()); }
    if destination.is_dir() {
        let mut entries = fs::read_dir(destination).map_err(|e| e.to_string())?;
        if entries.next().transpose().map_err(|e| e.to_string())?.is_some() {
            return Ok(()); // Existing new-format data is authoritative, never merge stale files.
        }
    }
    if !source.exists() {
        return fs::create_dir_all(destination).map_err(|e| e.to_string());
    }
    if !source.is_dir() { return Err(format!("旧数据路径不是目录：{}", source.display())); }
    let parent = destination.parent().ok_or("新数据目录无父目录")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let staging = tempfile::Builder::new().prefix(".unzip-rs-migration-")
        .tempdir_in(parent).map_err(|e| e.to_string())?;
    copy_contents(source, staging.path(), snapshot_catalog)?;
    if destination.exists() {
        // Only an empty directory can be removed here; never remove existing user data.
        fs::remove_dir(destination).map_err(|e| e.to_string())?;
    }
    fs::rename(staging.path(), destination)
        .map_err(|e| format!("无法完成数据迁移 {} → {}：{e}", source.display(), destination.display()))?;
    Ok(())
}

fn copy_contents(source: &Path, destination: &Path, snapshot_catalog: bool) -> Result<(), String> {
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let target = destination.join(&name);
        if kind.is_symlink() { return Err(format!("迁移时遇到链接，请手动检查：{}", entry.path().display())); }
        if kind.is_dir() {
            fs::create_dir(&target).map_err(|e| e.to_string())?;
            copy_contents(&entry.path(), &target, snapshot_catalog)?;
        } else if kind.is_file() {
            if snapshot_catalog && matches!(name.to_str(), Some("pack_mapping.sqlite3-wal" | "pack_mapping.sqlite3-shm" | "pack_mapping.sqlite3-journal")) {
                continue;
            }
            if snapshot_catalog && name == "pack_mapping.sqlite3" {
                let db = Connection::open_with_flags(entry.path(), OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .map_err(|e| format!("读取旧数据库失败，原文件已保留：{e}"))?;
                db.busy_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;
                db.execute("VACUUM main INTO ?1", [target.to_string_lossy().as_ref()])
                    .map_err(|e| format!("复制旧数据库失败，原文件已保留：{e}"))?;
            } else {
                fs::copy(entry.path(), target).map_err(|e| format!("复制 {} 失败：{e}", entry.path().display()))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_includes_wal_data_and_retains_legacy_files() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join(LEGACY_APP_DIR_NAME);
        let new = temp.path().join(APP_DIR_NAME);
        fs::create_dir(&old).unwrap();
        fs::write(old.join("config.json"), r#"{"passwords":["keep"]}"#).unwrap();
        let db = Connection::open(old.join("pack_mapping.sqlite3")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE records(value TEXT); INSERT INTO records VALUES ('committed WAL');").unwrap();
        migrate_directory(&old, &new, true).unwrap();
        let copied = Connection::open(new.join("pack_mapping.sqlite3")).unwrap();
        assert_eq!(copied.query_row("SELECT value FROM records", [], |r| r.get::<_, String>(0)).unwrap(), "committed WAL");
        assert!(old.join("config.json").is_file());
        assert!(old.join("pack_mapping.sqlite3").is_file());
        fs::write(new.join("config.json"), "new preferences").unwrap();
        migrate_directory(&old, &new, true).unwrap();
        assert_eq!(fs::read_to_string(new.join("config.json")).unwrap(), "new preferences");
    }

    #[test]
    fn invalid_database_never_publishes_partial_migration() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let new = temp.path().join("new");
        fs::create_dir(&old).unwrap();
        fs::write(old.join("config.json"), "keep").unwrap();
        fs::write(old.join("pack_mapping.sqlite3"), "invalid sqlite").unwrap();
        assert!(migrate_directory(&old, &new, true).is_err());
        assert!(!new.exists());
        assert_eq!(fs::read_to_string(old.join("config.json")).unwrap(), "keep");
    }

    #[test]
    fn profile_migration_preserves_theme_and_existing_new_profile() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let install = temp.path().join(APP_DIR_NAME);
        fs::create_dir(&install).unwrap();
        fs::write(install.join("unzip-app.exe"), "installed app").unwrap();
        let new = install.join("webview");
        let relative = "EBWebView/Default/Local Storage/leveldb/theme";
        fs::create_dir_all(old.join(relative).parent().unwrap()).unwrap();
        fs::write(old.join(relative), "wallpaper and preferences").unwrap();
        migrate_directory(&old, &new, false).unwrap();
        assert_eq!(fs::read_to_string(new.join(relative)).unwrap(), "wallpaper and preferences");
        fs::write(new.join(relative), "new theme").unwrap();
        migrate_directory(&old, &new, false).unwrap();
        assert_eq!(fs::read_to_string(new.join(relative)).unwrap(), "new theme");
        assert_eq!(fs::read_to_string(install.join("unzip-app.exe")).unwrap(), "installed app");
    }
}
