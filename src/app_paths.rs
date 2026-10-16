use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use atomic_write_file::AtomicWriteFile;
use directories::{BaseDirs, ProjectDirs};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const ORGANIZATION_NAME: &str = "QcClip";
const APPLICATION_NAME: &str = "QcClipData";
const CONFIG_FILE_NAME: &str = "config.json";
const STATE_FILE_NAME: &str = "state.json";
const DATABASE_FILE_NAME: &str = "qcclip.sqlite3";
const SOUND_DIRECTORY_NAME: &str = "sound";
const LOG_DIRECTORY_NAME: &str = "log";
const IMAGE_LIBRARY_DIRECTORY_NAME: &str = "image_library";
const LOCATION_FILE_NAME: &str = "location.json";
const PORTABLE_MARKER_FILE_NAME: &str = "QcClip.portable";

#[derive(Debug, Error)]
pub(crate) enum AppPathsError {
    #[error("无法确定应用数据目录")]
    DataDirectoryUnavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppPaths {
    host_root: PathBuf,
    default_data_root: PathBuf,
    data_root: PathBuf,
    portable: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct LocationFile {
    data_root: PathBuf,
}

impl AppPaths {
    pub(crate) fn discover() -> Result<Self, AppPathsError> {
        let executable = match std::env::current_exe() {
            Ok(executable) => Some(executable),
            Err(error) => {
                tracing::warn!(%error, "无法读取程序路径，使用安装版数据目录");
                None
            }
        };
        if let Some(paths) = executable
            .as_deref()
            .and_then(Self::portable_from_executable)
        {
            return Ok(paths);
        }
        let project_dirs = ProjectDirs::from("com", ORGANIZATION_NAME, APPLICATION_NAME)
            .ok_or(AppPathsError::DataDirectoryUnavailable)?;
        let base_dirs = BaseDirs::new().ok_or(AppPathsError::DataDirectoryUnavailable)?;
        let default_data_root = base_dirs.data_local_dir().join(project_dirs.project_path());
        let host_root = base_dirs.data_local_dir().join(ORGANIZATION_NAME);
        let location_path = host_root.join(LOCATION_FILE_NAME);
        let data_root = match read_location(&location_path) {
            Ok(Some(path)) => path,
            Ok(None) => default_data_root.clone(),
            Err(error) => {
                tracing::warn!(path = ?location_path, %error, "读取数据位置失败，已使用默认数据目录");
                default_data_root.clone()
            }
        };
        Ok(Self {
            host_root,
            default_data_root,
            data_root,
            portable: false,
        })
    }

    fn portable_from_executable(executable: &Path) -> Option<Self> {
        let directory = executable.parent()?;
        if !directory.join(PORTABLE_MARKER_FILE_NAME).is_file() {
            return None;
        }
        let data_root = directory.join(APPLICATION_NAME);
        Some(Self {
            host_root: directory.to_path_buf(),
            default_data_root: data_root.clone(),
            data_root,
            portable: true,
        })
    }

    #[cfg(test)]
    fn from_data_root(data_root: impl Into<PathBuf>) -> Self {
        let data_root = data_root.into();
        Self {
            host_root: PathBuf::new(),
            default_data_root: data_root.clone(),
            data_root,
            portable: false,
        }
    }

    pub(crate) fn is_portable(&self) -> bool {
        self.portable
    }

    pub(crate) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(crate) fn default_data_root(&self) -> &Path {
        &self.default_data_root
    }

    pub(crate) fn location_path(&self) -> PathBuf {
        self.host_root.join(LOCATION_FILE_NAME)
    }

    pub(crate) fn config_path(&self) -> PathBuf {
        self.data_root.join(CONFIG_FILE_NAME)
    }

    pub(crate) fn state_path(&self) -> PathBuf {
        self.data_root.join(STATE_FILE_NAME)
    }

    pub(crate) fn database_path(&self) -> PathBuf {
        self.data_root.join(DATABASE_FILE_NAME)
    }

    pub(crate) fn sound_directory(&self) -> PathBuf {
        self.data_root.join(SOUND_DIRECTORY_NAME)
    }

    pub(crate) fn log_directory(&self) -> PathBuf {
        self.data_root.join(LOG_DIRECTORY_NAME)
    }

    pub(crate) fn image_library_directory(&self) -> PathBuf {
        self.data_root.join(IMAGE_LIBRARY_DIRECTORY_NAME)
    }
}

pub(crate) fn write_location(path: &Path, data_root: &Path) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_vec_pretty(&LocationFile {
        data_root: data_root.to_path_buf(),
    })
    .map_err(std::io::Error::other)?;
    let mut file = AtomicWriteFile::open(path)?;
    file.write_all(&content)?;
    file.as_file().sync_all()?;
    file.commit()
}

fn read_location(path: &Path) -> std::io::Result<Option<PathBuf>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let location = serde_json::from_str::<LocationFile>(&content)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if !location.data_root.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "数据位置必须是绝对路径",
        ));
    }
    Ok(Some(location.data_root))
}

