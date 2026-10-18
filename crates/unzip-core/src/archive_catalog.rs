//! Shared archive records. SQLite is the authoritative store once initialized;
//! a valid legacy central JSON is imported once inside a transaction and left
//! untouched on disk. Records keep their full JSON (including unknown fields);
//! per-output hashes are mirrored into indexed rows for password lookup.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{
    params, params_from_iter, Connection, OptionalExtension, Transaction, TransactionBehavior,
};
use serde_json::{json, Map, Value};

use crate::archive_hash::{hash_file, ArchiveHash};
use crate::types::{LogLevel, RunCallback};

static CATALOG_LOCK: Mutex<()> = Mutex::new(());

/// Serializes tests that point UNZIP_CATALOG_PATH at a temp location; shared
/// across modules so catalog-env mutating tests in one file cannot race another.
#[cfg(test)]
pub(crate) static CATALOG_TEST_LOCK: Mutex<()> = Mutex::new(());

const META_INITIALIZED: &str = "cat.initialized";
const META_SCHEMA_VERSION: &str = "cat.schema_version";
const META_GENERATED_AT: &str = "cat.generated_at";
const META_UPDATED_AT_UNIX: &str = "cat.updated_at_unix";
const META_EXTRA_TOP_LEVEL: &str = "cat.extra_top_level";

const MAX_QUERY_PAGE_SIZE: u32 = 100;
const MAX_QUERY_FILTERS: usize = 16;
const MAX_QUERY_OPTION_FIELDS: usize = 32;
const MAX_FIELD_ID_LEN: usize = 64;
/// json_each paths, the LIKE escape character and EXPLAIN-style diagnostics all
/// assume an ASCII field id; any other shape is a caller bug, not a user value.
const FIELD_ID_CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";

pub fn catalog_path() -> PathBuf {
    if let Some(path) = std::env::var_os("UNZIP_CATALOG_PATH") {
        return PathBuf::from(path);
    }
    if let Some(dir) = std::env::var_os("APPDATA") {
        let base = PathBuf::from(dir);
        let old = base.join(crate::storage::LEGACY_APP_DIR_NAME);
        let current = base.join(crate::storage::APP_DIR_NAME);
        if let Err(error) = crate::storage::migrate_directory(&old, &current, true) {
            eprintln!("数据迁移失败，继续使用原目录 {}：{error}", old.display());
            return old.join("pack_mapping.json");
        }
        return current.join("pack_mapping.json");
    }
    crate::config::config_path().with_file_name("pack_mapping.json")
}

/// SQLite store derived as a sibling of the legacy central JSON, so
/// `UNZIP_CATALOG_PATH` selects both locations at once.
pub fn database_path() -> PathBuf {
    catalog_path().with_extension("sqlite3")
}

fn empty_catalog() -> Value {
    json!({"schema_version": 2, "generated_at": "", "total_count": 0, "mappings": []})
}

fn read_at(path: &Path) -> Result<Value, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(empty_catalog()),
        Err(e) => return Err(format!("读取记录失败（{}）：{e}", path.display())),
    };
    let data: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("记录 JSON 无效（{}），原文件已保留：{e}", path.display()))?;
    let items = data
        .get("mappings")
        .and_then(Value::as_array)
        .ok_or_else(|| "记录文件缺少 mappings 列表".to_string())?;
    for item in items {
        if !item.is_object()
            || item["archive_name"].as_str().is_none()
            || item["source_name"].as_str().is_none()
            || item["created_at"].as_str().is_none()
            || item["source_size"]["bytes"].as_u64().is_none()
            || item["compressed_size"]["bytes"].as_u64().is_none()
            || item["compression_ratio"].as_str().is_none()
        {
            return Err("记录中有不完整的条目，原文件已保留".into());
        }
    }
    Ok(data)
}

fn atomic_write(path: &Path, data: &Value, backup: bool) -> Result<(), String> {
    let parent = path.parent().ok_or("记录目录无效")?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut tmp, data).map_err(|e| e.to_string())?;
    tmp.write_all(b"\n").map_err(|e| e.to_string())?;
    tmp.as_file().sync_all().map_err(|e| e.to_string())?;
    if backup && path.exists() {
        fs::copy(path, path.with_extension("json.bak"))
            .map_err(|e| format!("备份记录失败，未覆盖原文件：{e}"))?;
    }
    tmp.persist(path)
        .map_err(|e| format!("保存记录失败：{}", e.error))?;
    Ok(())
}

fn lock_guard() -> MutexGuard<'static, ()> {
    CATALOG_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

fn db_err(e: rusqlite::Error) -> String {
    format!("记录数据库错误：{e}")
}

fn create_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS catalog_meta (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS records (
             record_id TEXT PRIMARY KEY,
             data TEXT NOT NULL,
             password TEXT
         );
         CREATE TABLE IF NOT EXISTS record_hashes (
             record_id TEXT NOT NULL REFERENCES records(record_id) ON DELETE CASCADE,
             file_name TEXT NOT NULL,
             size_bytes INTEGER NOT NULL,
             sha256 TEXT NOT NULL,
             PRIMARY KEY (record_id, file_name)
         );
         CREATE INDEX IF NOT EXISTS idx_record_hashes_size ON record_hashes(size_bytes);
         CREATE INDEX IF NOT EXISTS idx_record_hashes_sha256 ON record_hashes(sha256);",
    )
    .map_err(db_err)
}

fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM catalog_meta WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_err)
}

