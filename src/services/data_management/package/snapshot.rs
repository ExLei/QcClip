use super::{DataManagementService, ExportOptions, ImportMode, Manifest, Result, Scope, invalid};
use crate::services::{
    database::Database,
    storage::{RawBlob, RawBlobStore},
};
use rusqlite::{Connection, OptionalExtension, params, types::Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    time::Duration,
};

pub(super) fn copy_current(service: &DataManagementService, target: &Path) -> Result<()> {
    copy_database(service, target)?;
    for path in super::super::managed_files(&service.data_root)? {
        let relative = path
            .strip_prefix(&service.data_root)
            .map_err(|_| super::Error::Message("数据目录无效".into()))?;
        if relative.components().count() == 1
            && relative.to_string_lossy().starts_with("qcclip.sqlite3")
        {
            continue;
        }
        copy_file(&service.data_root, target, &relative.to_string_lossy())?;
    }
    Ok(())
}

fn copy_database(service: &DataManagementService, target: &Path) -> Result<()> {
    let source = service.database.connect_read()?;
    let writable = service.database.connect()?;
    if !Database::maintenance_supported(&writable)? {
        return invalid("较新结构的数据库请使用新版程序导出");
    }
    let mut destination = Connection::open(target.join("qcclip.sqlite3"))?;
    rusqlite::backup::Backup::new(&source, &mut destination)?.run_to_completion(
        256,
        Duration::from_millis(5),
        None,
    )?;
    destination.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(())
}

pub(super) fn export(
    service: &DataManagementService,
    root: &Path,
    options: &ExportOptions,
) -> Result<Manifest> {
    copy_database(service, root)?;
    let mut db = Connection::open(root.join("qcclip.sqlite3"))?;
    db.execute_batch("PRAGMA foreign_keys=ON;")?;
    let tx = db.transaction()?;
    if !options.scope.history {
        tx.execute("DELETE FROM clipboard_history", [])?;
    } else if let Some(days) = options.history_days {
        if days == 0 || days > 36500 {
            return invalid("历史天数应为 1–36500");
        }
        let cutoff = chrono::Utc::now().timestamp_millis() - i64::from(days) * 86_400_000;
        tx.execute(
            "DELETE FROM clipboard_history WHERE last_seen_at < ?1",
            [cutoff],
        )?;
    }
    if !options.scope.favorites {
        tx.execute("DELETE FROM favorites", [])?;
    } else if !options.favorite_tags.is_empty() {
        tx.execute_batch("CREATE TEMP TABLE selected_tags (id INTEGER PRIMARY KEY);")?;
        for tag in &options.favorite_tags {
            tx.execute("INSERT OR IGNORE INTO selected_tags VALUES (?1)", [tag])?;
        }
        tx.execute("DELETE FROM favorites WHERE NOT EXISTS (SELECT 1 FROM clipboard_item_tags t JOIN selected_tags s ON s.id=t.tag_id WHERE t.item_id=favorites.item_id)", [])?;
    }
    tx.execute_batch("DELETE FROM clipboard_items WHERE id NOT IN (SELECT item_id FROM favorites UNION SELECT item_id FROM clipboard_history);
        DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM clipboard_item_tags);
        DELETE FROM application_icon_cache;
        DELETE FROM application_icon_assets WHERE id NOT IN (SELECT last_source_app_icon_id FROM clipboard_history WHERE last_source_app_icon_id IS NOT NULL);
        DELETE FROM binary_blobs WHERE id NOT IN (SELECT blob_id FROM clipboard_formats WHERE blob_id IS NOT NULL);
        DELETE FROM sync_items; DELETE FROM sync_watermarks; DELETE FROM sync_receipts; DELETE FROM sync_local_activity; DELETE FROM legacy_import_records;")?;
    tx.commit()?;
    let mut resources = Vec::new();
    for table in ["binary_blobs", "application_icon_assets"] {
        let mut statement = db.prepare(&format!("SELECT relative_path FROM {table}"))?;
        resources.extend(
            statement
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?,
        );
    }
    let store =
        RawBlobStore::new(&service.data_root).map_err(super::DataManagementError::Storage)?;
    let mut statement = db.prepare("SELECT id FROM clipboard_items")?;
    let canonical_root = dunce::canonicalize(&service.data_root)?;
    for item in statement.query_map([], |row| row.get::<_, i64>(0))? {
        if let Some(paths) =
            crate::services::sync::managed_file_list(&db, &store, &service.data_root, item?)?
        {
            for path in paths {
                resources.push(
                    path.strip_prefix(&canonical_root)
                        .map_err(|_| super::Error::Message("受管理文件路径无效".into()))?
                        .to_string_lossy()
                        .to_string(),
                );
            }
        }
    }
    drop(statement);
    for relative in resources {
        copy_file(&service.data_root, root, &relative)?;
    }
    if options.scope.settings && service.data_root.join("config.json").is_file() {
        copy_file(&service.data_root, root, "config.json")?;
        copy_directory(&service.data_root, root, "sound")?;
    }
    if options.scope.gallery {
        copy_directory(&service.data_root, root, "image_library")?;
    }
    db.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
    drop(db);
    manifest(
        root,
        options.scope.clone(),
        options.history_days.is_some() || !options.favorite_tags.is_empty(),
    )
}

