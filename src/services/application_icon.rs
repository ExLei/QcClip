use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::domain::capture::SourceApplication;

const ICON_DIRECTORY: &str = "data/app-icons";
const ICON_SIZE: u16 = 32;

#[derive(Clone, Debug)]
pub(crate) struct PreparedApplicationIcon {
    pub(crate) app_key: String,
    pub(crate) app_path: String,
    pub(crate) file_size: i64,
    pub(crate) modified_at: i64,
    pub(crate) sha256: [u8; 32],
    pub(crate) relative_path: String,
    pub(crate) byte_length: i64,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct ApplicationIconStore {
    root: PathBuf,
}

impl ApplicationIconStore {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub(crate) fn resolve(
        &self,
        connection: &Connection,
        source: &SourceApplication,
    ) -> Option<PreparedApplicationIcon> {
        let app_path = source.path.as_deref()?.to_owned();
        let path = Path::new(&app_path);
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::debug!(%error, "读取来源应用文件元数据失败");
                return None;
            }
        };
        if !metadata.is_file() {
            tracing::debug!("来源应用路径不是文件，跳过应用图标缓存");
            return None;
        }
        let file_size = match i64::try_from(metadata.len()) {
            Ok(file_size) => file_size,
            Err(_) => {
                tracing::debug!("来源应用文件大小超出支持范围，跳过应用图标缓存");
                return None;
            }
        };
        let modified_at = modified_timestamp(&metadata).unwrap_or(0);
        let app_key = normalize_app_path(path);

        if let Some(cached) = self.load_cached(connection, &app_key, file_size, modified_at) {
            return Some(cached);
        }

        let (png_data, width, height) = match extract_icon(path) {
            Some(icon) => icon,
            None => {
                tracing::debug!("提取来源应用图标失败");
                return None;
            }
        };
        let sha256 = hash_bytes(&png_data);
        let relative_path = format!("{ICON_DIRECTORY}/{}.png", encode_hex(&sha256));
        if let Err(error) = self.ensure_asset_file(&relative_path, &png_data) {
            tracing::debug!(?error, "保存应用图标缓存失败");
            return None;
        }

        Some(PreparedApplicationIcon {
            app_key,
            app_path,
            file_size,
            modified_at,
            sha256,
            relative_path,
            byte_length: match i64::try_from(png_data.len()) {
                Ok(byte_length) => byte_length,
                Err(_) => {
                    tracing::debug!("来源应用图标大小超出支持范围");
                    return None;
                }
            },
            width,
            height,
        })
    }

    fn load_cached(
        &self,
        connection: &Connection,
        app_key: &str,
        file_size: i64,
        modified_at: i64,
    ) -> Option<PreparedApplicationIcon> {
        let cached = connection
            .query_row(
                "SELECT c.app_path, c.file_size, c.modified_at,
                        a.sha256, a.relative_path, a.byte_length, a.width, a.height
                 FROM application_icon_cache AS c
                 JOIN application_icon_assets AS a ON a.id = c.icon_asset_id
                 WHERE c.app_key = ?1",
                [app_key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, u32>(6)?,
                        row.get::<_, u32>(7)?,
                    ))
                },
            )
            .optional()
            .ok()??;
        if cached.1 != file_size || cached.2 != modified_at {
            return None;
        }

        let sha256: [u8; 32] = cached.3.try_into().ok()?;
        if !self.verify_asset(&cached.4, &sha256, cached.5) {
            return None;
        }

        Some(PreparedApplicationIcon {
            app_key: app_key.to_owned(),
            app_path: cached.0,
            file_size,
            modified_at,
            sha256,
            relative_path: cached.4,
            byte_length: cached.5,
            width: cached.6,
            height: cached.7,
        })
    }

    pub(crate) fn ensure_asset_file(
        &self,
        relative_path: &str,
        bytes: &[u8],
    ) -> Result<(), std::io::Error> {
        let path = self.asset_path(relative_path).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "应用图标路径无效")
        })?;
        if path.is_file() {
            if fs::read(&path)
                .map(|existing| existing == bytes)
                .unwrap_or(false)
            {
                return Ok(());
            }
            fs::remove_file(&path)?;
        }
        let directory = path.parent().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "应用图标目录无效")
        })?;
        fs::create_dir_all(directory)?;
        let temporary_path = directory.join(format!(
            ".{}.{}.tmp",
            Uuid::new_v4(),
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
        fs::write(&temporary_path, bytes)?;
        match fs::rename(&temporary_path, &path) {
            Ok(()) => Ok(()),
            Err(error) if path.is_file() => {
                let result = if fs::read(&path)
                    .map(|existing| existing == bytes)
                    .unwrap_or(false)
                {
                    Ok(())
                } else {
                    Err(error)
                };
                let _ = fs::remove_file(&temporary_path);
                result
            }
            Err(error) => {
                let _ = fs::remove_file(&temporary_path);
                Err(error)
            }
        }
    }

    fn verify_asset(
        &self,
        relative_path: &str,
        expected_hash: &[u8; 32],
        expected_length: i64,
    ) -> bool {
        let Some(path) = self.asset_path(relative_path) else {
            return false;
        };
        let Ok(bytes) = fs::read(path) else {
            return false;
        };
        i64::try_from(bytes.len()).ok() == Some(expected_length)
            && hash_bytes(&bytes) == *expected_hash
    }

    fn asset_path(&self, relative_path: &str) -> Option<PathBuf> {
        let relative = Path::new(relative_path);
        if relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
            || relative_path.replace('\\', "/")
                != format!(
                    "{ICON_DIRECTORY}/{}",
                    relative.file_name()?.to_string_lossy()
                )
        {
            return None;
        }
        Some(self.root.join(relative))
    }
}