fn meta_set(tx: &Transaction, key: &str, value: &str) -> Result<(), String> {
    tx.execute(
        "INSERT INTO catalog_meta(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(db_err)?;
    Ok(())
}

fn is_initialized(conn: &Connection) -> Result<bool, String> {
    let table: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'catalog_meta'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    match table {
        None => Ok(false),
        Some(_) => meta_get(conn, META_INITIALIZED).map(|value| value.is_some()),
    }
}

fn store_record(tx: &Transaction, record: &Value) -> Result<(), String> {
    let id = record["record_id"].as_str().ok_or("记录缺少 ID")?;
    let data = serde_json::to_string(record).map_err(|e| e.to_string())?;
    let password = record["password"].as_str();
    tx.execute(
        "INSERT INTO records(record_id, data, password) VALUES(?1, ?2, ?3)
         ON CONFLICT(record_id) DO UPDATE SET
             data = excluded.data,
             password = excluded.password",
        params![id, data, password],
    )
    .map_err(db_err)?;
    tx.execute(
        "DELETE FROM record_hashes WHERE record_id = ?1",
        params![id],
    )
    .map_err(db_err)?;
    let hashes: Vec<ArchiveHash> =
        serde_json::from_value(record["output_hashes"].clone()).unwrap_or_default();
    let mut keys = HashSet::new();
    for (index, hash) in hashes.into_iter().enumerate() {
        // The full JSON retains the original name. This table is only an index:
        // distinguish repeated names so a valid legacy record can keep every hash.
        let mut key = hash.file_name;
        if !keys.insert(key.clone()) {
            key = format!("\0{index}");
            while !keys.insert(key.clone()) {
                key.push('\0');
            }
        }
        tx.execute(
            "INSERT INTO record_hashes(record_id, file_name, size_bytes, sha256)
             VALUES(?1, ?2, ?3, ?4)",
            params![id, key, hash.size_bytes as i64, hash.sha256],
        )
        .map_err(db_err)?;
    }
    Ok(())
}

fn get_record(conn: &Connection, id: &str) -> Result<Option<Value>, String> {
    let text: Option<String> = conn
        .query_row(
            "SELECT data FROM records WHERE record_id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    text.map(|text| serde_json::from_str(&text).map_err(|e| format!("记录数据无效：{e}")))
        .transpose()
}

fn load_extra(conn: &Connection) -> Result<Map<String, Value>, String> {
    match meta_get(conn, META_EXTRA_TOP_LEVEL)? {
        Some(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => Ok(map),
            _ => Err("记录元数据无效，请先备份数据库再修复".into()),
        },
        None => Ok(Map::new()),
    }
}

/// Persist unknown top-level JSON attributes so load/export keep them.
fn save_extra(tx: &Transaction, extra: &Map<String, Value>) -> Result<(), String> {
    let text = serde_json::to_string(&Value::Object(extra.clone())).map_err(|e| e.to_string())?;
    meta_set(tx, META_EXTRA_TOP_LEVEL, &text)
}

fn migrate_json(tx: &mut Transaction, data: &Value) -> Result<(), String> {
    let mut extra = Map::new();
    if let Some(object) = data.as_object() {
        for (key, value) in object {
            match key.as_str() {
                "mappings" | "total_count" => continue,
                "schema_version" | "generated_at" | "updated_at_unix" => {
                    let meta_key = match key.as_str() {
                        "schema_version" => META_SCHEMA_VERSION,
                        "generated_at" => META_GENERATED_AT,
                        _ => META_UPDATED_AT_UNIX,
                    };
                    let text = match value {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    meta_set(tx, meta_key, &text)?;
                }
                _ => {
                    extra.insert(key.clone(), value.clone());
                }
            }
        }
    }
    if !extra.is_empty() {
        save_extra(tx, &extra)?;
    }
    let mut ids = HashSet::new();
    for mut item in data["mappings"].as_array().cloned().unwrap_or_default() {
        if item["record_id"].as_str().is_none() {
            if let Some(path) = item["archive_path"].as_str() {
                item["record_id"] = json!(record_id(Path::new(path)));
            }
        }
        let id = item["record_id"]
            .as_str()
            .ok_or("记录缺少 ID，原文件已保留")?;
        if !ids.insert(id.to_string()) {
            return Err("记录中有重复 ID，原文件已保留".into());
        }
        store_record(tx, &item)?;
    }
    Ok(())
}

fn touch_meta(tx: &Transaction) -> Result<(), String> {
    meta_set(tx, META_SCHEMA_VERSION, "2")?;
    meta_set(
        tx,
        META_UPDATED_AT_UNIX,
        &std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string(),
    )?;
    meta_set(tx, META_GENERATED_AT, &crate::pack::chrono_like_today())?;
    Ok(())
}

/// Open the SQLite catalog, creating the schema and atomically importing a valid
/// legacy central JSON on first use. An invalid legacy JSON aborts with an error
/// before any database file is created, so data can never hide behind an empty DB.
fn open_db() -> Result<Connection, String> {
    let json_path = catalog_path();
    let db_path = database_path();
    if !db_path.is_file() && json_path.is_file() {
        read_at(&json_path)?;
    }
    let parent = db_path.parent().ok_or("记录目录无效")?;
    fs::create_dir_all(parent).map_err(|e| format!("创建记录目录失败：{e}"))?;
    let mut conn = Connection::open(&db_path).map_err(db_err)?;
    conn.busy_timeout(Duration::from_secs(30)).map_err(db_err)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(db_err)?;
    if is_initialized(&conn)? {
        return Ok(conn);
    }
    let mut tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_err)?;
    create_schema(&tx)?;
    if !is_initialized(&tx)? {
        if json_path.is_file() {
            let data = read_at(&json_path)?;
            migrate_json(&mut tx, &data)?;
        }
        meta_set(&tx, META_INITIALIZED, "1")?;
    }
    tx.commit().map_err(db_err)?;
    Ok(conn)
}

fn read_catalog(conn: &Connection) -> Result<Value, String> {
    let mut stmt = conn
        .prepare("SELECT data FROM records ORDER BY rowid")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(db_err)?;
    let mut mappings = Vec::new();
    for row in rows {
        let text = row.map_err(db_err)?;
        mappings.push(serde_json::from_str(&text).map_err(|e| format!("记录数据无效：{e}"))?);
    }
    drop(stmt);
    let mut data = empty_catalog();
    data["total_count"] = json!(mappings.len());
    data["mappings"] = Value::Array(mappings);
    if let Some(version) = meta_get(conn, META_SCHEMA_VERSION)? {
        data["schema_version"] = serde_json::from_str(&version).unwrap_or_else(|_| json!(2));
    }
    if let Some(generated) = meta_get(conn, META_GENERATED_AT)? {
        data["generated_at"] = Value::String(generated);
    }
    if let Some(updated) = meta_get(conn, META_UPDATED_AT_UNIX)? {
        if let Ok(number) = updated.parse::<u64>() {
            data["updated_at_unix"] = json!(number);
        }
    }
    if let Some(object) = data.as_object_mut() {
        for (key, value) in load_extra(conn)? {
            object.entry(key).or_insert(value);
        }
    }
    Ok(data)
}

pub fn load() -> Result<Value, String> {
    let _guard = lock_guard();
    let mut conn = open_db()?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(db_err)?;
    let data = read_catalog(&tx)?;
    tx.commit().map_err(db_err)?;
    Ok(data)
}

/// 全部记录（跨所有输出目录）的权威 archive_name 列表，供打包续号匹配系列用。
/// 只查名称列不取整记录；读取失败如实报错，调用方不得静默沿用可能撞号的起点。
/// 尚无任何记录存储时不创建数据库，视为空列表。
pub fn archive_names() -> Result<Vec<String>, String> {
    if !database_path().is_file() && !catalog_path().is_file() {
        return Ok(Vec::new());
    }
    let _guard = lock_guard();
    let conn = open_db()?;
    let mut stmt = conn
        .prepare("SELECT json_extract(data, '$.archive_name') FROM records")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, Option<String>>(0))
        .map_err(db_err)?;
    let mut names = Vec::new();
    for row in rows {
        if let Some(name) = row.map_err(db_err)? {
            if !name.is_empty() {
                names.push(name);
            }
        }
    }
    Ok(names)
}

pub fn record_id(archive_path: &Path) -> String {
    let text = archive_path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

/// Update archive properties after packing, preserving annotations and unknown fields.
pub fn upsert(items: Vec<Value>) -> Result<(), String> {
    let _guard = lock_guard();
    let mut conn = open_db()?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_err)?;
    let result = upsert_in(&tx, &items);
    match result {
        Ok(()) => {
            touch_meta(&tx)?;
            tx.commit().map_err(db_err)
        }
        Err(e) => Err(e),
    }
}

fn upsert_in(tx: &Transaction, items: &[Value]) -> Result<(), String> {
    for item in items {
        let id = item["record_id"].as_str().ok_or("记录缺少 ID")?;
        let merged = match get_record(tx, id)? {
            Some(mut old) => {
                let old_object = old.as_object_mut().ok_or("旧记录无效")?;
                for (key, value) in item.as_object().ok_or("记录无效")? {
                    if key != "metadata" {
                        old_object.insert(key.clone(), value.clone());
                    }
                }
                old
            }
            None => item.clone(),
        };
        store_record(tx, &merged)?;
    }
    Ok(())
}

/// Import an old output-directory JSON without modifying it or overwriting newer local records.
pub fn import_legacy(file: &Path) -> Result<usize, String> {
    if !file.is_file() {
        return Err("未找到映射 JSON 文件".into());
    }
    if fs::canonicalize(file).ok() == fs::canonicalize(catalog_path()).ok() {
        return Ok(0);
    }
    let imported = read_at(file)?;
    let dir = file.parent().ok_or("映射目录无效")?;
    let mut items = imported["mappings"]
        .as_array()
        .ok_or("映射列表无效")?
        .clone();
    for item in &mut items {
        // A legacy name must be a basename; do not let an imported name escape its directory.
        let name = item["archive_name"].as_str().ok_or("压缩包名无效")?;
        if name.contains(['/', '\\']) || name == "." || name == ".." || name.contains(':') {
            return Err("旧记录包含无效的压缩包文件名".into());
        }
        let archive_path = item
            .get("archive_path")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| dir.join(name));
        item["record_id"] = json!(record_id(&archive_path));
        item["archive_path"] = json!(archive_path);
        item["output_dir"] = json!(archive_path.parent());
    }
    let _guard = lock_guard();
    let mut conn = open_db()?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_err)?;
    match import_in(&tx, &imported, &items) {
        Ok(count) => {
            if count > 0 {
                touch_meta(&tx)?;
            }
            tx.commit().map_err(db_err)?;
            Ok(count)
        }
        Err(e) => Err(e),
    }
}

