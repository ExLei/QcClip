use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) const WAIT_FOR_INSTANCE_ARGUMENT: &str = "--wait-for-instance";

pub(crate) struct SingleInstance {
    _lock: File,
    activation_path: PathBuf,
}

impl SingleInstance {
    pub(crate) fn acquire() -> io::Result<Option<Self>> {
        let directories = directories::BaseDirs::new()
            .ok_or_else(|| io::Error::other("无法确定单实例运行目录"))?;
        // 独立于可迁移的数据目录，保证安装版和便携版也只能运行一个实例。
        let directory = directories.data_local_dir().join("QcClip").join("runtime");
        let wait = if std::env::args_os().any(|argument| argument == WAIT_FOR_INSTANCE_ARGUMENT) {
            Duration::from_secs(15)
        } else {
            Duration::ZERO
        };
        Self::acquire_at(&directory, wait)
    }

    pub(crate) fn acquire_at(directory: &Path, wait: Duration) -> io::Result<Option<Self>> {
        fs::create_dir_all(directory)?;
        // 锁文件必须保留；删除后可能让不同进程锁住不同文件，失去互斥。
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("single-instance.lock"))?;
        let activation_path = directory.join("activate-main-window");
        let deadline = Instant::now() + wait;
        loop {
            match lock.try_lock() {
                Ok(()) => {
                    return Ok(Some(Self {
                        _lock: lock,
                        activation_path,
                    }));
                }
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(TryLockError::WouldBlock) if wait.is_zero() => {
                    // 文件通知会保留到主窗口就绪，多次启动合并成一次显示请求。
                    match OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(activation_path)
                    {
                        Ok(_) => {}
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error),
                    }
                    return Ok(None);
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "等待旧实例退出超时",
                    ));
                }
                Err(TryLockError::Error(error)) => return Err(error),
            }
        }
    }

    pub(crate) fn activation_path(&self) -> PathBuf {
        self.activation_path.clone()
    }
}

pub(crate) fn take_activation_request(path: &Path) -> io::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}