#[cfg(test)]
mod tests {
    use super::AppPaths;
    use std::fs;

    #[test]
    fn location_read_distinguishes_missing_corrupt_and_relative_paths() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("location.json");
        assert_eq!(super::read_location(&path).unwrap(), None);
        fs::write(&path, "{损坏").unwrap();
        assert_eq!(
            super::read_location(&path).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        fs::write(&path, r#"{"data_root":"relative"}"#).unwrap();
        assert!(super::read_location(&path).is_err());
        super::write_location(&path, root.path()).unwrap();
        assert_eq!(
            super::read_location(&path).unwrap(),
            Some(root.path().to_path_buf())
        );
    }

    #[test]
    fn derived_paths_share_the_same_data_root() {
        let paths = AppPaths::from_data_root("data");

        assert_eq!(paths.data_root().to_string_lossy(), "data");
        assert_eq!(
            paths.config_path(),
            std::path::Path::new("data/config.json")
        );
        assert_eq!(paths.state_path(), std::path::Path::new("data/state.json"));
        assert_eq!(
            paths.database_path(),
            std::path::Path::new("data/qcclip.sqlite3")
        );
        assert_eq!(paths.sound_directory(), std::path::Path::new("data/sound"));
        assert_eq!(paths.log_directory(), std::path::Path::new("data/log"));
        assert_eq!(
            paths.image_library_directory(),
            std::path::Path::new("data/image_library")
        );
    }

    #[test]
    fn portable_marker_keeps_all_data_next_to_the_executable() {
        let directory = tempfile::tempdir().expect("应创建临时目录");
        fs::write(directory.path().join("QcClip.portable"), b"").expect("应创建便携标记");
        let paths = AppPaths::portable_from_executable(&directory.path().join("QcClip.exe"))
            .expect("应识别便携版");
        let data_root = directory.path().join("QcClipData");

        assert!(paths.is_portable());
        assert_eq!(paths.data_root(), data_root);
        assert_eq!(paths.default_data_root(), data_root);
        assert_eq!(paths.config_path(), data_root.join("config.json"));
        assert_eq!(paths.database_path(), data_root.join("qcclip.sqlite3"));
        assert_eq!(paths.state_path(), data_root.join("state.json"));
        assert_eq!(paths.log_directory(), data_root.join("log"));
        assert_eq!(paths.sound_directory(), data_root.join("sound"));
        assert_eq!(
            paths.image_library_directory(),
            data_root.join("image_library")
        );
    }

    #[test]
    fn portable_mode_ignores_custom_data_locations() {
        let directory = tempfile::tempdir().expect("应创建临时目录");
        let custom = tempfile::tempdir().expect("应创建自定义数据目录");
        fs::write(directory.path().join("QcClip.portable"), b"").expect("应创建便携标记");
        super::write_location(&directory.path().join("location.json"), custom.path())
            .expect("应保存自定义数据位置");

        let paths = AppPaths::portable_from_executable(&directory.path().join("QcClip.exe"))
            .expect("应识别便携版");
        assert_eq!(paths.data_root(), directory.path().join("QcClipData"));
    }

    #[test]
    fn portable_data_follows_the_moved_directory() {
        let directory = tempfile::tempdir().expect("应创建临时目录");
        let original = directory.path().join("original");
        let moved = directory.path().join("moved");
        fs::create_dir_all(original.join("QcClipData")).expect("应创建便携数据目录");
        fs::write(original.join("QcClip.portable"), b"").expect("应创建便携标记");
        fs::write(original.join("QcClipData/config.json"), b"{}").expect("应创建配置文件");
        fs::rename(&original, &moved).expect("应移动便携目录");

        let paths = AppPaths::portable_from_executable(&moved.join("QcClip.exe"))
            .expect("应识别移动后的便携版");
        assert_eq!(paths.data_root(), moved.join("QcClipData"));
        assert_eq!(fs::read(paths.config_path()).expect("应读取原配置"), b"{}");
    }

    #[test]
    fn portable_mode_requires_a_marker_file() {
        let directory = tempfile::tempdir().expect("应创建临时目录");
        let executable = directory.path().join("QcClip.exe");
        fs::create_dir(directory.path().join("QcClipData")).expect("应创建数据目录");
        assert!(AppPaths::portable_from_executable(&executable).is_none());

        fs::create_dir(directory.path().join("QcClip.portable")).expect("应创建同名目录");
        assert!(AppPaths::portable_from_executable(&executable).is_none());
        assert!(!AppPaths::from_data_root(directory.path()).is_portable());
    }
}
