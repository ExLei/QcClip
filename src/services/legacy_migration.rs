use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Cursor, Read},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use uuid::Uuid;

use super::{
    application_icon::ApplicationIconStore,
    capture::{
        ClipboardPreview, HashedClipboard, generate_preview, hash_clipboard, select_default_preview,
    },
    data_management::DataManagementService,
    database::{Database, LIST_ORDER_GAP},
    storage::RawBlobStore,
};
use crate::domain::capture::{
    CapturedClipboard, FormatCaptureState, FormatKind, RawFormat, SourceApplication,
    normalize_text_bytes,
};

mod staged;
use staged::StagedMigration;

const MAX_ARCHIVE_ENTRIES: usize = 1_000_000;
const MAX_FORMAT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_ICON_BYTES: u64 = 1024 * 1024;
const LEDGER_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS legacy_import_records (
    source_key TEXT PRIMARY KEY NOT NULL,
    item_id INTEGER REFERENCES clipboard_items(id) ON DELETE SET NULL,
    imported_at INTEGER NOT NULL
);";

#[derive(Debug, thiserror::Error)]
pub(crate) enum MigrationError {
    #[error("旧版数据无效：{0}")]
    Invalid(String),
    #[error("迁移文件操作失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("迁移暂存数据读写失败：{0}")]
    Json(#[from] serde_json::Error),
    #[error("迁移数据库操作失败：{0}")]
    Sql(#[from] rusqlite::Error),
    #[error("迁移压缩包读取失败：{0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("迁移内容转换失败：{0}")]
    Capture(#[from] super::capture::CaptureError),
    #[error("迁移数据库校验失败：{0}")]
    Database(#[from] super::database::DatabaseError),
    #[error("迁移二进制存储失败：{0}")]
    Storage(#[from] super::storage::RawBlobError),
    #[error("迁移前备份失败：{0}")]
    Backup(#[from] super::data_management::DataManagementError),
    #[error("迁移未提交，数据库事务已回滚：{cause}。迁移前备份：{backup}")]
    RolledBack { cause: String, backup: String },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MigrationOptions {
    pub(crate) history: bool,
    pub(crate) favorites: bool,
}

impl Default for MigrationOptions {
    fn default() -> Self {
        Self {
            history: false,
            favorites: true,
        }
    }
}

#[derive(Default, Debug)]
pub(crate) struct MigrationPreview {
    pub(crate) history: usize,
    pub(crate) favorites: usize,
    pub(crate) groups: usize,
    pub(crate) duplicate_contents: usize,
    pub(crate) already_imported: usize,
    pub(crate) skipped: usize,
    pub(crate) warnings: Vec<String>,
    pub(crate) warning_count: usize,
}

impl MigrationPreview {
    fn warn(&mut self, message: String) {
        self.warning_count += 1;
        if self.warnings.len() < 100 {
            self.warnings.push(message);
        }
    }
}

pub(crate) struct PreparedMigration {
    pub(crate) source: PathBuf,
    pub(crate) preview: MigrationPreview,
    groups: Vec<LegacyGroup>,
    staged_path: PathBuf,
    options: MigrationOptions,
    _scratch: TempDir,
}

impl PreparedMigration {
    pub(crate) fn selected_count(&self, options: MigrationOptions) -> usize {
        usize::from(options.history) * self.preview.history
            + usize::from(options.favorites) * self.preview.favorites
    }
}

struct LegacyGroup {
    name: String,
    color: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct LegacyRecord {
    kind: String,
    id: String,
    content: String,
    html: Option<String>,
    content_type: String,
    image_id: Option<String>,
    group: String,
    title: String,
    pinned: bool,
    paste_count: i64,
    source_app: Option<String>,
    source_icon_hash: Option<String>,
    created_at: i64,
    updated_at: i64,
}

struct PreparedRecord {
    legacy: LegacyRecord,
    captured: CapturedClipboard,
    preview: ClipboardPreview,
    hashed: HashedClipboard,
    source_key: String,
    icon: Option<LegacyIcon>,
}

struct LegacyIcon {
    bytes: Vec<u8>,
    sha256: [u8; 32],
    relative_path: String,
    width: u32,
    height: u32,
}

#[derive(Default, Debug)]
pub(crate) struct MigrationReport {
    pub(crate) added_contents: usize,
    pub(crate) merged_contents: usize,
    pub(crate) added_history: usize,
    pub(crate) added_favorites: usize,
    pub(crate) added_tags: usize,
    pub(crate) imported_icons: usize,
    pub(crate) skipped: usize,
    pub(crate) backup: PathBuf,
}

pub(crate) fn detect_source() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    let root = base.data_local_dir().join("quickclipboard");
    resolve_source_root(&root)
        .ok()
        .filter(|root| root.join("quickclipboard.db").is_file())
}

fn resolve_source_root(selected: &Path) -> Result<PathBuf, MigrationError> {
    let root = if selected.join("quickclipboard.db").is_file() {
        selected.to_path_buf()
    } else if selected.join("data/settings.json").is_file()
        || selected.join("data/quickclipboard.db").is_file()
    {
        selected.join("data")
    } else {
        selected.to_path_buf()
    };
    if let Ok(text) = fs::read_to_string(root.join("settings.json")) {
        let settings: serde_json::Value = serde_json::from_str(&text)
            .map_err(|error| MigrationError::Invalid(format!("旧版设置无法解析：{error}")))?;
        if settings
            .get("use_custom_storage")
            .and_then(|value| value.as_bool())
            == Some(true)
        {
            let path = settings
                .get("custom_storage_path")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    MigrationError::Invalid("旧版已启用自定义存储，但没有设置有效路径".to_owned())
                })?;
            let path = PathBuf::from(path);
            if path.is_absolute() && path.join("quickclipboard.db").is_file() {
                return Ok(path);
            }
            return Err(MigrationError::Invalid(
                "旧版设置中的自定义数据目录不存在或无效，请直接选择实际数据目录".to_owned(),
            ));
        }
    }
    if root.join("quickclipboard.db").is_file() {
        return Ok(root);
    }
    Err(MigrationError::Invalid(
        "目录中没有找到 quickclipboard.db，请选择旧版数据目录或导出的 ZIP".to_owned(),
    ))
}

pub(crate) fn prepare(
    source: &Path,
    destination: &Path,
    options: MigrationOptions,
) -> Result<PreparedMigration, MigrationError> {
    if !options.history && !options.favorites {
        return Err(MigrationError::Invalid(
            "请选择需要迁移的数据范围".to_owned(),
        ));
    }
    // 系统临时目录可能位于不支持规范化路径的卷（如内存盘），会导致 os error 1；
    // 放在当前数据目录下，该目录启动时已通过规范化校验。
    let scratch = tempfile::Builder::new()
        .prefix(".legacy-migration-")
        .tempdir_in(destination)?;
    let mut preview = MigrationPreview::default();
    let root = if source.is_file() {
        let extracted = scratch.path().join("source");
        fs::create_dir(&extracted)?;
        extract_zip(source, &extracted, options, &mut preview)?;
        if !extracted.join("quickclipboard.db").is_file() {
            return Err(MigrationError::Invalid(
                "ZIP 根目录中没有 quickclipboard.db".to_owned(),
            ));
        }
        extracted
    } else {
        resolve_source_root(source)?
    };
    let old_path = dunce::canonicalize(root.join("quickclipboard.db"))?;
    if old_path == dunce::canonicalize(destination.join("qcclip.sqlite3"))? {
        return Err(MigrationError::Invalid(
            "源数据库不能是当前数据库".to_owned(),
        ));
    }
    let old = Connection::open_with_flags(&old_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    old.busy_timeout(Duration::from_secs(5))?;
    let mut snapshot = Connection::open(scratch.path().join("snapshot.sqlite3"))?;
    snapshot.execute_batch("PRAGMA cache_size = -4096; PRAGMA temp_store = FILE;")?;
    // SQLite 在线备份同时读取主库和 WAL，不修改旧版的结构、数据或版本号。
    rusqlite::backup::Backup::new(&old, &mut snapshot)?.run_to_completion(
        128,
        Duration::from_millis(10),
        None,
    )?;
    drop(old);
    if snapshot.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))? != "ok" {
        return Err(MigrationError::Invalid(
            "旧版数据库完整性检查未通过".to_owned(),
        ));
    }
    if root.join("image_library").is_dir() {
        preview.warn("本期暂不迁移旧版图库，请保留旧版 image_library 目录".to_owned());
    }
    if root.join("settings.json").is_file() {
        preview.warn("本期暂不迁移旧版设置，当前版本的设置将继续保留".to_owned());
    }
    let groups = if options.favorites {
        read_groups(&snapshot, &mut preview)?
    } else {
        Vec::new()
    };
    check_group_conflicts(&groups, destination, &mut preview)?;
    let has_raw = table_exists(&snapshot, "clipboard_data")?;
    let destination_connection = Connection::open_with_flags(
        destination.join("qcclip.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let has_ledger = table_exists(&destination_connection, "legacy_import_records")?;
    let staged_path = scratch.path().join("prepared.sqlite3");
    let staged = StagedMigration::create(&staged_path)?;
    let mut icons = HashMap::new();
    read_records(&snapshot, options, |legacy| {
        let label = format!(
            "{} {}",
            if legacy.kind == "clipboard" {
                "历史"
            } else {
                "收藏"
            },
            legacy.id
        );
        let captured = match (|| {
            let raw = if has_raw {
                read_raw_formats(&snapshot, &legacy)?
            } else {
                Vec::new()
            };
            let captured = convert_record(&root, &legacy, raw, &mut preview)?;
            if captured.formats.iter().any(|format| {
                format
                    .bytes
                    .as_ref()
                    .is_some_and(|bytes| bytes.len() as u64 > MAX_FORMAT_BYTES)
            }) {
                return Err(MigrationError::Invalid(
                    "重建后的单个原始格式超过 64 MB".to_owned(),
                ));
            }
            Ok::<_, MigrationError>(captured)
        })() {
            Ok(captured) => captured,
            Err(MigrationError::Invalid(error)) => {
                preview.skipped += 1;
                preview.warn(format!("{label}：{error}，已跳过"));
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if captured.formats.is_empty() {
            preview.skipped += 1;
            preview.warn(format!("{label}：没有可恢复的内容，已跳过"));
            return Ok(());
        }
        let hashed = hash_clipboard(&captured)?;
        let format_preview = generate_preview(&captured)?;
        if select_default_preview(&captured, &format_preview.formats).is_none() {
            preview.skipped += 1;
            preview.warn(format!("{label}：没有可显示的格式，已跳过"));
            return Ok(());
        }
        let icon_id = if legacy.kind == "clipboard" {
            if let Some(hash) = legacy
                .source_icon_hash
                .as_deref()
                .filter(|hash| !hash.is_empty())
            {
                if let Some(id) = icons.get(hash) {
                    *id
                } else {
                    let id = match read_legacy_icon(&root, hash) {
                        Ok(icon) => Some(staged.save_icon(&icon)?),
                        Err(error) => {
                            preview.warn(format!(
                                "{label}：来源应用图标无法恢复：{error}，将保留应用名称"
                            ));
                            None
                        }
                    };
                    icons.insert(hash.to_owned(), id);
                    id
                }
            } else {
                None
            }
        } else {
            None
        };
        let source_key = record_key(&legacy, &hashed);
        if has_ledger
            && destination_connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM legacy_import_records WHERE source_key = ?1)",
                [&source_key],
                |row| row.get::<_, bool>(0),
            )?
        {
            preview.already_imported += 1;
        }
        if staged.save_record(&legacy, &captured, &hashed, &source_key, icon_id)? {
            preview.duplicate_contents += 1;
        }
        if legacy.kind == "clipboard" {
            preview.history += 1;
        } else {
            preview.favorites += 1;
        }
        Ok(())
    })?;
    staged.finish()?;
    preview.groups = groups.len();
    Ok(PreparedMigration {
        source: source.to_path_buf(),
        preview,
        groups,
        staged_path,
        options,
        _scratch: scratch,
    })
}

fn read_legacy_icon(root: &Path, hash: &str) -> Result<LegacyIcon, MigrationError> {
    use image::ImageDecoder;

    if hash.len() != 16 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(MigrationError::Invalid("旧版图标哈希无效".to_owned()));
    }
    let root = dunce::canonicalize(root)?;
    let path = dunce::canonicalize(root.join("app_icons").join(format!("{hash}.png")))?;
    if !path.starts_with(root.join("app_icons")) {
        return Err(MigrationError::Invalid(
            "旧版图标路径超出缓存目录".to_owned(),
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_ICON_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_ICON_BYTES {
        return Err(MigrationError::Invalid("旧版图标超过 1 MB".to_owned()));
    }
    let sha256: [u8; 32] = Sha256::digest(&bytes).into();
    let hex: String = sha256.iter().map(|byte| format!("{byte:02x}")).collect();
    if !hex[..16].eq_ignore_ascii_case(hash) {
        return Err(MigrationError::Invalid(
            "旧版图标内容与哈希不一致".to_owned(),
        ));
    }
    let decoder = image::codecs::png::PngDecoder::new(Cursor::new(&bytes))
        .map_err(|error| MigrationError::Invalid(format!("旧版图标不是有效 PNG：{error}")))?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || width > 256 || height > 256 {
        return Err(MigrationError::Invalid(
            "旧版图标尺寸超出支持范围".to_owned(),
        ));
    }
    image::DynamicImage::from_decoder(decoder)
        .map_err(|error| MigrationError::Invalid(format!("旧版图标无法解码：{error}")))?;
    Ok(LegacyIcon {
        bytes,
        sha256,
        relative_path: format!("data/app-icons/{hex}.png"),
        width,
        height,
    })
}

fn extract_zip(
    source: &Path,
    root: &Path,
    options: MigrationOptions,
    preview: &mut MigrationPreview,
) -> Result<(), MigrationError> {
    let mut archive = zip::ZipArchive::new(File::open(source)?)?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(MigrationError::Invalid(
            "ZIP 条目数量超过一百万个".to_owned(),
        ));
    }
    let mut paths = HashSet::new();
    let mut total = 0_u64;
    let mut has_gallery = false;
    let mut has_settings = false;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().replace('\\', "/");
        let path = Path::new(&name);
        if name.contains(':')
            || name.contains('\0')
            || path.is_absolute()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            || !paths.insert(name.to_lowercase())
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(MigrationError::Invalid(format!(
                "ZIP 包含不安全或重复路径：{name}"
            )));
        }
        let normalized = name.to_lowercase();
        has_gallery |= normalized == "image_library" || normalized.starts_with("image_library/");
        has_settings |= normalized == "settings.json";
        let needed = matches!(
            normalized.as_str(),
            "quickclipboard.db" | "quickclipboard.db-wal"
        ) || normalized.starts_with("clipboard_images/")
            || options.history && normalized.starts_with("app_icons/");
        if !needed || entry.is_dir() {
            continue;
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| MigrationError::Invalid("ZIP 大小溢出".to_owned()))?;
        if total > MAX_ARCHIVE_BYTES {
            return Err(MigrationError::Invalid(
                "ZIP 所需文件的解压数据超过 16 GB".to_owned(),
            ));
        }
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let expected = entry.size();
        let copied = std::io::copy(
            &mut entry.by_ref().take(expected.saturating_add(1)),
            &mut File::create(target)?,
        )?;
        if copied != expected {
            return Err(MigrationError::Invalid(format!(
                "ZIP 文件长度不一致：{name}"
            )));
        }
    }
    if has_gallery {
        preview.warn("本期暂不迁移旧版图库，请保留旧版 image_library 目录".to_owned());
    }
    if has_settings {
        preview.warn("本期暂不迁移旧版设置，当前版本的设置将继续保留".to_owned());
    }
    Ok(())
}

fn table_exists(connection: &Connection, name: &str) -> Result<bool, MigrationError> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [name],
        |row| row.get(0),
    )?)
}

fn columns(connection: &Connection, table: &str) -> Result<HashSet<String>, MigrationError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    Ok(statement
        .query_map([], |row| row.get(1))?
        .collect::<Result<_, _>>()?)
}

fn read_groups(
    connection: &Connection,
    preview: &mut MigrationPreview,
) -> Result<Vec<LegacyGroup>, MigrationError> {
    if !table_exists(connection, "groups")? {
        return Ok(Vec::new());
    }
    let fields = columns(connection, "groups")?;
    if !fields.contains("name") {
        return Err(MigrationError::Invalid("旧版分组表缺少名称字段".to_owned()));
    }
    let color = if fields.contains("color") {
        "color"
    } else {
        "'#1677FF'"
    };
    let order = if fields.contains("order_index") {
        "order_index"
    } else {
        "name"
    };
    let mut statement = connection.prepare(&format!(
        "SELECT name, {color} FROM groups ORDER BY {order}, name"
    ))?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut result = Vec::new();
    for row in rows {
        let (name, color) = row?;
        if name.trim() == "全部" {
            continue;
        }
        if valid_tag_name(&name) {
            result.push(LegacyGroup {
                name: name.trim().to_owned(),
                color: legacy_color(&color),
            });
        } else {
            preview.warn(format!(
                "分组“{name}”名称为空或超过 100 个字符，不能转换为标签"
            ));
        }
    }
    Ok(result)
}

fn check_group_conflicts(
    groups: &[LegacyGroup],
    destination: &Path,
    preview: &mut MigrationPreview,
) -> Result<(), MigrationError> {
    let connection = Connection::open_with_flags(
        destination.join("qcclip.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut seen = HashSet::new();
    for group in groups {
        let normalized = group.name.to_lowercase();
        if !seen.insert(normalized.clone()) {
            preview.warn(format!(
                "分组“{}”与其他旧版分组规范化后同名，将合并为一个标签",
                group.name
            ));
        }
        let existing: Option<String> = connection
            .query_row(
                "SELECT color FROM tags WHERE normalized_name = ?1",
                [&normalized],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(color) = existing {
            preview.warn(format!(
                "分组“{}”将使用现有同名标签，并保留现有颜色 {color}",
                group.name
            ));
        }
    }
    Ok(())
}

fn read_records(
    connection: &Connection,
    options: MigrationOptions,
    mut visit: impl FnMut(LegacyRecord) -> Result<(), MigrationError>,
) -> Result<(), MigrationError> {
    for (table, kind) in [("clipboard", "clipboard"), ("favorites", "favorite")] {
        if kind == "clipboard" && !options.history || kind == "favorite" && !options.favorites {
            continue;
        }
        let fields = columns(connection, table)?;
        for field in ["id", "content", "content_type", "created_at", "updated_at"] {
            if !fields.contains(field) {
                return Err(MigrationError::Invalid(format!(
                    "旧版 {table} 表缺少 {field} 字段"
                )));
            }
        }
        let optional = |name: &str, fallback: &str| {
            if fields.contains(name) {
                name.to_owned()
            } else {
                fallback.to_owned()
            }
        };
        let sql = format!("SELECT CAST(id AS TEXT), content, {}, content_type, {}, {}, {}, {}, {}, {}, created_at, updated_at, {}
            FROM {table} ORDER BY {}, created_at, id",
            optional("html_content", "NULL"), optional("image_id", "NULL"),
            optional("group_name", "'全部'"), optional("title", "''"), optional("is_pinned", "0"),
            optional("paste_count", "0"), optional("source_app", "NULL"), optional("source_icon_hash", "NULL"),
            optional("item_order", "id"));
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map([], |row| {
            Ok(LegacyRecord {
                kind: kind.to_owned(),
                id: row.get(0)?,
                content: row.get(1)?,
                html: row.get(2)?,
                content_type: row.get(3)?,
                image_id: row.get(4)?,
                group: row.get(5)?,
                title: row.get(6)?,
                pinned: row.get::<_, i64>(7)? != 0,
                paste_count: row.get::<_, i64>(8)?.max(0),
                source_app: row.get(9)?,
                created_at: seconds_to_millis(row.get(10)?)?,
                updated_at: seconds_to_millis(row.get(11)?)?,
                source_icon_hash: row.get(12)?,
            })
        })?;
        for row in rows {
            visit(row?)?;
        }
    }
    Ok(())
}

fn seconds_to_millis(seconds: i64) -> rusqlite::Result<i64> {
    seconds
        .checked_mul(1000)
        .filter(|value| *value >= 0)
        .ok_or(rusqlite::Error::IntegralValueOutOfRange(0, seconds))
}

fn read_raw_formats(
    connection: &Connection,
    record: &LegacyRecord,
) -> Result<Vec<(String, Vec<u8>, bool)>, MigrationError> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM (SELECT 1 FROM clipboard_data
        WHERE target_kind = ?1 AND target_id = ?2 LIMIT 257)",
        params![record.kind, record.id],
        |row| row.get(0),
    )?;
    if count > 256 {
        return Err(MigrationError::Invalid(
            "单条记录的原始格式数量超过 256 个".to_owned(),
        ));
    }
    let too_large: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM clipboard_data WHERE target_kind = ?1 AND target_id = ?2 AND length(raw_data) > ?3)",
        params![record.kind, record.id, MAX_FORMAT_BYTES as i64], |row| row.get(0),
    )?;
    if too_large {
        return Err(MigrationError::Invalid(format!(
            "记录 {} 的单个原始格式超过 64 MB",
            record.id
        )));
    }
    let mut statement = connection.prepare(
        "SELECT format_name, raw_data, is_primary FROM clipboard_data
        WHERE target_kind = ?1 AND target_id = ?2 ORDER BY format_order, id LIMIT 257",
    )?;
    let rows = statement.query_map(params![record.kind, record.id], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? != 0))
    })?;
    let formats: Vec<_> = rows.collect::<Result<_, _>>()?;
    Ok(formats)
}