fn import_in(tx: &Transaction, imported: &Value, items: &[Value]) -> Result<usize, String> {
    // Keep previously unknown top-level attributes on the first import.
    let mut extra = load_extra(tx)?;
    let mut extra_changed = false;
    if let Some(object) = imported.as_object() {
        for (key, value) in object {
            if key == "mappings" || key == "total_count" {
                continue;
            }
            // Mirrors the old JSON merge: schema_version/generated_at always
            // exist on a loaded catalog; updated_at_unix only once saved.
            let managed = match key.as_str() {
                "schema_version" | "generated_at" => true,
                "updated_at_unix" => meta_get(tx, META_UPDATED_AT_UNIX)?.is_some(),
                _ => false,
            };
            if !managed && !extra.contains_key(key) {
                extra.insert(key.clone(), value.clone());
                extra_changed = true;
            }
        }
    }
    if extra_changed {
        save_extra(tx, &extra)?;
    }
    let mut count = 0;
    for item in items {
        let id = item["record_id"].as_str().ok_or("记录缺少 ID")?;
        if get_record(tx, id)?.is_none() {
            store_record(tx, item)?;
            count += 1;
        }
    }
    Ok(count)
}

pub fn update_metadata(id: &str, metadata: Value) -> Result<(), String> {
    if !metadata.is_object() {
        return Err("备注和字段格式无效".into());
    }
    let _guard = lock_guard();
    let mut conn = open_db()?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_err)?;
    let result = (|| {
        // This command changes only annotations, never passwords, hashes or other rows.
        let mut row = get_record(&tx, id)?.ok_or("记录已不存在，请刷新后重试")?;
        row["metadata"] = metadata;
        store_record(&tx, &row)
    })();
    match result {
        Ok(()) => {
            touch_meta(&tx)?;
            tx.commit().map_err(db_err)
        }
        Err(e) => Err(e),
    }
}

/// LIKE treats `%` and `_` as wildcards; the escape character makes them literal.
fn escape_like(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '%' || ch == '_' || ch == '!' {
            out.push('!');
        }
        out.push(ch);
    }
    out
}

fn validate_field_id(field_id: &str) -> Result<(), String> {
    if field_id.is_empty()
        || field_id.len() > MAX_FIELD_ID_LEN
        || !field_id.as_bytes()[0].is_ascii_alphabetic()
        || ["__proto__", "constructor", "prototype"].contains(&field_id)
    {
        return Err("字段 ID 无效".into());
    }
    if !field_id.chars().all(|c| FIELD_ID_CHARS.contains(c)) {
        return Err(format!("字段 ID 无效：{field_id}"));
    }
    Ok(())
}

/// Sort keys mirror LibraryView.vue; unknown sorts fall back to name ascending.
fn sort_clause(sort: &str) -> &'static str {
    match sort {
        "size" => "json_extract(data, '$.source_size.bytes') DESC, record_id ASC",
        "ratio" => {
            "CAST(REPLACE(TRIM(json_extract(data, '$.compression_ratio')), '%', '') AS REAL) ASC, record_id ASC"
        }
        "date" => "json_extract(data, '$.created_at') DESC, record_id ASC",
        _ => "json_extract(data, '$.source_name') ASC, record_id ASC",
    }
}

/// Query one page of the archive library straight from SQLite.
///
/// Semantics mirror the library page (LibraryView.vue): search is a
/// case-insensitive ASCII substring over source_name, archive_name,
/// metadata.notes and every metadata.custom_fields value (`%`/`_` in the
/// query are literal), filters AND on exact custom field strings, and `total`
/// counts after search/filters while `stats` stay global. `filter_options`
/// lists distinct non-empty string values across the whole library, including
/// values no longer present in the configured options. Only page records are
/// materialized as full JSON. `page` is 1-based and `page_size` must be
/// 1..=100; an unknown `sort` defaults to name ascending. All page, count,
/// stats and option queries run in one deferred transaction snapshot.
pub fn query_page(
    page: u32,
    page_size: u32,
    search: &str,
    sort: &str,
    filters: &HashMap<String, String>,
    option_fields: &[String],
) -> Result<Value, String> {
    if page == 0 {
        return Err("页码必须从 1 开始".into());
    }
    if page_size == 0 || page_size > MAX_QUERY_PAGE_SIZE {
        return Err(format!("每页数量必须在 1 到 {MAX_QUERY_PAGE_SIZE} 之间"));
    }
    let mut active_filters: Vec<(&String, &String)> = filters
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .collect();
    if active_filters.len() > MAX_QUERY_FILTERS {
        return Err(format!("筛选条件过多（最多 {MAX_QUERY_FILTERS} 个）"));
    }
    active_filters.sort_by(|a, b| a.0.cmp(b.0));
    for (field_id, _) in &active_filters {
        validate_field_id(field_id)?;
    }
    let mut distinct_options: Vec<&str> = Vec::new();
    for field_id in option_fields {
        if field_id.is_empty() {
            continue;
        }
        validate_field_id(field_id)?;
        if distinct_options.len() >= MAX_QUERY_OPTION_FIELDS {
            return Err(format!("选项字段过多（最多 {MAX_QUERY_OPTION_FIELDS} 个）"));
        }
        if !distinct_options.contains(&field_id.as_str()) {
            distinct_options.push(field_id);
        }
    }

    let _guard = lock_guard();
    let mut conn = open_db()?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(db_err)?;
    let result = query_page_in(
        &tx,
        page,
        page_size,
        search,
        sort,
        &active_filters,
        &distinct_options,
    );
    match result {
        Ok(value) => {
            tx.commit().map_err(db_err)?;
            Ok(value)
        }
        Err(e) => Err(e),
    }
}

