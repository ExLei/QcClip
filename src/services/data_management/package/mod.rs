mod archive;
mod cloud;
mod snapshot;

#[cfg(test)]
mod tests;

use super::{DataManagementError, DataManagementService};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) use cloud::CloudPackage;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("{0}")]
    Message(String),
    #[error("数据包文件操作失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("数据包数据库操作失败：{0}")]
    Sql(#[from] rusqlite::Error),
    #[error("数据包清单无效：{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Data(#[from] DataManagementError),
    #[error("{0}")]
    Database(#[from] crate::services::database::DatabaseError),
    #[error("{0}")]
    Sync(#[from] crate::services::sync::Error),
}
pub(crate) type Result<T> = std::result::Result<T, Error>;
fn invalid<T>(message: &str) -> Result<T> {
    Err(Error::Message(message.into()))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scope {
    pub history: bool,
    pub favorites: bool,
    pub settings: bool,
    pub gallery: bool,
}
impl Scope {
    pub fn any(&self) -> bool {
        self.history || self.favorites || self.settings || self.gallery
    }
    pub fn contains(&self, other: &Self) -> bool {
        (!other.history || self.history)
            && (!other.favorites || self.favorites)
            && (!other.settings || self.settings)
            && (!other.gallery || self.gallery)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ExportOptions {
    pub scope: Scope,
    pub history_days: Option<u32>,
    pub favorite_tags: Vec<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub version: u32,
    pub created_at: i64,
    pub scope: Scope,
    pub history_count: u64,
    pub favorite_count: u64,
    pub filtered: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportMode {
    Merge,
    Replace,
}

pub(crate) struct PreparedPackage {
    directory: tempfile::TempDir,
    pub manifest: Manifest,
}
impl PreparedPackage {
    pub fn root(&self) -> &Path {
        self.directory.path()
    }
}

impl DataManagementService {
    pub(crate) fn package_tags(&self) -> Result<Vec<(i64, String)>> {
        let connection = self.database.connect_read()?;
        let mut statement =
            connection.prepare("SELECT id, name FROM tags ORDER BY sort_order, id")?;
        Ok(statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn export_package(
        &self,
        destination: &Path,
        options: &ExportOptions,
        password: Option<&str>,
    ) -> Result<()> {
        if !options.scope.any() {
            return invalid("请选择要导出的数据");
        }
        if password.is_some_and(str::is_empty) {
            return invalid("加密密码不能为空");
        }
        if let Some(parent) = destination.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        if super::path_is_within_root(destination, &self.data_root)? {
            return invalid("导出文件不能保存在应用数据目录内");
        }
        let directory = tempfile::tempdir()?;
        let manifest = crate::services::data_runtime::with_exclusive_operation(|| {
            snapshot::export(self, directory.path(), options)
        })?;
        archive::write(directory.path(), &manifest, destination, password)
    }

    pub(crate) fn inspect_package(
        &self,
        path: &Path,
        password: Option<&str>,
    ) -> Result<PreparedPackage> {
        let directory = tempfile::tempdir()?;
        let manifest = archive::read(path, directory.path(), password)?;
        snapshot::validate(directory.path(), &manifest)?;
        Ok(PreparedPackage {
            directory,
            manifest,
        })
    }

    pub(crate) fn package_is_encrypted(path: &Path) -> Result<bool> {
        archive::is_encrypted(path)
    }

    pub(crate) fn import_package(
        &self,
        package: &PreparedPackage,
        scope: &Scope,
        mode: ImportMode,
    ) -> Result<(PathBuf, crate::services::data_runtime::DataSwitchGuard)> {
        if !scope.any() || !package.manifest.scope.contains(scope) {
            return invalid("请选择数据包中包含的数据范围");
        }
        // 从备份到应用期间保持独占，避免覆盖用户在预览之后的新操作。
        let (result, guard) =
            crate::services::data_runtime::with_exclusive_operation_and_guard(|| {
                let marker = self
                    .data_root
                    .join(super::IMPORT_DIRECTORY)
                    .join(super::IMPORT_MARKER);
                if fs::symlink_metadata(&marker).is_ok() {
                    super::preserve_failed_import(&self.data_root)?;
                }
                let current = tempfile::tempdir()?;
                snapshot::copy_current(self, current.path())?;
                let backup = self.data_root.join(".package-backups").join(format!(
                    "before-import-{}.qcclip-data",
                    uuid::Uuid::new_v4()
                ));
                fs::create_dir_all(backup.parent().unwrap())?;
                let manifest = snapshot::manifest(
                    current.path(),
                    Scope {
                        history: true,
                        favorites: true,
                        settings: true,
                        gallery: true,
                    },
                    false,
                )?;
                archive::write(current.path(), &manifest, &backup, None)?;
                snapshot::merge(current.path(), package.root(), scope, mode)?;
                let merged = snapshot::manifest(current.path(), manifest.scope, false)?;
                snapshot::validate(current.path(), &merged)?;
                self.stage_package_directory(current.path())?;
                let applied = super::apply_pending_import_while_paused(&self.data_root)?;
                if !applied {
                    return invalid("数据包未能应用，请重试");
                }
                Ok::<_, Error>(backup)
            });
        Ok((result?, guard))
    }

    fn stage_package_directory(&self, source: &Path) -> Result<()> {
        let pending = self.data_root.join(super::IMPORT_DIRECTORY);
        fs::create_dir_all(&pending)?;
        if pending.join(super::IMPORT_MARKER).exists() {
            return invalid("导入暂存状态发生变化，请重新尝试导入");
        }
        let name = uuid::Uuid::new_v4().to_string();
        let directory = pending.join(&name);
        fs::create_dir(&directory)?;
        let result = (|| {
            for entry in fs::read_dir(source)? {
                let entry = entry?;
                if super::is_importable_root_entry(&entry.file_name()) {
                    super::copy_import_entry(&entry.path(), &directory.join(entry.file_name()))?;
                }
            }
            let mut marker =
                atomic_write_file::AtomicWriteFile::open(pending.join(super::IMPORT_MARKER))?;
            std::io::Write::write_all(&mut marker, name.as_bytes())?;
            marker.commit()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&directory);
        }
        result
    }
}