fn convert_record(
    root: &Path,
    record: &LegacyRecord,
    raw: Vec<(String, Vec<u8>, bool)>,
    preview: &mut MigrationPreview,
) -> Result<CapturedClipboard, MigrationError> {
    let mut formats = Vec::new();
    let mut primary = None;
    let mut image_paths = Vec::new();
    for (name, bytes, is_primary) in raw {
        if name == "__QC_IMAGE_PNG_PATH__" {
            if let Ok(path) = String::from_utf8(bytes) {
                image_paths.push(path);
            }
            continue;
        }
        let (key, kind, native_id, encoding) = match name.as_str() {
            "CF_UNICODETEXT" => (name.clone(), FormatKind::Text, Some(13), Some("utf-16le")),
            "CF_TEXT" => (name.clone(), FormatKind::Text, Some(1), None),
            "CF_HDROP" => (name.clone(), FormatKind::FileList, Some(15), None),
            "HTML Format" => (
                "registered:HTML Format".to_owned(),
                FormatKind::Html,
                None,
                Some("utf-8"),
            ),
            "Rich Text Format" => (
                "registered:Rich Text Format".to_owned(),
                FormatKind::Rtf,
                None,
                None,
            ),
            _ => {
                preview.warn(format!(
                    "记录 {}：未识别的旧版格式“{name}”已跳过",
                    record.id
                ));
                continue;
            }
        };
        if bytes.is_empty() || name == "CF_UNICODETEXT" && bytes.len() % 2 != 0 {
            preview.warn(format!(
                "记录 {}：格式“{name}”为空或编码不完整，将尝试重建",
                record.id
            ));
            continue;
        }
        let bytes = normalize_text_bytes("windows", &key, &bytes).to_vec();
        let mut format = RawFormat::captured(
            "windows",
            &key,
            name,
            kind,
            bytes,
            formats.len() as u32,
            true,
        );
        format.native_format_id = native_id;
        format.encoding = encoding.map(str::to_owned);
        if is_primary {
            primary = Some(("windows".to_owned(), key));
        }
        formats.push(format);
    }
    let is_files = record.content.starts_with("files:");
    let mut cache_image_only = false;
    if is_files {
        let data: serde_json::Value =
            serde_json::from_str(record.content.strip_prefix("files:").unwrap_or_default())
                .map_err(|error| {
                    MigrationError::Invalid(format!("记录 {} 的文件列表无效：{error}", record.id))
                })?;
        let paths: Vec<String> = data
            .get("files")
            .and_then(|files| files.as_array())
            .into_iter()
            .flatten()
            .filter_map(|file| {
                file.get("path")
                    .and_then(|path| path.as_str())
                    .map(str::to_owned)
            })
            .collect();
        cache_image_only = record.content_type == "image"
            && !paths.is_empty()
            && paths.iter().all(|path| path.contains("clipboard_images"));
        for path in &paths {
            if !Path::new(path).exists() {
                preview.warn(format!("记录 {}：文件路径已失效：{path}", record.id));
            }
        }
        if !cache_image_only
            && !formats
                .iter()
                .any(|format| format.format_kind == FormatKind::FileList)
            && !paths.is_empty()
        {
            formats.push(generated_format(
                "CF_HDROP",
                FormatKind::FileList,
                encode_hdrop(&paths),
                formats.len(),
            ));
        }
        image_paths.extend(
            paths
                .into_iter()
                .filter(|path| path.contains("clipboard_images")),
        );
    }
    if let Some(ids) = &record.image_id {
        image_paths.extend(
            ids.split(',')
                .filter(|id| !id.is_empty())
                .map(|id| format!("clipboard_images/{id}.png")),
        );
    }
    let root_canonical = dunce::canonicalize(root)?;
    let mut loaded_image = false;
    for image_path in image_paths {
        let requested = if Path::new(&image_path).is_absolute() {
            PathBuf::from(&image_path)
        } else {
            root.join(&image_path)
        };
        let path = match dunce::canonicalize(requested) {
            Ok(path) if path.starts_with(&root_canonical) => path,
            _ => continue,
        };
        if fs::metadata(&path)?.len() > MAX_FORMAT_BYTES {
            preview.warn(format!(
                "记录 {}：缓存图片超过 64 MB，已跳过图片",
                record.id
            ));
            continue;
        }
        let bytes = fs::read(&path)?;
        if super::image_preview::probe_image_dimensions(&bytes, "registered:PNG").is_none() {
            continue;
        }
        formats.push(generated_format(
            "registered:PNG",
            FormatKind::Image,
            bytes,
            formats.len(),
        ));
        loaded_image = true;
        break;
    }
    if record.image_id.is_some() && !loaded_image {
        preview.warn(format!(
            "记录 {}：旧版缓存图片缺失或无效，仅迁移其他可恢复格式",
            record.id
        ));
    }
    if cache_image_only {
        formats.retain(|format| format.format_kind != FormatKind::FileList);
    }
    let image_only = record.content_type == "image" && loaded_image;
    if !is_files
        && !record.content.is_empty()
        && !record.content.starts_with("image:")
        && !formats
            .iter()
            .any(|format| format.format_kind == FormatKind::Text)
        && !image_only
    {
        formats.push(generated_format(
            "CF_UNICODETEXT",
            FormatKind::Text,
            encode_text(&record.content),
            formats.len(),
        ));
    }
    if !formats
        .iter()
        .any(|format| format.format_kind == FormatKind::Html)
        && let Some(html) = record.html.as_deref().filter(|html| !html.is_empty())
    {
        formats.push(generated_format(
            "registered:HTML Format",
            FormatKind::Html,
            encode_html(html),
            formats.len(),
        ));
    }
    if record
        .html
        .as_deref()
        .is_some_and(|html| html.contains("clipboard_images") || html.contains("file://"))
    {
        preview.warn(format!(
            "记录 {}：HTML 引用旧版本地资源，富文本中的图片可能仍依赖旧目录",
            record.id
        ));
    }
    if record.kind == "favorite" && record.group.trim() != "全部" && !valid_tag_name(&record.group)
    {
        preview.warn(format!(
            "收藏 {}：分组名称无法转换为标签，收藏内容仍会迁移",
            record.id
        ));
    }
    Ok(CapturedClipboard {
        formats,
        primary_format_identity: primary,
        preview_candidate: Some(record.content.clone()),
        source_application: SourceApplication {
            name: record.source_app.clone(),
            path: None,
        },
    })
}