pub(super) fn manifest(root: &Path, scope: Scope, filtered: bool) -> Result<Manifest> {
    let connection = Connection::open(root.join("qcclip.sqlite3"))?;
    Ok(Manifest {
        version: 2,
        created_at: chrono::Utc::now().timestamp_millis(),
        scope,
        history_count: connection.query_row("SELECT COUNT(*) FROM clipboard_history", [], |r| {
            r.get::<_, i64>(0)
        })? as u64,
        favorite_count: connection
            .query_row("SELECT COUNT(*) FROM favorites", [], |r| r.get::<_, i64>(0))?
            as u64,
        filtered,
    })
}

pub(super) fn validate(root: &Path, manifest: &Manifest) -> Result<()> {
    if !root.join("qcclip.sqlite3").is_file() {
        return invalid("数据包缺少数据库");
    }
    let db = Database::open(root.join("qcclip.sqlite3"))?;
    let writable = db.connect()?;
    if !Database::maintenance_supported(&writable)? {
        return invalid("请使用新版程序导入此数据包");
    }
    drop(writable);
    let connection = db.connect_read()?;
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if integrity != "ok" || connection.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return invalid("数据包数据库完整性校验失败");
    }
    let actual = self::manifest(root, manifest.scope.clone(), manifest.filtered)?;
    if actual.history_count != manifest.history_count
        || actual.favorite_count != manifest.favorite_count
    {
        return invalid("数据包清单与实际内容不一致");
    }
    let store = RawBlobStore::new(root).map_err(super::DataManagementError::Storage)?;
    let mut statement =
        connection.prepare("SELECT sha256, byte_length, relative_path FROM binary_blobs")?;
    for row in statement.query_map([], |r| {
        Ok((
            r.get::<_, Vec<u8>>(0)?,
            r.get::<_, i64>(1)? as u64,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (hash, length, path) = row?;
        let hash = hash
            .try_into()
            .map_err(|_| super::Error::Message("资源哈希无效".into()))?;
        store
            .verify(&RawBlob::from_record(hash, length, path))
            .map_err(super::DataManagementError::Storage)?;
    }
    use sha2::{Digest, Sha256};
    let mut inline_statement = connection.prepare(
        "SELECT inline_data, data_hash FROM clipboard_formats WHERE storage_kind='inline'",
    )?;
    let mut inline_rows = inline_statement.query([])?;
    while let Some(row) = inline_rows.next()? {
        let bytes: Vec<u8> = row.get(0)?;
        let hash: Vec<u8> = row.get(1)?;
        if hash != Sha256::digest(&bytes).as_slice() {
            return invalid("数据包内的剪贴板内容校验失败");
        }
    }
    let mismatch: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM clipboard_formats f JOIN binary_blobs b ON b.id=f.blob_id WHERE f.data_hash<>b.sha256 OR f.byte_length<>b.byte_length)", [], |r| r.get(0))?;
    if mismatch {
        return invalid("数据包资源与剪贴板格式不一致");
    }
    for row in rows(&connection, "application_icon_assets")? {
        let relative = text(&row, "relative_path")?.replace('\\', "/");
        if !relative.starts_with("data/app-icons/") || relative.split('/').count() != 3 {
            return invalid("数据包图标路径无效");
        }
        let path = super::super::safe_join(root, &relative)?;
        if super::super::hash_file(&path)?.as_slice()
            != match &row["sha256"] {
                Value::Blob(bytes) => bytes.as_slice(),
                _ => return invalid("图标哈希无效"),
            }
        {
            return invalid("数据包图标校验失败");
        }
    }
    if manifest.scope.settings && root.join("config.json").exists() {
        let _: crate::config::AppConfig =
            serde_json::from_slice(&fs::read(root.join("config.json"))?)?;
    }
    // 不读取普通文件引用指向的本机文件，只验证包内受管理的同步文件。
    let mut statement = connection.prepare("SELECT id FROM clipboard_items")?;
    for id in statement.query_map([], |r| r.get::<_, i64>(0))? {
        crate::services::sync::managed_file_list(&connection, &store, root, id?)?;
    }
    Ok(())
}

pub(super) fn copy_file(source: &Path, target: &Path, relative: &str) -> Result<()> {
    let path = super::super::safe_join(source, relative)?;
    if !fs::symlink_metadata(&path)?.is_file() {
        return invalid("数据包资源不是普通文件");
    }
    let canonical = dunce::canonicalize(&path)?;
    if !canonical.starts_with(dunce::canonicalize(source)?) {
        return invalid("数据包资源路径超出数据目录");
    }
    let target = super::super::safe_join(target, relative)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(path, target)?;
    Ok(())
}
fn copy_directory(source: &Path, target: &Path, relative: &str) -> Result<()> {
    let path = super::super::safe_join(source, relative)?;
    if !path.exists() {
        return Ok(());
    }
    fs::create_dir_all(super::super::safe_join(target, relative)?)?;
    let mut files = Vec::new();
    super::super::collect_files(source, &path, &mut files)?;
    for file in files {
        copy_file(
            source,
            target,
            &file.strip_prefix(source).unwrap().to_string_lossy(),
        )?;
    }
    Ok(())
}

// 在本机快照上合并，完整校验成功后才交给既有目录替换流程。
pub(super) fn merge(target: &Path, source: &Path, scope: &Scope, mode: ImportMode) -> Result<()> {
    let source_db = Connection::open_with_flags(
        source.join("qcclip.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut target_db = Connection::open(target.join("qcclip.sqlite3"))?;
    target_db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF;")?;
    let tx = target_db.transaction()?;
    let mut wanted = HashSet::new();
    for (table, enabled) in [
        ("clipboard_history", scope.history),
        ("favorites", scope.favorites),
    ] {
        if enabled {
            let mut statement = source_db.prepare(&format!("SELECT item_id FROM {table}"))?;
            wanted.extend(
                statement
                    .query_map([], |r| r.get::<_, i64>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            );
            if mode == ImportMode::Replace {
                tx.execute(&format!("DELETE FROM {table}"), [])?;
            }
        }
    }
    let mut blob_map = HashMap::new();
    let mut icon_map = HashMap::new();
    let mut item_map = HashMap::new();
    let mut tag_map = HashMap::new();
    for table in ["binary_blobs", "application_icon_assets"] {
        let map = if table == "binary_blobs" {
            &mut blob_map
        } else {
            &mut icon_map
        };
        for mut row in rows(&source_db, table)? {
            let id = integer(&row, "id")?;
            let referenced = if table == "binary_blobs" {
                rows_for_item_ids(&source_db, "clipboard_formats", "blob_id", id, &wanted)?
            } else if scope.history {
                source_db.query_row("SELECT EXISTS(SELECT 1 FROM clipboard_history WHERE last_source_app_icon_id=?1)", [id], |r| r.get(0))?
            } else {
                false
            };
            if !referenced {
                continue;
            }
            let existing: Option<i64> = tx
                .query_row(
                    &format!("SELECT id FROM {table} WHERE sha256=?1"),
                    [&row["sha256"]],
                    |r| r.get(0),
                )
                .optional()?;
            let local = if let Some(local) = existing {
                local
            } else {
                let path = text(&row, "relative_path")?;
                if target.join(&path).exists()
                    && super::super::hash_file(&target.join(&path))?
                        != super::super::hash_file(&source.join(&path))?
                {
                    return invalid("导入资源与本机同路径资源冲突");
                }
                copy_file(source, target, &path)?;
                insert(&tx, table, &mut row)?
            };
            map.insert(id, local);
        }
    }
    let mut item_ids: Vec<_> = wanted.iter().copied().collect();
    item_ids.sort_unstable();
    for id in item_ids {
        let mut row = query_rows(
            &source_db,
            "SELECT * FROM clipboard_items WHERE id=?1",
            [id],
        )?
        .pop()
        .ok_or_else(|| super::Error::Message("数据包内容引用不存在".into()))?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM clipboard_items WHERE hash_version=?1 AND content_hash=?2",
                params![row["hash_version"], row["content_hash"]],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(local) = existing {
            item_map.insert(id, local);
            continue;
        }
        let state = row["capture_state"].clone();
        row.insert("capture_state".into(), Value::Text("pending".into()));
        row.insert("parent_item_id".into(), Value::Null);
        row.insert("origin_kind".into(), Value::Text("imported".into()));
        let local = insert(&tx, "clipboard_items", &mut row)?;
        for mut format in rows_for_item(&source_db, "clipboard_formats", id)? {
            format.insert("item_id".into(), Value::Integer(local));
            remap(&mut format, "blob_id", &blob_map)?;
            insert(&tx, "clipboard_formats", &mut format)?;
        }
        tx.execute(
            "UPDATE clipboard_items SET capture_state=?1 WHERE id=?2",
            params![state, local],
        )?;
        item_map.insert(id, local);
    }
    for mut row in rows(&source_db, "tags")? {
        let id = integer(&row, "id")?;
        if !rows_for_item_ids(&source_db, "clipboard_item_tags", "tag_id", id, &wanted)? {
            continue;
        }
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM tags WHERE normalized_name=?1",
                [&row["normalized_name"]],
                |r| r.get(0),
            )
            .optional()?;
        let local = if let Some(local) = existing {
            local
        } else {
            let order: i64 = tx.query_row(
                "SELECT COALESCE(MAX(sort_order), 0) + 1000000 FROM tags",
                [],
                |r| r.get(0),
            )?;
            row.insert("sort_order".into(), Value::Integer(order));
            insert(&tx, "tags", &mut row)?
        };
        tag_map.insert(id, local);
    }
    for (id, local) in &item_map {
        if mode == ImportMode::Replace {
            tx.execute("DELETE FROM clipboard_item_tags WHERE item_id=?1", [local])?;
            tx.execute("DELETE FROM clipboard_item_notes WHERE item_id=?1", [local])?;
        }
        for mut row in rows_for_item(&source_db, "clipboard_item_tags", *id)? {
            row.insert("item_id".into(), Value::Integer(*local));
            remap(&mut row, "tag_id", &tag_map)?;
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM clipboard_item_tags WHERE item_id=?1 AND tag_id=?2)",
                params![local, row["tag_id"]],
                |r| r.get(0),
            )?;
            if !exists {
                insert(&tx, "clipboard_item_tags", &mut row)?;
            }
        }
        for mut row in rows_for_item(&source_db, "clipboard_item_notes", *id)? {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM clipboard_item_notes WHERE item_id=?1)",
                [local],
                |r| r.get(0),
            )?;
            if !exists {
                row.insert("item_id".into(), Value::Integer(*local));
                insert(&tx, "clipboard_item_notes", &mut row)?;
            }
        }
    }
    for (table, enabled) in [
        ("clipboard_history", scope.history),
        ("favorites", scope.favorites),
    ] {
        if !enabled {
            continue;
        }
        let mut relations = rows(&source_db, table)?;
        relations.sort_by_key(|row| integer(row, "sort_order").unwrap_or(0));
        for mut row in relations {
            remap(&mut row, "item_id", &item_map)?;
            let exists: bool = tx.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE item_id=?1)"),
                [&row["item_id"]],
                |r| r.get(0),
            )?;
            if exists {
                continue;
            }
            let order: i64 = tx.query_row(
                &format!(
                    "SELECT COALESCE(MAX(sort_order),0) + 1000000 FROM {table} WHERE is_pinned=?1"
                ),
                [&row["is_pinned"]],
                |r| r.get(0),
            )?;
            row.insert("sort_order".into(), Value::Integer(order));
            if table == "clipboard_history" {
                remap(&mut row, "last_source_app_icon_id", &icon_map)?;
            }
            insert(&tx, table, &mut row)?;
        }
    }
    for mut row in rows(&source_db, "sync_sources")? {
        let id = integer(&row, "item_id")?;
        if let Some(local) = item_map.get(&id)
            && !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sync_sources WHERE item_id=?1)",
                [local],
                |r| r.get::<_, bool>(0),
            )?
        {
            row.insert("item_id".into(), Value::Integer(*local));
            insert(&tx, "sync_sources", &mut row)?;
        }
    }
    tx.commit()?;
    // 受管理的同步文件保留包内随机目录；普通路径引用不读取文件内容。
    let source_store = RawBlobStore::new(source).map_err(super::DataManagementError::Storage)?;
    let canonical_source = dunce::canonicalize(source)?;
    for id in &wanted {
        if let Some(paths) =
            crate::services::sync::managed_file_list(&source_db, &source_store, source, *id)?
        {
            for path in paths {
                let relative = path
                    .strip_prefix(&canonical_source)
                    .map_err(|_| super::Error::Message("受管理文件路径无效".into()))?
                    .to_string_lossy();
                let destination = super::super::safe_join(target, &relative)?;
                if destination.exists()
                    && super::super::hash_file(&destination)? != super::super::hash_file(&path)?
                {
                    return invalid("导入文件与本机同路径文件冲突");
                }
                copy_file(source, target, &relative)?;
            }
        }
    }
    if scope.settings {
        if source.join("config.json").is_file() {
            copy_file(source, target, "config.json")?;
        }
        copy_directory(source, target, "sound")?;
    }
    if scope.gallery {
        merge_gallery(source, target, mode)?;
    }
    target_db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(())
}