fn query_page_in(
    tx: &Transaction,
    page: u32,
    page_size: u32,
    search: &str,
    sort: &str,
    active_filters: &[(&String, &String)],
    option_fields: &[&str],
) -> Result<Value, String> {
    let mut where_sql = String::new();
    let mut where_params: Vec<String> = Vec::new();
    let pattern: Option<String> = if search.trim().is_empty() {
        None
    } else {
        Some(format!("%{}%", escape_like(search.trim())))
    };
    if let Some(pattern) = &pattern {
        // LIKE is case-insensitive for ASCII; non-ASCII (e.g. Chinese) matches
        // as a literal UTF-8 substring, and ESCAPE makes `%`/`_` literal.
        where_sql.push_str(
            " AND (json_extract(data, '$.source_name') LIKE ? ESCAPE '!'\n             OR json_extract(data, '$.archive_name') LIKE ? ESCAPE '!'\n             OR EXISTS (SELECT 1 FROM json_each(COALESCE(data -> 'metadata', '{}'))\n                        WHERE json_each.type = 'text'\n                          AND json_each.value LIKE ? ESCAPE '!')\n             OR EXISTS (SELECT 1 FROM json_each(COALESCE(data -> 'metadata' -> 'custom_fields', '{}'))\n                        WHERE json_each.type = 'text'\n                          AND json_each.value LIKE ? ESCAPE '!'))",
        );
        for _ in 0..4 {
            where_params.push(pattern.clone());
        }
    }
    for (field_id, value) in active_filters {
        where_sql.push_str(
            " AND EXISTS (SELECT 1 FROM json_each(COALESCE(data -> 'metadata' -> 'custom_fields', '{}'))\n                    WHERE json_each.key = ? AND json_each.type = 'text'\n                      AND json_each.value = ?)",
        );
        where_params.push((*field_id).clone());
        where_params.push((*value).clone());
    }

    let total: i64 = tx
        .query_row(
            &format!("SELECT COUNT(*) FROM records WHERE 1 = 1{where_sql}"),
            params_from_iter(where_params.iter()),
            |row| row.get(0),
        )
        .map_err(db_err)?;

    let mut page_sql = format!(
        "SELECT data FROM records WHERE 1 = 1{where_sql} ORDER BY {}",
        sort_clause(sort)
    );
    let limit = page_size as i64;
    let offset = (page as u64)
        .saturating_sub(1)
        .saturating_mul(page_size as u64) as i64;
    page_sql.push_str(" LIMIT ? OFFSET ?");
    let mut page_params = where_params;
    page_params.push(limit.to_string());
    page_params.push(offset.to_string());
    let mut stmt = tx.prepare(&page_sql).map_err(db_err)?;
    let rows = stmt
        .query_map(params_from_iter(page_params.iter()), |row| {
            row.get::<_, String>(0)
        })
        .map_err(db_err)?;
    let mut items = Vec::new();
    for row in rows {
        let text = row.map_err(db_err)?;
        items.push(serde_json::from_str(&text).map_err(|e| format!("记录数据无效：{e}"))?);
    }
    drop(stmt);

    // Stats stay global: every record counts, malformed optional values read as 0,
    // and avg_ratio is the arithmetic mean of the numeric percentage like the UI.
    let (stat_total, source_bytes, compressed_bytes, ratio_sum): (i64, f64, f64, f64) = tx
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CAST(json_extract(data, '$.source_size.bytes') AS INTEGER)), 0),
                    COALESCE(SUM(CAST(json_extract(data, '$.compressed_size.bytes') AS INTEGER)), 0),
                    COALESCE(SUM(CAST(REPLACE(TRIM(json_extract(data, '$.compression_ratio')), '%', '') AS REAL)), 0)
             FROM records",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(db_err)?;
    let avg_ratio = if stat_total > 0 {
        ratio_sum / stat_total as f64
    } else {
        0.0
    };

    let mut filter_options = Map::new();
    for field_id in option_fields {
        let mut stmt = tx
            .prepare(
                "SELECT DISTINCT json_each.value
                 FROM records, json_each(COALESCE(data -> 'metadata' -> 'custom_fields', '{}'))
                 WHERE json_each.key = ?1 AND json_each.type = 'text'
                   AND json_each.value != ''
                 ORDER BY json_each.value",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![field_id], |row| row.get::<_, String>(0))
            .map_err(db_err)?;
        let mut values = Vec::new();
        for row in rows {
            values.push(Value::String(row.map_err(db_err)?));
        }
        drop(stmt);
        filter_options.insert((*field_id).to_string(), Value::Array(values));
    }

    Ok(json!({
        "items": Value::Array(items),
        "page": page,
        "page_size": page_size,
        "total": total,
        "stats": {
            "total": stat_total,
            "source_bytes": source_bytes,
            "compressed_bytes": compressed_bytes,
            "avg_ratio": avg_ratio,
        },
        "filter_options": Value::Object(filter_options),
    }))
}

pub fn export(file: &Path) -> Result<(), String> {
    if file == catalog_path()
        || file == database_path()
        || file.parent().and_then(|p| fs::canonicalize(p).ok())
            == catalog_path()
                .parent()
                .and_then(|p| fs::canonicalize(p).ok())
    {
        return Err("请选择记录目录以外的备份位置".into());
    }
    let data = {
        let _guard = lock_guard();
        let mut conn = open_db()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(db_err)?;
        let data = read_catalog(&tx)?;
        tx.commit().map_err(db_err)?;
        data
    };
    atomic_write(file, &data, false)
}

#[derive(Default)]
pub struct HashPasswords {
    connection: Option<Connection>,
    by_size: HashMap<u64, HashMap<String, Vec<String>>>,
    cache: HashMap<PathBuf, Vec<String>>,
}

impl HashPasswords {
    pub fn load(enabled: bool, cb: &dyn RunCallback) -> Self {
        let mut index = Self::default();
        // No store at all: keep the unpack path from creating an empty database.
        if !enabled || (!database_path().is_file() && !catalog_path().is_file()) {
            return index;
        }
        let connection = {
            let _guard = lock_guard();
            open_db()
        };
        match connection {
            Ok(connection) => index.connection = Some(connection),
            Err(e) => cb.on_log(
                &format!("[警告] 哈希密码记录读取失败，将使用规则和密码库：{e}"),
                LogLevel::Warn,
            ),
        }
        index
    }