fn generated_format(key: &str, kind: FormatKind, bytes: Vec<u8>, order: usize) -> RawFormat {
    let name = key.strip_prefix("registered:").unwrap_or(key);
    let bytes = normalize_text_bytes("windows", key, &bytes).to_vec();
    let mut format = RawFormat::captured("windows", key, name, kind, bytes, order as u32, true);
    format.capture_state = FormatCaptureState::Generated;
    format.native_format_id = match key {
        "CF_UNICODETEXT" => Some(13),
        "CF_HDROP" => Some(15),
        _ => None,
    };
    format.encoding = match kind {
        FormatKind::Text => Some("utf-16le".to_owned()),
        FormatKind::Html => Some("utf-8".to_owned()),
        _ => None,
    };
    format
}

fn encode_text(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn encode_hdrop(paths: &[String]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in [20_u32, 0, 0, 0, 1] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for path in paths {
        bytes.extend_from_slice(&encode_text(path));
    }
    bytes.extend_from_slice(&[0, 0]);
    bytes
}

fn encode_html(html: &str) -> Vec<u8> {
    let prefix = "<html><body><!--StartFragment-->";
    let suffix = "<!--EndFragment--></body></html>";
    let header = "Version:1.0\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    format!("Version:1.0\r\nStartHTML:{:010}\r\nEndHTML:{:010}\r\nStartFragment:{:010}\r\nEndFragment:{:010}\r\n{prefix}{html}{suffix}",
        header.len(), header.len() + prefix.len() + html.len() + suffix.len(), header.len() + prefix.len(), header.len() + prefix.len() + html.len()).into_bytes()
}

