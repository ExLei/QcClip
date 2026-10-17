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

pub(crate) fn activation_requests(
    path: PathBuf,
) -> io::Result<async_channel::Receiver<io::Result<()>>> {
    let (sender, receiver) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("qcclip-instance-activation".to_owned())
        .spawn(move || {
            if let Err(error) = watch_activation_requests(&path, &sender) {
                let _ = sender.try_send(Err(error));
            }
        })?;
    Ok(receiver)
}

#[cfg(target_os = "windows")]
fn watch_activation_requests(
    path: &Path,
    sender: &async_channel::Sender<io::Result<()>>,
) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows::{
        Win32::{
            Foundation::{HANDLE, WAIT_OBJECT_0},
            Storage::FileSystem::{
                FILE_NOTIFY_CHANGE_FILE_NAME, FindCloseChangeNotification,
                FindFirstChangeNotificationW, FindNextChangeNotification,
            },
            System::Threading::{INFINITE, WaitForSingleObject},
        },
        core::PCWSTR,
    };

    struct ChangeNotification(HANDLE);
    impl Drop for ChangeNotification {
        fn drop(&mut self) {
            // SAFETY: 该句柄由 FindFirstChangeNotificationW 创建，仅释放一次。
            unsafe {
                let _ = FindCloseChangeNotification(self.0);
            }
        }
    }

    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("单实例通知目录无效"))?;
    let directory: Vec<u16> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: 目录路径以空字符结尾，缓冲区在调用期间有效。
    let notification = ChangeNotification(
        unsafe {
            FindFirstChangeNotificationW(
                PCWSTR(directory.as_ptr()),
                false,
                FILE_NOTIFY_CHANGE_FILE_NAME,
            )
        }
        .map_err(io::Error::from)?,
    );
    loop {
        if sender.is_closed() {
            return Ok(());
        }
        if take_activation_request(path)? && sender.try_send(Ok(())).is_err() {
            return Ok(());
        }
        // 先监听再读取请求；通知后先重新武装，避免初始化及连续启动时漏掉唤出请求。
        // SAFETY: 通知句柄有效，线程阻塞等待系统目录变更，不进行定时轮询。
        if unsafe { WaitForSingleObject(notification.0, INFINITE) } != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        unsafe { FindNextChangeNotification(notification.0) }.map_err(io::Error::from)?;
    }
}

#[cfg(not(target_os = "windows"))]
fn watch_activation_requests(
    path: &Path,
    sender: &async_channel::Sender<io::Result<()>>,
) -> io::Result<()> {
    while !sender.is_closed() {
        if take_activation_request(path)? && sender.try_send(Ok(())).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