    fn index_for_size(
        conn: &Connection,
        size: u64,
    ) -> Result<HashMap<String, Vec<String>>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT r.password, h.sha256
                 FROM record_hashes h JOIN records r ON r.record_id = h.record_id
                 WHERE h.size_bytes = ?1 AND r.password IS NOT NULL AND r.password != ''
                 ORDER BY r.rowid, h.rowid",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![size as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_err)?;
        let mut candidates: HashMap<String, Vec<String>> = HashMap::new();
        for row in rows {
            let (password, sha256) = row.map_err(db_err)?;
            if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let passwords = candidates.entry(sha256.to_ascii_lowercase()).or_default();
            if !passwords.iter().any(|p| *p == password) {
                passwords.push(password);
            }
        }
        Ok(candidates)
    }

    fn candidates_for_size(
        &mut self,
        size: u64,
        cb: &dyn RunCallback,
    ) -> Option<&HashMap<String, Vec<String>>> {
        if !self.by_size.contains_key(&size) {
            let conn = self.connection.as_ref()?;
            match Self::index_for_size(conn, size) {
                Ok(candidates) => {
                    self.by_size.insert(size, candidates);
                }
                Err(e) => {
                    cb.on_log(
                        &format!("[警告] 哈希密码记录读取失败，将使用规则和密码库：{e}"),
                        LogLevel::Warn,
                    );
                    self.connection = None;
                    return None;
                }
            }
        }
        self.by_size
            .get(&size)
            .filter(|candidates| !candidates.is_empty())
    }

    pub fn passwords_for(&mut self, file: &Path, cb: &dyn RunCallback) -> Vec<String> {
        if let Some(cached) = self.cache.get(file) {
            return cached.clone();
        }
        let mut passwords = Vec::new();
        if let Some(candidates) = file
            .metadata()
            .ok()
            .and_then(|m| self.candidates_for_size(m.len(), cb))
        {
            cb.on_log(
                &format!(
                    "[哈希] 正在比对 {} 的 SHA-256",
                    file.file_name().unwrap_or_default().to_string_lossy()
                ),
                LogLevel::Info,
            );
            match hash_file(file, cb) {
                Ok(hash) => {
                    if let Some(pws) = candidates.get(&hash.sha256) {
                        passwords = pws.clone();
                        cb.on_log("[哈希] 命中压缩记录，将优先尝试记录密码", LogLevel::Info);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => cb.on_log(
                    &format!("[警告] 哈希比对失败，将继续使用规则和密码库：{e}"),
                    LogLevel::Warn,
                ),
            }
        }
        self.cache.insert(file.to_path_buf(), passwords.clone());
        passwords
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::MutexGuard;

    /// Serializes tests that point UNZIP_CATALOG_PATH at a temp directory.
    struct CatalogEnv {
        _guard: MutexGuard<'static, ()>,
        dir: tempfile::TempDir,
    }

    impl CatalogEnv {
        fn new() -> Self {
            let guard = CATALOG_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            let dir = tempfile::TempDir::new().expect("temp dir");
            std::env::set_var("UNZIP_CATALOG_PATH", dir.path().join("pack_mapping.json"));
            Self { _guard: guard, dir }
        }

        fn json_path(&self) -> PathBuf {
            self.dir.path().join("pack_mapping.json")
        }

        fn write_json(&self, text: &str) {
            fs::write(self.json_path(), text).expect("write catalog json");
        }
    }

    impl Drop for CatalogEnv {
        fn drop(&mut self) {
            std::env::remove_var("UNZIP_CATALOG_PATH");
        }
    }

    fn record(id: &str, name: &str) -> Value {
        json!({
            "record_id": id,
            "archive_path": format!("D:/Archives/{name}"),
            "output_dir": "D:/Archives",
            "source_name": name,
            "archive_name": name,
            "password": null,
            "source_size": {"bytes": 2, "mb": "0.00", "gb": "0.000", "human": "2 B"},
            "compressed_size": {"bytes": 1, "mb": "0.00", "gb": "0.000", "human": "1 B"},
            "compression_ratio": "50.0%",
            "is_volume": false,
            "volume_count": 1,
            "created_at": "2026-10-16",
        })
    }

    #[derive(Default)]
    struct LogSink(Mutex<Vec<String>>);

    impl RunCallback for LogSink {
        fn on_log(&self, msg: &str, _level: LogLevel) {
            self.0.lock().unwrap().push(msg.to_string());
        }
    }

    impl LogSink {
        fn contains(&self, needle: &str) -> bool {
            self.0.lock().unwrap().iter().any(|m| m.contains(needle))
        }
    }

    #[test]
    fn first_load_migrates_valid_json_once_and_preserves_source() {
        let env = CatalogEnv::new();
        let original = json!({
            "schema_version": 2,
            "generated_at": "2026-10-01",
            "updated_at_unix": 1792108800u64,
            "custom_top": {"keep": true},
            "mappings": [
                record("d:/archives/a.zip", "a.zip"),
                record("d:/archives/b.zip", "b.zip"),
            ],
        });
        let text = serde_json::to_string_pretty(&original).unwrap();
        env.write_json(&text);

        let data = load().expect("load migrates");
        assert_eq!(data["total_count"], json!(2));
        assert_eq!(data["mappings"].as_array().unwrap().len(), 2);
        assert_eq!(data["generated_at"], json!("2026-10-01"));
        assert_eq!(data["updated_at_unix"], json!(1792108800));
        assert_eq!(data["custom_top"], json!({"keep": true}));
        // The legacy JSON stays byte-identical on disk.
        assert_eq!(fs::read_to_string(env.json_path()).unwrap(), text);
        assert!(database_path().is_file());

        // SQLite is authoritative after init: removing the JSON changes nothing.
        fs::remove_file(env.json_path()).unwrap();
        let again = load().expect("load after source removal");
        assert_eq!(again["total_count"], json!(2));
    }

    #[test]
    fn invalid_central_json_aborts_without_creating_database() {
        let env = CatalogEnv::new();
        env.write_json("{ not json");
        let err = load().expect_err("invalid JSON must error");
        assert!(err.contains("原文件已保留"), "unexpected error: {err}");
        assert!(!database_path().exists());
        assert_eq!(fs::read_to_string(env.json_path()).unwrap(), "{ not json");
    }

    #[test]
    fn incomplete_central_record_aborts_without_creating_database() {
        let env = CatalogEnv::new();
        let mut bad = record("d:/archives/a.zip", "a.zip");
        bad.as_object_mut().unwrap().remove("created_at");
        env.write_json(&json!({"schema_version": 2, "mappings": [bad]}).to_string());
        let err = load().expect_err("incomplete record must error");
        assert!(err.contains("不完整的条目"), "unexpected error: {err}");
        assert!(!database_path().exists());
    }

    #[test]
    fn duplicate_ids_in_legacy_json_abort_instead_of_overwriting_a_record() {
        let env = CatalogEnv::new();
        let text = json!({
            "schema_version": 2,
            "mappings": [
                record("d:/archives/same.zip", "first.zip"),
                record("d:/archives/same.zip", "second.zip"),
            ],
        })
        .to_string();
        env.write_json(&text);
        let err = load().expect_err("duplicate legacy IDs must not be silently merged");
        assert!(err.contains("重复 ID"), "unexpected error: {err}");
        assert_eq!(fs::read_to_string(env.json_path()).unwrap(), text);
    }

    #[test]
    fn legacy_json_with_repeated_hash_filename_migrates_without_losing_candidates() {
        let env = CatalogEnv::new();
        let mut item = record("d:/archives/a.zip", "a.zip");
        item["password"] = json!("pw");
        item["output_hashes"] = json!([
            {"file_name": "a.zip", "size_bytes": 1, "sha256": "a".repeat(64)},
            {"file_name": "a.zip", "size_bytes": 2, "sha256": "b".repeat(64)},
        ]);
        env.write_json(&json!({"schema_version": 2, "mappings": [item]}).to_string());
        let data = load().expect("valid legacy JSON should migrate");
        assert_eq!(
            data["mappings"][0]["output_hashes"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let cb = LogSink::default();
        let mut index = HashPasswords::load(true, &cb);
        assert_eq!(
            index.candidates_for_size(1, &cb).unwrap()[&"a".repeat(64)],
            ["pw"]
        );
        assert_eq!(
            index.candidates_for_size(2, &cb).unwrap()[&"b".repeat(64)],
            ["pw"]
        );
    }

    #[test]
    fn missing_catalog_starts_empty_database() {
        let env = CatalogEnv::new();
        let _ = &env;
        let data = load().expect("fresh load");
        assert_eq!(data["mappings"].as_array().unwrap().len(), 0);
        assert_eq!(data["schema_version"], json!(2));
        assert!(database_path().is_file());
    }

    #[test]
    fn upsert_merges_preserving_metadata_and_unknown_fields() {
        let env = CatalogEnv::new();
        let _ = &env;
        let mut first = record("d:/archives/a.zip", "a.zip");
        first["password"] = json!("pw1");
        first["metadata"] = json!({"notes": "keep me"});
        first["custom_unknown"] = json!("u1");
        upsert(vec![first]).expect("first upsert");

        let mut second = record("d:/archives/a.zip", "a.zip");
        second["archive_name"] = json!("a-renamed.zip");
        second["password"] = json!("pw2");
        upsert(vec![second]).expect("second upsert");

        let data = load().expect("load");
        assert_eq!(data["total_count"], json!(1));
        let rows = data["mappings"].as_array().unwrap();
        let row = &rows[0];
        assert_eq!(row["archive_name"], json!("a-renamed.zip"));
        assert_eq!(row["password"], json!("pw2"));
        assert_eq!(row["metadata"], json!({"notes": "keep me"}));
        assert_eq!(row["custom_unknown"], json!("u1"));
        assert!(data["updated_at_unix"].as_u64().is_some());
    }

    #[test]
    fn upsert_rolls_back_entire_batch_on_invalid_item() {
        let env = CatalogEnv::new();
        let _ = &env;
        let mut bad = record("d:/archives/bad.zip", "bad.zip");
        bad.as_object_mut().unwrap().remove("record_id");
        let err = upsert(vec![record("d:/archives/good.zip", "good.zip"), bad])
            .expect_err("invalid item must error");
        assert!(err.contains("记录缺少 ID"), "unexpected error: {err}");
        let data = load().expect("load after rollback");
        assert_eq!(data["mappings"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn legacy_import_adds_only_new_records_and_keeps_existing() {
        let env = CatalogEnv::new();
        let mut central = record("d:/out/x.zip", "x.zip");
        central["metadata"] = json!({"notes": "new notes"});
        upsert(vec![central]).expect("seed central");

        let legacy_dir = env.dir.path().join("legacy_out");
        fs::create_dir_all(&legacy_dir).unwrap();
        let legacy_file = legacy_dir.join("pack_mapping.json");
        let mut dup = record("d:/out/x.zip", "x.zip");
        dup["archive_path"] = json!("D:\\Out\\X.zip");
        dup["metadata"] = json!({"notes": "old notes"});
        let mut fresh = record("d:/out/y.zip", "y.zip");
        fresh["archive_path"] = json!("D:\\Out\\y.zip");
        fresh["output_dir"] = json!("D:\\Out");
        let legacy_text = serde_json::to_string_pretty(&json!({
            "schema_version": 2,
            "mappings": [dup, fresh],
        }))
        .unwrap();
        fs::write(&legacy_file, &legacy_text).unwrap();

        let imported = import_legacy(&legacy_file).expect("import");
        assert_eq!(imported, 1);
        let data = load().expect("load");
        let rows = data["mappings"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        let x = rows
            .iter()
            .find(|r| r["record_id"] == json!("d:/out/x.zip"))
            .expect("x present");
        assert_eq!(x["metadata"], json!({"notes": "new notes"}));
        assert!(rows.iter().any(|r| r["record_id"] == json!("d:/out/y.zip")));
        // Source JSON untouched; re-import and self-import add nothing.
        assert_eq!(fs::read_to_string(&legacy_file).unwrap(), legacy_text);
        assert_eq!(import_legacy(&legacy_file).expect("re-import"), 0);
        env.write_json("{}");
        assert_eq!(import_legacy(&env.json_path()).expect("self-import"), 0);
        assert_eq!(load().unwrap()["total_count"], json!(2));
    }

    #[test]
    fn legacy_import_rejects_bad_archive_name_without_changes() {
        let env = CatalogEnv::new();
        upsert(vec![record("d:/out/safe.zip", "safe.zip")]).expect("seed");

        let legacy_text =
            json!({"schema_version": 2, "mappings": [record("d:/out/evil.zip", "../escape.zip")]})
                .to_string();
        let legacy_file = env.dir.path().join("legacy_bad.json");
        fs::write(&legacy_file, &legacy_text).unwrap();

        let err = import_legacy(&legacy_file).expect_err("bad name must error");
        assert!(
            err.contains("无效的压缩包文件名"),
            "unexpected error: {err}"
        );
        // Failed validation applies nothing and leaves both files untouched.
        assert_eq!(load().unwrap()["mappings"].as_array().unwrap().len(), 1);
        assert_eq!(fs::read_to_string(&legacy_file).unwrap(), legacy_text);
    }

    #[test]
    fn export_writes_full_catalog_outside_records_dir_only() {
        let env = CatalogEnv::new();
        upsert(vec![record("d:/out/e.zip", "e.zip")]).expect("seed");
        let data = load().unwrap();

        let export_dir = tempfile::TempDir::new().expect("export dir");
        let target = export_dir.path().join("backup.json");
        export(&target).expect("export");
        let exported: Value = serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
        assert_eq!(exported["mappings"], data["mappings"]);
        assert_eq!(exported["total_count"], data["total_count"]);
        assert!(exported["updated_at_unix"].is_u64());

        let inside = env.dir.path().join("backup2.json");
        let err = export(&inside).expect_err("same-dir export must fail");
        assert!(err.contains("记录目录以外"), "unexpected error: {err}");
        assert!(export(&database_path()).is_err(), "must not export over DB");
        assert!(
            export(&env.json_path()).is_err(),
            "must not export over JSON"
        );
    }

    #[test]
    fn exported_json_restores_password_metadata_and_unknown_fields_into_fresh_db() {
        let env = CatalogEnv::new();
        let mut item = record("d:/out/restore.zip", "restore.zip");
        item["password"] = json!("secret");
        item["metadata"] = json!({"notes": "keep", "custom_fields": {"category": "docs"}});
        item["future_field"] = json!({"preserve": true});
        env.write_json(
            &json!({"schema_version": 2, "future_top": [1, 2], "mappings": [item]}).to_string(),
        );
        let backup_dir = tempfile::TempDir::new().unwrap();
        let backup = backup_dir.path().join("backup.json");
        export(&backup).unwrap();

        std::env::set_var("UNZIP_CATALOG_PATH", env.dir.path().join("restored.json"));
        assert_eq!(import_legacy(&backup).unwrap(), 1);
        let restored = load().unwrap();
        assert_eq!(restored["future_top"], json!([1, 2]));
        let row = &restored["mappings"][0];
        assert_eq!(row["password"], json!("secret"));
        assert_eq!(row["metadata"]["notes"], json!("keep"));
        assert_eq!(row["metadata"]["custom_fields"]["category"], json!("docs"));
        assert_eq!(row["future_field"], json!({"preserve": true}));
    }

    #[test]
    fn update_metadata_replaces_only_metadata() {
        let env = CatalogEnv::new();
        let _ = &env;
        let mut rec = record("d:/out/m.zip", "m.zip");
        rec["password"] = json!("pw");
        rec["output_hashes"] =
            json!([{"file_name": "m.zip", "size_bytes": 3, "sha256": "a".repeat(64)}]);
        upsert(vec![rec]).expect("seed");

        update_metadata("d:/out/m.zip", json!({"notes": "n"})).expect("update");
        let data = load().unwrap();
        let row = &data["mappings"].as_array().unwrap()[0];
        assert_eq!(row["metadata"], json!({"notes": "n"}));
        assert_eq!(row["password"], json!("pw"));
        assert_eq!(row["output_hashes"].as_array().unwrap().len(), 1);

        let err = update_metadata("d:/out/missing.zip", json!({"notes": "x"}))
            .expect_err("missing record");
        assert!(err.contains("记录已不存在"), "unexpected error: {err}");
        let err = update_metadata("d:/out/m.zip", json!("nope")).expect_err("non-object metadata");
        assert!(err.contains("格式无效"), "unexpected error: {err}");
        let data = load().unwrap();
        let row = &data["mappings"].as_array().unwrap()[0];
        assert_eq!(row["metadata"], json!({"notes": "n"}));
    }

    #[test]
    fn hash_passwords_lookup_hits_db_and_falls_back_with_warning() {
        use sha2::{Digest, Sha256};

        let env = CatalogEnv::new();
        let content = b"pack me up";
        let sha256 = format!("{:x}", Sha256::digest(content));
        let file_path = env.dir.path().join("target.bin");
        fs::write(&file_path, content).unwrap();

        let mut rec = record("d:/out/h.zip", "h.zip");
        rec["password"] = json!("secret");
        rec["output_hashes"] = json!([{
            "file_name": "h.zip",
            "size_bytes": content.len(),
            "sha256": sha256,
        }]);
        upsert(vec![rec]).expect("seed");

        let cb = LogSink::default();
        let mut index = HashPasswords::load(true, &cb);
        assert!(
            index.by_size.is_empty(),
            "hash rows should load by size on demand"
        );
        assert_eq!(
            index.passwords_for(&file_path, &cb),
            vec!["secret".to_string()]
        );
        assert_eq!(index.by_size.len(), 1);
        // Second lookup is served from the per-path cache.
        assert_eq!(index.passwords_for(&file_path, &cb).len(), 1);

        let mut disabled = HashPasswords::load(false, &cb);
        assert!(disabled.passwords_for(&file_path, &cb).is_empty());

        // A corrupted store falls back with a warning instead of failing unpack.
        fs::write(database_path(), b"not a sqlite database").unwrap();
        let mut broken = HashPasswords::load(true, &cb);
        assert!(broken.passwords_for(&file_path, &cb).is_empty());
        assert!(
            cb.contains("[警告] 哈希密码记录读取失败"),
            "expected fallback warning"
        );
    }

    #[test]
    fn hash_passwords_empty_when_no_store_exists() {
        let env = CatalogEnv::new();
        let _ = &env;
        let cb = LogSink::default();
        let file_path = env.dir.path().join("anything.bin");
        fs::write(&file_path, b"x").unwrap();
        let mut index = HashPasswords::load(true, &cb);
        assert!(index.passwords_for(&file_path, &cb).is_empty());
        assert!(!database_path().exists(), "lookup must not create a DB");
        assert!(!cb.contains("哈希密码记录读取失败"));
    }

    #[test]
    fn updating_record_keeps_password_candidate_order() {
        let _env = CatalogEnv::new();
        let hash = "a".repeat(64);
        for (id, name, password) in [
            ("d:/out/first.zip", "first.zip", "first"),
            ("d:/out/second.zip", "second.zip", "second"),
        ] {
            let mut item = record(id, name);
            item["password"] = json!(password);
            item["output_hashes"] = json!([{
                "file_name": name,
                "size_bytes": 3,
                "sha256": hash,
            }]);
            upsert(vec![item]).unwrap();
        }
        let cb = LogSink::default();
        let mut before = HashPasswords::load(true, &cb);
        assert_eq!(
            before.candidates_for_size(3, &cb).unwrap()[&hash],
            ["first", "second"]
        );

        let mut updated = record("d:/out/first.zip", "first.zip");
        updated["password"] = json!("first");
        updated["output_hashes"] = json!([{
            "file_name": "first.zip", "size_bytes": 3, "sha256": hash,
        }]);
        upsert(vec![updated]).unwrap();
        let mut after = HashPasswords::load(true, &cb);
        assert_eq!(
            after.candidates_for_size(3, &cb).unwrap()[&hash],
            ["first", "second"]
        );
    }

    #[test]
    fn custom_catalog_path_uses_its_own_sqlite_file() {
        let env = CatalogEnv::new();
        let custom_json = env.dir.path().join("separate_records.json");
        std::env::set_var("UNZIP_CATALOG_PATH", &custom_json);
        assert_eq!(database_path(), custom_json.with_extension("sqlite3"));
        upsert(vec![record("d:/archives/custom.zip", "custom.zip")]).unwrap();
        assert!(custom_json.with_extension("sqlite3").is_file());
        assert!(!env.dir.path().join("pack_mapping.sqlite3").exists());
    }

    #[test]
    fn malformed_extra_metadata_is_reported_without_dropping_it() {
        let _env = CatalogEnv::new();
        load().unwrap();
        let conn = Connection::open(database_path()).unwrap();
        conn.execute(
            "INSERT INTO catalog_meta(key, value) VALUES(?1, ?2)",
            params![META_EXTRA_TOP_LEVEL, "not-json"],
        )
        .unwrap();
        assert!(load().unwrap_err().contains("记录元数据无效"));
    }

    fn sized_record(
        id: &str,
        name: &str,
        source_bytes: u64,
        compressed_bytes: u64,
        ratio: &str,
        date: &str,
    ) -> Value {
        let mut item = record(id, name);
        item["source_size"] =
            json!({"bytes": source_bytes, "mb": "0.00", "gb": "0.000", "human": "0 B"});
        item["compressed_size"] =
            json!({"bytes": compressed_bytes, "mb": "0.00", "gb": "0.000", "human": "0 B"});
        item["compression_ratio"] = json!(ratio);
        item["created_at"] = json!(date);
        item
    }

    fn meta_record(id: &str, name: &str, notes: &str, fields: Value) -> Value {
        let mut item = sized_record(id, name, 1000, 500, "50.0%", "2026-01-01");
        item["metadata"] = json!({"notes": notes, "custom_fields": fields});
        item
    }

    fn page_names(value: &Value) -> Vec<String> {
        value["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["source_name"].as_str().unwrap().to_string())
            .collect()
    }

    fn no_filters() -> HashMap<String, String> {
        HashMap::new()
    }

    #[test]
    fn query_page_paginates_three_pages_with_deterministic_boundary() {
        let env = CatalogEnv::new();
        let _ = &env;
        upsert(vec![
            record("d:/out/g1.zip", "g1.zip"),
            record("d:/out/g2.zip", "g2.zip"),
            record("d:/out/g3.zip", "g3.zip"),
            record("d:/out/g4.zip", "g4.zip"),
            record("d:/out/g5.zip", "g5.zip"),
            record("d:/out/g6.zip", "g6.zip"),
            record("d:/out/g7.zip", "g7.zip"),
        ])
        .unwrap();

        let first = query_page(1, 3, "", "name", &no_filters(), &[]).unwrap();
        assert_eq!(first["page"], json!(1));
        assert_eq!(first["page_size"], json!(3));
        assert_eq!(first["total"], json!(7));
        assert_eq!(page_names(&first), ["g1.zip", "g2.zip", "g3.zip"]);

        let second = query_page(2, 3, "", "name", &no_filters(), &[]).unwrap();
        assert_eq!(page_names(&second), ["g4.zip", "g5.zip", "g6.zip"]);
        // Empty third page: total is unaffected by LIMIT.
        let third = query_page(3, 3, "", "name", &no_filters(), &[]).unwrap();
        assert_eq!(page_names(&third), ["g7.zip"]);
        let beyond = query_page(4, 3, "", "name", &no_filters(), &[]).unwrap();
        assert_eq!(beyond["items"].as_array().unwrap().len(), 0);
        assert_eq!(beyond["total"], json!(7));

        let stats = &first["stats"];
        assert_eq!(stats["total"], json!(7));
        assert_eq!(stats["source_bytes"].as_f64(), Some(14.0));
        assert_eq!(stats["compressed_bytes"].as_f64(), Some(7.0));
        assert_eq!(stats["avg_ratio"].as_f64(), Some(50.0));
    }

    #[test]
    fn query_page_search_covers_notes_custom_fields_and_literal_wildcards() {
        let env = CatalogEnv::new();
        let _ = &env;
        upsert(vec![
            meta_record("d:/out/a.zip", "my_file.zip", "核心说明", json!({})),
            meta_record(
                "d:/out/b.zip",
                "myXfile.zip",
                "plain",
                json!({"category": "docs"}),
            ),
            meta_record(
                "d:/out/c.zip",
                "gamma.zip",
                "save 10% off",
                json!({"category": "old tag"}),
            ),
            meta_record("d:/out/d.zip", "delta.zip", "", json!({})),
        ])
        .unwrap();

        // ASCII case-insensitive across names and notes.
        assert_eq!(
            query_page(1, 10, "MY_FILE", "name", &no_filters(), &[]).unwrap()["total"],
            json!(1)
        );
        assert_eq!(
            query_page(1, 10, "核心", "name", &no_filters(), &[]).unwrap()["total"],
            json!(1)
        );
        // Custom field values are searched too.
        assert_eq!(
            query_page(1, 10, "docs", "name", &no_filters(), &[]).unwrap()["total"],
            json!(1)
        );
        // `%` and `_` are literal, not wildcards.
        let percent = query_page(1, 10, "10%", "name", &no_filters(), &[]).unwrap();
        assert_eq!(percent["total"], json!(1));
        assert_eq!(page_names(&percent), ["gamma.zip"]);
        let underscored = query_page(1, 10, "my_file", "name", &no_filters(), &[]).unwrap();
        assert_eq!(underscored["total"], json!(1));
        assert_eq!(
            page_names(&underscored),
            ["my_file.zip"],
            "the `_` must not act as a wildcard matching myXfile.zip"
        );
        // No match: empty items, stats stay global.
        let none = query_page(1, 10, "不存在zzz", "name", &no_filters(), &[]).unwrap();
        assert_eq!(none["total"], json!(0));
        assert_eq!(none["items"].as_array().unwrap().len(), 0);
        assert_eq!(none["stats"]["total"], json!(4));
    }

    #[test]
    fn query_page_filters_and_options_include_values_beyond_configured_options() {
        let env = CatalogEnv::new();
        let _ = &env;
        upsert(vec![
            meta_record(
                "d:/out/a.zip",
                "a.zip",
                "",
                json!({"category": "docs", "genre": "rpg", "genre-tag": "classic", "flag": true}),
            ),
            meta_record(
                "d:/out/b.zip",
                "b.zip",
                "",
                json!({"category": "docs", "genre": "slg"}),
            ),
            meta_record(
                "d:/out/c.zip",
                "c.zip",
                "",
                json!({"category": "removed choice"}),
            ),
            record("d:/out/d.zip", "d.zip"),
        ])
        .unwrap();

        let mut filters = no_filters();
        filters.insert("category".into(), "docs".into());
        let page = query_page(1, 10, "", "name", &filters, &["category".to_string()]).unwrap();
        assert_eq!(page["total"], json!(2));
        assert_eq!(page_names(&page), ["a.zip", "b.zip"]);
        let mut hyphen_filter = no_filters();
        hyphen_filter.insert("genre-tag".into(), "classic".into());
        assert_eq!(
            query_page(
                1,
                10,
                "",
                "name",
                &hyphen_filter,
                &["genre-tag".to_string()]
            )
            .unwrap()["total"],
            json!(1)
        );
        assert_eq!(
            page["filter_options"]["category"],
            json!(["docs", "removed choice"])
        );
        // Two filters AND together.
        filters.insert("genre".into(), "rpg".into());
        assert_eq!(
            query_page(1, 10, "", "name", &filters, &[]).unwrap()["total"],
            json!(1)
        );
        // An empty filter value is ignored like the UI's 全部.
        filters.insert("genre".into(), "".into());
        assert_eq!(
            query_page(1, 10, "", "name", &filters, &[]).unwrap()["total"],
            json!(2)
        );
        // Non-string field values (e.g. checkbox booleans) never match or leak.
        filters.insert("flag".into(), "true".into());
        assert_eq!(
            query_page(1, 10, "", "name", &filters, &[]).unwrap()["total"],
            json!(0)
        );
        let options = query_page(1, 10, "", "name", &no_filters(), &["flag".to_string()]).unwrap();
        assert_eq!(options["filter_options"]["flag"], json!([]));
    }

    #[test]
    fn query_page_all_sort_modes_are_stable() {
        let env = CatalogEnv::new();
        let _ = &env;
        upsert(vec![
            sized_record(
                "d:/out/03c.zip",
                "charlie.zip",
                300,
                30,
                "10.0%",
                "2026-03-01",
            ),
            sized_record(
                "d:/out/01a.zip",
                "alpha.zip",
                100,
                10,
                "30.0%",
                "2026-01-01",
            ),
            sized_record(
                "d:/out/02b.zip",
                "bravo.zip",
                200,
                20,
                "20.0%",
                "2026-02-01",
            ),
            sized_record("d:/out/04d.zip", "原神.zip", 400, 40, "5.0%", "2026-02-01"),
        ])
        .unwrap();

        // UTF-8 BINARY collation orders ASCII before CJK bytes.
        assert_eq!(
            page_names(&query_page(1, 10, "", "name", &no_filters(), &[]).unwrap()),
            ["alpha.zip", "bravo.zip", "charlie.zip", "原神.zip"]
        );
        assert_eq!(
            page_names(&query_page(1, 10, "", "size", &no_filters(), &[]).unwrap()),
            ["原神.zip", "charlie.zip", "bravo.zip", "alpha.zip"]
        );
        assert_eq!(
            page_names(&query_page(1, 10, "", "ratio", &no_filters(), &[]).unwrap()),
            ["原神.zip", "charlie.zip", "bravo.zip", "alpha.zip"]
        );
        // Date descending; equal dates break ties deterministically by record_id.
        assert_eq!(
            page_names(&query_page(1, 10, "", "date", &no_filters(), &[]).unwrap()),
            ["charlie.zip", "bravo.zip", "原神.zip", "alpha.zip"]
        );
        // Unknown sort falls back to name ascending.
        assert_eq!(
            page_names(&query_page(1, 10, "", "bogus", &no_filters(), &[]).unwrap()),
            ["alpha.zip", "bravo.zip", "charlie.zip", "原神.zip"]
        );
    }

    #[test]
    fn query_page_stats_stay_global_while_total_follows_filters() {
        let env = CatalogEnv::new();
        let _ = &env;
        upsert(vec![
            sized_record("d:/out/a.zip", "a.zip", 1000, 100, "50.0%", "2026-01-01"),
            sized_record("d:/out/b.zip", "b.zip", 3000, 600, "25.0%", "2026-01-02"),
        ])
        .unwrap();
        let mut filters = no_filters();
        filters.insert("kind".into(), "none".into());
        let page = query_page(1, 10, "", "name", &filters, &[]).unwrap();
        assert_eq!(page["total"], json!(0));
        let stats = &page["stats"];
        assert_eq!(stats["total"], json!(2));
        assert_eq!(stats["source_bytes"].as_f64(), Some(4000.0));
        assert_eq!(stats["compressed_bytes"].as_f64(), Some(700.0));
        // (50 + 25) / 2 like the UI's arithmetic mean of percentages.
        assert_eq!(stats["avg_ratio"].as_f64(), Some(37.5));

        // Empty library: stats are zero, not NaN.
        let empty_dir = tempfile::TempDir::new().unwrap();
        std::env::set_var(
            "UNZIP_CATALOG_PATH",
            empty_dir.path().join("pack_mapping.json"),
        );
        let empty = query_page(1, 10, "", "name", &no_filters(), &[]).unwrap();
        assert_eq!(empty["total"], json!(0));
        assert_eq!(empty["stats"]["avg_ratio"], json!(0.0));
    }

    #[test]
    fn query_page_rejects_invalid_arguments() {
        let env = CatalogEnv::new();
        let _ = &env;
        upsert(vec![record("d:/out/a.zip", "a.zip")]).unwrap();
        assert!(query_page(0, 10, "", "name", &no_filters(), &[]).is_err());
        assert!(query_page(1, 0, "", "name", &no_filters(), &[]).is_err());
        assert!(query_page(1, 101, "", "name", &no_filters(), &[]).is_err());
        let mut bad_field = no_filters();
        bad_field.insert("bad id!".into(), "x".into());
        assert!(query_page(1, 10, "", "name", &bad_field, &[]).is_err());
        assert!(query_page(1, 10, "", "name", &no_filters(), &["bad id!".to_string()]).is_err());
        let too_many: HashMap<String, String> =
            (0..20).map(|i| (format!("f{i}"), "x".into())).collect();
        assert!(query_page(1, 10, "", "name", &too_many, &[]).is_err());
        // Page size boundary values are accepted.
        assert!(query_page(1, 1, "", "name", &no_filters(), &[]).is_ok());
        assert!(query_page(1, 100, "", "name", &no_filters(), &[]).is_ok());
    }

    #[test]
    fn query_page_reflects_metadata_updates() {
        let env = CatalogEnv::new();
        let _ = &env;
        upsert(vec![
            meta_record("d:/out/a.zip", "a.zip", "", json!({"category": "docs"})),
            record("d:/out/b.zip", "b.zip"),
        ])
        .unwrap();
        assert_eq!(
            query_page(1, 10, "hello", "name", &no_filters(), &[]).unwrap()["total"],
            json!(0)
        );
        let mut filters = no_filters();
        filters.insert("category".into(), "docs".into());
        assert_eq!(
            query_page(1, 10, "", "name", &filters, &[]).unwrap()["total"],
            json!(1)
        );

        update_metadata(
            "d:/out/b.zip",
            json!({"notes": "hello", "custom_fields": {"category": "docs"}}),
        )
        .unwrap();
        let found = query_page(1, 10, "hello", "name", &no_filters(), &[]).unwrap();
        assert_eq!(found["total"], json!(1));
        assert_eq!(page_names(&found), ["b.zip"]);
        assert_eq!(
            query_page(1, 10, "", "name", &filters, &[]).unwrap()["total"],
            json!(2)
        );
        // Distinct option values follow the update too.
        let options =
            query_page(1, 10, "", "name", &no_filters(), &["category".to_string()]).unwrap();
        assert_eq!(options["filter_options"]["category"], json!(["docs"]));
    }
}