fn valid_tag_name(name: &str) -> bool {
    !name.trim().is_empty() && name.trim().chars().count() <= 100
}

fn legacy_color(value: &str) -> String {
    let text = value.trim();
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return format!("#{}", hex.to_uppercase());
    }
    if hex.len() == 3 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return format!(
            "#{}",
            hex.chars()
                .flat_map(|ch| [ch, ch])
                .collect::<String>()
                .to_uppercase()
        );
    }
    let number = if let Some(hex) = text.strip_prefix('#').filter(|hex| hex.len() == 8) {
        u32::from_str_radix(hex, 16)
            .ok()
            .filter(|value| value >> 24 != 0)
    } else {
        text.parse::<i64>()
            .ok()
            .map(|value| value as u32)
            .filter(|value| *value != 0)
    };
    number
        .map(|value| format!("#{:06X}", value & 0x00ff_ffff))
        .unwrap_or_else(|| "#1677FF".to_owned())
}

fn record_key(record: &LegacyRecord, hashed: &HashedClipboard) -> String {
    let mut hasher = Sha256::new();
    for part in [
        record.kind.as_str(),
        record.id.as_str(),
        &record.created_at.to_string(),
    ] {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hasher.update(hashed.content_hash);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn migrate(
    prepared: &PreparedMigration,
    options: MigrationOptions,
    service: &DataManagementService,
    progress: &AtomicUsize,
) -> Result<MigrationReport, MigrationError> {
    if !options.history && !options.favorites {
        return Err(MigrationError::Invalid(
            "请选择需要迁移的数据范围".to_owned(),
        ));
    }
    if options.history && !prepared.options.history
        || options.favorites && !prepared.options.favorites
    {
        return Err(MigrationError::Invalid(
            "迁移范围已改变，请重新分析旧版数据".to_owned(),
        ));
    }
    super::data_runtime::with_exclusive_operation(|| {
        let root = service.data_root();
        let database = Database::open(root.join("qcclip.sqlite3"))?;
        let mut connection = database.connect_rewrite()?;
        let store = RawBlobStore::new(root)?;
        let parent = root
            .parent()
            .ok_or_else(|| MigrationError::Invalid("无法确定备份目录".to_owned()))?;
        let folder = root.file_name().unwrap_or_default().to_string_lossy();
        let backup = parent
            .join(format!("{folder}-migration-backups"))
            .join(format!("before-legacy-{}.qcclip-data", Uuid::new_v4()));
        service.export_data(&backup)?;
        let mut report = MigrationReport {
            backup,
            skipped: prepared.preview.skipped,
            ..Default::default()
        };
        let mut new_blobs = Vec::new();
        let result = (|| {
            let transaction =
                connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            transaction.execute_batch(LEDGER_SCHEMA)?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            let mut tags = HashMap::new();
            if options.favorites {
                for group in prepared.groups.iter().rev() {
                    let source_key = format!("group:{}", group.name.to_lowercase());
                    if transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM legacy_import_records WHERE source_key = ?1)",
                        [&source_key],
                        |row| row.get::<_, bool>(0),
                    )? {
                        continue;
                    }
                    let tag =
                        ensure_tag(&transaction, &group.name, &group.color, now, &mut report)?;
                    tags.insert(group.name.to_lowercase(), tag);
                    transaction.execute("INSERT INTO legacy_import_records (source_key, imported_at) VALUES (?1, ?2)", params![source_key, now])?;
                }
            }
            let staged = StagedMigration::open(&prepared.staged_path)?;
            let mut completed = 0;
            staged.for_each_record(options, |record| {
                completed += 1;
                let imported: Option<Option<i64>> = transaction.query_row(
                    "SELECT item_id FROM legacy_import_records WHERE source_key = ?1", [&record.source_key], |row| row.get(0)).optional()?;
                if let Some(item_id) = imported {
                    let filled = match item_id {
                        Some(item_id) => import_history_icon(&transaction, item_id, &record, root, now, &mut new_blobs, &mut report)?,
                        None => false,
                    };
                    if !filled { report.skipped += 1; }
                    progress.store(completed, Ordering::Relaxed); return Ok(());
                }
                let item_id = import_content(&transaction, &record, &store, root, now, &mut new_blobs, &mut report)?;
                if record.legacy.kind == "clipboard" {
                    import_history(&transaction, item_id, &record.legacy, &mut report)?;
                    import_history_icon(&transaction, item_id, &record, root, now, &mut new_blobs, &mut report)?;
                }
                else { import_favorite(&transaction, item_id, &record.legacy, &mut report)?; }
                if record.legacy.kind == "favorite" {
                    let group = record.legacy.group.trim();
                    if group != "全部" && valid_tag_name(group) {
                        let tag = match tags.get(&group.to_lowercase()) {
                            Some(tag) => *tag,
                            None => ensure_tag(&transaction, group, "#1677FF", now, &mut report)?,
                        };
                        transaction.execute("INSERT OR IGNORE INTO clipboard_item_tags (uuid, item_id, tag_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                            params![Uuid::new_v4().to_string(), item_id, tag, now])?;
                    }
                    import_title(&transaction, item_id, &record.legacy.title, now)?;
                }
                transaction.execute("INSERT INTO legacy_import_records (source_key, item_id, imported_at) VALUES (?1, ?2, ?3)", params![record.source_key, item_id, now])?;
                progress.store(completed, Ordering::Relaxed);
                Ok(())
            })?;
            if completed != prepared.selected_count(options) {
                return Err(MigrationError::Invalid(
                    "暂存记录数量与迁移预览不一致".to_owned(),
                ));
            }
            transaction.commit()?;
            Ok::<_, MigrationError>(())
        })();
        if let Err(error) = result {
            for path in new_blobs {
                if let Err(error) = fs::remove_file(&path) {
                    tracing::warn!("清理迁移未提交的资产文件失败：{}：{error}", path.display());
                }
            }
            return Err(MigrationError::RolledBack {
                cause: error.to_string(),
                backup: report.backup.display().to_string(),
            });
        }
        Ok(report)
    })
}

fn import_history_icon(
    tx: &Transaction<'_>,
    item_id: i64,
    record: &PreparedRecord,
    root: &Path,
    now: i64,
    new_files: &mut Vec<PathBuf>,
    report: &mut MigrationReport,
) -> Result<bool, MigrationError> {
    let Some(icon) = record.icon.as_ref() else {
        return Ok(false);
    };
    let Some(app) = record
        .legacy
        .source_app
        .as_deref()
        .filter(|app| !app.is_empty())
    else {
        return Ok(false);
    };
    // 只给来源一致的现存历史补空图标，不恢复删除记录或覆盖后来捕获的来源。
    let eligible: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM clipboard_history
        WHERE item_id = ?1 AND last_source_app_name = ?2 AND last_source_app_icon_id IS NULL)",
        params![item_id, app],
        |row| row.get(0),
    )?;
    if !eligible {
        return Ok(false);
    }
    let path = root.join(&icon.relative_path);
    let existed = path.exists();
    if existed && fs::read(&path)? != icon.bytes {
        return Err(MigrationError::Invalid(
            "新版已有图标文件与迁移图标内容不一致".to_owned(),
        ));
    }
    ApplicationIconStore::new(root).ensure_asset_file(&icon.relative_path, &icon.bytes)?;
    if !existed {
        new_files.push(path);
    }
    tx.execute("INSERT INTO application_icon_assets (uuid, sha256, relative_path, byte_length, width, height, created_at, last_used_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
        ON CONFLICT(sha256) DO UPDATE SET last_used_at = MAX(last_used_at, excluded.last_used_at)",
        params![Uuid::new_v4().to_string(), icon.sha256.as_slice(), icon.relative_path, icon.bytes.len() as i64, icon.width, icon.height, now])?;
    let icon_id: i64 = tx.query_row(
        "SELECT id FROM application_icon_assets WHERE sha256 = ?1",
        [icon.sha256.as_slice()],
        |row| row.get(0),
    )?;
    tx.execute(
        "UPDATE clipboard_history SET last_source_app_icon_id = ?1 WHERE item_id = ?2",
        params![icon_id, item_id],
    )?;
    report.imported_icons += 1;
    Ok(true)
}