fn modified_timestamp(metadata: &fs::Metadata) -> Option<i64> {
    metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
}

fn normalize_app_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

#[cfg(target_os = "windows")]
fn extract_icon(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    use std::io::Cursor;

    use file_icon_provider::get_file_icon;
    use image::{ImageFormat, RgbaImage};

    let icon = get_file_icon(path, ICON_SIZE).ok()?;
    let image = RgbaImage::from_raw(icon.width, icon.height, icon.pixels.clone())?;
    let width = image.width();
    let height = image.height();
    let mut png_data = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut png_data), ImageFormat::Png)
        .ok()?;
    Some((png_data, width, height))
}

#[cfg(not(target_os = "windows"))]
fn extract_icon(_path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    None
}

fn hash_bytes(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

fn encode_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn upsert_application_icon(
    transaction: &rusqlite::Transaction<'_>,
    prepared: Option<&PreparedApplicationIcon>,
    timestamp: i64,
) -> Result<Option<i64>, rusqlite::Error> {
    let Some(prepared) = prepared else {
        return Ok(None);
    };

    transaction.execute(
        "INSERT INTO application_icon_assets (
            uuid, sha256, relative_path, byte_length, width, height, created_at, last_used_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         ON CONFLICT(sha256) DO UPDATE SET last_used_at = excluded.last_used_at",
        params![
            Uuid::new_v4().to_string(),
            prepared.sha256.as_slice(),
            prepared.relative_path,
            prepared.byte_length,
            i64::from(prepared.width),
            i64::from(prepared.height),
            timestamp,
        ],
    )?;
    let icon_asset_id: i64 = transaction.query_row(
        "SELECT id FROM application_icon_assets WHERE sha256 = ?1",
        [prepared.sha256.as_slice()],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO application_icon_cache (
            app_key, app_path, file_size, modified_at, icon_asset_id, last_checked_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(app_key) DO UPDATE SET
            app_path = excluded.app_path,
            file_size = excluded.file_size,
            modified_at = excluded.modified_at,
            icon_asset_id = excluded.icon_asset_id,
            last_checked_at = excluded.last_checked_at",
        params![
            prepared.app_key,
            prepared.app_path,
            prepared.file_size,
            prepared.modified_at,
            icon_asset_id,
            timestamp,
        ],
    )?;
    Ok(Some(icon_asset_id))
}