type Row = std::collections::BTreeMap<String, Value>;
fn rows(connection: &Connection, table: &str) -> Result<Vec<Row>> {
    query_rows(connection, &format!("SELECT * FROM {table}"), [])
}
fn rows_for_item(connection: &Connection, table: &str, id: i64) -> Result<Vec<Row>> {
    query_rows(
        connection,
        &format!("SELECT * FROM {table} WHERE item_id=?1"),
        [id],
    )
}
fn rows_for_item_ids(
    connection: &Connection,
    table: &str,
    column: &str,
    id: i64,
    wanted: &HashSet<i64>,
) -> Result<bool> {
    let mut statement =
        connection.prepare(&format!("SELECT item_id FROM {table} WHERE {column}=?1"))?;
    for item in statement.query_map([id], |r| r.get::<_, i64>(0))? {
        if wanted.contains(&item?) {
            return Ok(true);
        }
    }
    Ok(false)
}
fn query_rows(
    connection: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<Row>> {
    let mut statement = connection.prepare(sql)?;
    let names: Vec<_> = statement
        .column_names()
        .iter()
        .map(|n| n.to_string())
        .collect();
    Ok(statement
        .query_map(params, |row| {
            names
                .iter()
                .enumerate()
                .map(|(i, n)| Ok((n.clone(), row.get(i)?)))
                .collect::<rusqlite::Result<Row>>()
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}
fn insert(connection: &Connection, table: &str, row: &mut Row) -> Result<i64> {
    row.remove("id");
    if row.contains_key("uuid") {
        row.insert("uuid".into(), Value::Text(uuid::Uuid::new_v4().to_string()));
    }
    // 列名只允许来自当前程序结构，不执行包内的动态 SQL。
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let allowed: HashSet<String> = statement
        .query_map([], |r| r.get(1))?
        .collect::<rusqlite::Result<_>>()?;
    row.retain(|key, _| allowed.contains(key));
    if row
        .keys()
        .any(|key| !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
    {
        return invalid("数据包字段名称无效");
    }
    let columns = row.keys().cloned().collect::<Vec<_>>().join(",");
    let placeholders = vec!["?"; row.len()].join(",");
    connection.execute(
        &format!("INSERT INTO {table} ({columns}) VALUES ({placeholders})"),
        rusqlite::params_from_iter(row.values()),
    )?;
    Ok(connection.last_insert_rowid())
}
fn integer(row: &Row, key: &str) -> Result<i64> {
    match row.get(key) {
        Some(Value::Integer(v)) => Ok(*v),
        _ => invalid("数据包整数字段无效"),
    }
}
fn text(row: &Row, key: &str) -> Result<String> {
    match row.get(key) {
        Some(Value::Text(v)) => Ok(v.clone()),
        _ => invalid("数据包文本字段无效"),
    }
}
fn remap(row: &mut Row, key: &str, map: &HashMap<i64, i64>) -> Result<()> {
    if row.get(key).is_none_or(|v| *v == Value::Null) {
        return Ok(());
    }
    let id = integer(row, key)?;
    let local = map
        .get(&id)
        .ok_or_else(|| super::Error::Message("数据包关系引用不存在".into()))?;
    row.insert(key.into(), Value::Integer(*local));
    Ok(())
}

fn merge_gallery(source: &Path, target: &Path, mode: ImportMode) -> Result<()> {
    let source_service =
        crate::services::image_library::ImageLibraryService::new(source.join("image_library"));
    let target_service =
        crate::services::image_library::ImageLibraryService::new(target.join("image_library"));
    if mode == ImportMode::Replace {
        let dir = target.join("image_library");
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::create_dir_all(&dir)?;
        copy_directory(source, target, "image_library")?;
        for group in source_service
            .load_groups()
            .map_err(super::Error::Message)?
        {
            fs::create_dir_all(dir.join(group.name))?;
        }
        return Ok(());
    }
    let mut groups = target_service
        .load_groups()
        .map_err(super::Error::Message)?;
    for mut group in source_service
        .load_groups()
        .map_err(super::Error::Message)?
    {
        if let Some(existing) = groups
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case(&group.name))
        {
            let source_dir = source.join("image_library").join(&group.name);
            let target_dir = target.join("image_library").join(&existing.name);
            fs::create_dir_all(&target_dir)?;
            if source_dir.is_dir() {
                for entry in fs::read_dir(&source_dir)? {
                    let entry = entry?;
                    if !entry.file_type()?.is_file() {
                        continue;
                    }
                    let mut destination = target_dir.join(entry.file_name());
                    if destination.exists() {
                        if super::super::hash_file(&destination)?
                            == super::super::hash_file(&entry.path())?
                        {
                            continue;
                        }
                        let extension = entry
                            .path()
                            .extension()
                            .and_then(|s| s.to_str())
                            .unwrap_or("png")
                            .to_owned();
                        let hash = super::super::hash_file(&entry.path())?;
                        let name = hash
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>();
                        destination = target_dir.join(format!("import-{name}.{extension}"));
                        if destination.exists() {
                            if super::super::hash_file(&destination)? == hash {
                                continue;
                            }
                            return invalid("图库合并文件发生哈希路径冲突");
                        }
                    }
                    fs::copy(entry.path(), destination)?;
                }
            }
            continue;
        }
        let old_id = group.id.clone();
        if groups.iter().any(|g| g.id == group.id) {
            group.id = uuid::Uuid::new_v4().to_string();
        }
        copy_directory(source, target, &format!("image_library/{}", group.name))?;
        fs::create_dir_all(target.join("image_library").join(&group.name))?;
        // 组图标使用 ID 命名，合并后与新 ID 一致。
        let icon = format!("image_library/.groups_icons/{}.png", group.id);
        let source_icon = source.join(format!("image_library/.groups_icons/{old_id}.png"));
        if source_icon.is_file() {
            fs::create_dir_all(target.join("image_library/.groups_icons"))?;
            fs::copy(source_icon, target.join(&icon))?;
        }
        groups.push(group);
    }
    fs::create_dir_all(target.join("image_library"))?;
    fs::write(
        target.join("image_library/groups.json"),
        serde_json::to_vec_pretty(&groups)?,
    )?;
    Ok(())
}