fn ensure_tag(
    tx: &Transaction<'_>,
    name: &str,
    color: &str,
    now: i64,
    report: &mut MigrationReport,
) -> Result<i64, MigrationError> {
    let name = name.trim();
    let normalized = name.to_lowercase();
    if let Some(id) = tx
        .query_row(
            "SELECT id FROM tags WHERE normalized_name = ?1",
            [&normalized],
            |row| row.get(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    let order = next_order(tx, "tags", None)?;
    tx.execute("INSERT INTO tags (uuid, name, normalized_name, color, sort_order, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![Uuid::new_v4().to_string(), name, normalized, color, order, now])?;
    report.added_tags += 1;
    Ok(tx.last_insert_rowid())
}

fn next_order(
    tx: &Transaction<'_>,
    table: &str,
    pinned: Option<bool>,
) -> Result<i64, MigrationError> {
    let filter = pinned
        .map(|pinned| format!(" WHERE is_pinned = {}", i32::from(pinned)))
        .unwrap_or_default();
    let maximum: i64 = tx.query_row(
        &format!("SELECT COALESCE(MAX(sort_order), 0) FROM {table}{filter}"),
        [],
        |row| row.get(0),
    )?;
    maximum
        .checked_add(LIST_ORDER_GAP)
        .ok_or_else(|| MigrationError::Invalid("列表排序值超过支持范围".to_owned()))
}

fn import_content(
    tx: &Transaction<'_>,
    record: &PreparedRecord,
    store: &RawBlobStore,
    root: &Path,
    now: i64,
    new_blobs: &mut Vec<PathBuf>,
    report: &mut MigrationReport,
) -> Result<i64, MigrationError> {
    if let Some(id) = tx.query_row("SELECT id FROM clipboard_items WHERE hash_version = ?1 AND content_hash = ?2 AND capture_state = 'ready'",
        params![record.hashed.hash_version, record.hashed.content_hash.as_slice()], |row| row.get(0)).optional()? {
        report.merged_contents += 1; return Ok(id);
    }
    let primary = select_default_preview(&record.captured, &record.preview.formats)
        .ok_or_else(|| MigrationError::Invalid("内容没有可用的主格式".to_owned()))?;
    tx.execute("INSERT INTO clipboard_items (uuid, origin_kind, hash_version, content_hash, content_kind, capture_state,
        preview_text, preview_char_count, format_count, total_bytes, created_at, updated_at)
        VALUES (?1, 'imported', ?2, ?3, ?4, 'pending', ?5, ?6, 0, 0, ?7, ?8)",
        params![Uuid::new_v4().to_string(), record.hashed.hash_version, record.hashed.content_hash.as_slice(), record.preview.content_kind.as_str(),
            primary.text, primary.char_count as i64, record.legacy.created_at, now])?;
    let item_id = tx.last_insert_rowid();
    let mut total = 0_i64;
    for (index, format) in record.captured.formats.iter().enumerate() {
        let bytes = format
            .bytes
            .as_deref()
            .ok_or_else(|| MigrationError::Invalid("迁移格式缺少字节".to_owned()))?;
        let digest = record.hashed.format_hashes[index]
            .as_ref()
            .ok_or_else(|| MigrationError::Invalid("迁移格式缺少哈希".to_owned()))?;
        total += bytes.len() as i64;
        let (storage, inline, blob_id) = if bytes.len() <= 64 * 1024 {
            ("inline", Some(bytes), None)
        } else {
            let existing = tx.query_row("SELECT id, byte_length, relative_path FROM binary_blobs WHERE sha256 = ?1 AND storage_state = 'ready'", [digest.as_slice()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))).optional()?;
            let id = match existing {
                Some((id, length, path)) => {
                    store.verify(&super::storage::RawBlob::from_record(
                        *digest,
                        length as u64,
                        path,
                    ))?;
                    id
                }
                None => {
                    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
                    let existed = root.join(format!("data/{hex}.bin")).exists()
                        || root.join(format!("data/{hex}.bin.zst")).exists();
                    let pending = store.write_pending(bytes)?;
                    let blob = store.commit_pending(&pending)?;
                    let path = root.join(blob.relative_path());
                    // 数据运行锁保证这次新增的文件不会被其他写入同时引用。
                    if !existed {
                        new_blobs.push(path);
                    }
                    tx.execute("INSERT INTO binary_blobs (uuid, sha256, byte_length, relative_path, storage_state, created_at, last_verified_at)
                        VALUES (?1, ?2, ?3, ?4, 'ready', ?5, ?5)", params![Uuid::new_v4().to_string(), digest.as_slice(), bytes.len() as i64,
                            blob.relative_path().to_string_lossy(), now])?;
                    tx.last_insert_rowid()
                }
            };
            ("external", None, Some(id))
        };
        let preview = record
            .preview
            .formats
            .iter()
            .find(|preview| {
                preview.format_namespace == format.format_namespace
                    && preview.format_key == format.format_key
            })
            .ok_or_else(|| MigrationError::Invalid("迁移格式缺少预览".to_owned()))?;
        let is_primary = primary.format_namespace == format.format_namespace
            && primary.format_key == format.format_key;
        tx.execute("INSERT INTO clipboard_formats (uuid, item_id, format_order, format_namespace, format_key, format_name, native_format_id,
            format_kind, is_primary, storage_kind, inline_data, blob_id, byte_length, data_hash, restore_state, encoding, preview_text,
            preview_char_count, preview_content_kind, image_width, image_height, capture_state, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 'restorable', ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
            params![Uuid::new_v4().to_string(), item_id, index as i64, format.format_namespace, format.format_key, format.format_name,
                format.native_format_id.map(i64::from), format.format_kind.as_str(), is_primary, storage, inline, blob_id, bytes.len() as i64,
                digest.as_slice(), format.encoding, preview.text, preview.char_count as i64, preview.content_kind.as_str(),
                preview.image_dimensions.map(|size| i64::from(size.width)), preview.image_dimensions.map(|size| i64::from(size.height)), format.capture_state.as_str(), record.legacy.created_at])?;
    }
    tx.execute("UPDATE clipboard_items SET capture_state = 'ready', format_count = ?1, total_bytes = ?2 WHERE id = ?3",
        params![record.captured.formats.len() as i64, total, item_id])?;
    report.added_contents += 1;
    Ok(item_id)
}

fn import_history(
    tx: &Transaction<'_>,
    item_id: i64,
    record: &LegacyRecord,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let existing: Option<i64> = tx
        .query_row(
            "SELECT id FROM clipboard_history WHERE item_id = ?1",
            [item_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        // 已有内容保留当前位置、置顶和来源，只补充范围及统计，不把导入当作新复制事件。
        tx.execute("UPDATE clipboard_history SET first_seen_at = MIN(first_seen_at, ?1), last_seen_at = MAX(last_seen_at, ?2),
            paste_count = paste_count + ?3 WHERE id = ?4", params![record.created_at.min(record.updated_at), record.created_at.max(record.updated_at), record.paste_count, id])?;
    } else {
        let order = next_order(tx, "clipboard_history", Some(record.pinned))?;
        tx.execute("INSERT INTO clipboard_history (uuid, item_id, first_seen_at, last_seen_at, capture_count, paste_count, is_pinned,
            sort_order, last_source_app_name, updated_at) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8, ?4)",
            params![Uuid::new_v4().to_string(), item_id, record.created_at.min(record.updated_at), record.created_at.max(record.updated_at),
                record.paste_count, record.pinned, order, record.source_app])?;
        report.added_history += 1;
    }
    Ok(())
}

fn import_favorite(
    tx: &Transaction<'_>,
    item_id: i64,
    record: &LegacyRecord,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let existing: Option<i64> = tx
        .query_row(
            "SELECT id FROM favorites WHERE item_id = ?1",
            [item_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        tx.execute("UPDATE favorites SET paste_count = paste_count + ?1, added_at = MIN(added_at, ?2) WHERE id = ?3", params![record.paste_count, record.created_at, id])?;
    } else {
        let order = next_order(tx, "favorites", Some(false))?;
        tx.execute("INSERT INTO favorites (uuid, item_id, is_pinned, sort_order, paste_count, added_at, updated_at) VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6)",
            params![Uuid::new_v4().to_string(), item_id, order, record.paste_count, record.created_at, record.created_at.max(record.updated_at)])?;
        report.added_favorites += 1;
    }
    Ok(())
}

fn import_title(
    tx: &Transaction<'_>,
    item_id: i64,
    title: &str,
    now: i64,
) -> Result<(), MigrationError> {
    if title.trim().is_empty() {
        return Ok(());
    }
    let text = title.trim();
    let existing: Option<String> = tx
        .query_row(
            "SELECT note_text FROM clipboard_item_notes WHERE item_id = ?1",
            [item_id],
            |row| row.get(0),
        )
        .optional()?;
    match existing {
        Some(existing) if existing.lines().any(|line| line == text) => {}
        Some(existing) => {
            tx.execute("UPDATE clipboard_item_notes SET note_text = ?1, updated_at = ?2 WHERE item_id = ?3", params![format!("{existing}\n\n{text}"), now, item_id])?;
        }
        None => {
            tx.execute("INSERT INTO clipboard_item_notes (uuid, item_id, note_text, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                params![Uuid::new_v4().to_string(), item_id, text, now])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
