use std::{
    io,
    os::windows::{
        ffi::OsStrExt as _,
        io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle},
    },
    path::Path,
    sync::Arc,
    thread::{self, JoinHandle},
};

use windows::{
    Win32::{
        Foundation::{HANDLE, WAIT_OBJECT_0},
        Storage::FileSystem::{
            FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
            FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FindCloseChangeNotification,
            FindFirstChangeNotificationW, FindNextChangeNotification,
        },
        System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects},
    },
    core::PCWSTR,
};

struct ChangeNotification(isize);

impl ChangeNotification {
    fn handle(&self) -> HANDLE {
        HANDLE(self.0 as _)
    }
}

impl Drop for ChangeNotification {
    fn drop(&mut self) {
        // SAFETY: 句柄仅由本对象持有，且在线程结束等待后释放。
        unsafe {
            let _ = FindCloseChangeNotification(self.handle());
        }
    }
}

pub(super) struct DirectoryWatcher {
    changes: async_channel::Receiver<()>,
    stop: Arc<OwnedHandle>,
    worker: Option<JoinHandle<()>>,
}

impl DirectoryWatcher {
    pub(super) fn new(directory: &Path) -> io::Result<Self> {
        let path: Vec<u16> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: 路径以空字符结尾，在调用期间有效。
        let notification = ChangeNotification(
            unsafe {
                FindFirstChangeNotificationW(
                    PCWSTR(path.as_ptr()),
                    true,
                    FILE_NOTIFY_CHANGE_FILE_NAME
                        | FILE_NOTIFY_CHANGE_DIR_NAME
                        | FILE_NOTIFY_CHANGE_LAST_WRITE
                        | FILE_NOTIFY_CHANGE_SIZE,
                )
            }
            .map_err(io::Error::from)?
            .0 as isize,
        );
        // SAFETY: 创建独立的手动重置事件，由 OwnedHandle 负责唯一一次关闭。
        let stop = unsafe { CreateEventW(None, true, false, None) }.map_err(io::Error::from)?;
        // SAFETY: 接管刚创建的事件句柄，此后不再通过原始句柄释放。
        let stop = Arc::new(unsafe { OwnedHandle::from_raw_handle(stop.0) });
        let worker_stop = stop.clone();
        let (sender, changes) = async_channel::bounded(1);
        let worker = thread::Builder::new()
            .name("qcclip-theme-watch".to_owned())
            .spawn(move || {
                let handles = [HANDLE(worker_stop.as_raw_handle()), notification.handle()];
                loop {
                    // SAFETY: 两个句柄均在线程退出前保持有效，无事件时无限期阻塞。
                    let result = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
                    if result == WAIT_OBJECT_0 {
                        break;
                    }
                    if result.0 != WAIT_OBJECT_0.0 + 1 {
                        tracing::error!("等待主题目录变更失败：{}", io::Error::last_os_error());
                        break;
                    }
                    // SAFETY: 先重新监听，再通知主线程，避免重载期间漏掉后续修改。
                    if let Err(error) = unsafe { FindNextChangeNotification(notification.handle()) }
                    {
                        tracing::error!("重新监听主题目录失败：{error}");
                        break;
                    }
                    match sender.try_send(()) {
                        Ok(()) | Err(async_channel::TrySendError::Full(())) => {}
                        Err(async_channel::TrySendError::Closed(())) => break,
                    }
                }
            })?;
        Ok(Self {
            changes,
            stop,
            worker: Some(worker),
        })
    }

    pub(super) async fn changed(&self) -> Result<(), async_channel::RecvError> {
        self.changes.recv().await
    }
}

impl Drop for DirectoryWatcher {
    fn drop(&mut self) {
        // SAFETY: 事件由 Arc 持有，唤醒线程并等其退出后才会关闭。
        unsafe {
            let _ = SetEvent(HANDLE(self.stop.as_raw_handle()));
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{Duration, Instant},
    };

    fn wait_for_change(watcher: &DirectoryWatcher) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match watcher.changes.try_recv() {
                Ok(()) => return,
                Err(async_channel::TryRecvError::Empty) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                result => panic!("未收到主题目录变更事件：{result:?}"),
            }
        }
    }

    #[test]
    fn reports_create_modify_rename_and_remove() {
        for operation in ["create", "modify", "rename", "remove"] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("theme.json");
            if operation != "create" {
                fs::write(&path, "{}").unwrap();
            }
            let watcher = DirectoryWatcher::new(directory.path()).unwrap();
            match operation {
                "create" => fs::write(&path, "{}").unwrap(),
                "modify" => fs::write(&path, "{\"themes\": []}").unwrap(),
                "rename" => fs::rename(&path, directory.path().join("renamed.json")).unwrap(),
                "remove" => fs::remove_file(&path).unwrap(),
                _ => unreachable!(),
            }
            wait_for_change(&watcher);
        }
    }

    #[test]
    fn dropping_watcher_stops_idle_worker() {
        let directory = tempfile::tempdir().unwrap();
        let mut watcher = DirectoryWatcher::new(directory.path()).unwrap();
        let worker = watcher.worker.take().unwrap();
        drop(watcher);
        worker.join().expect("主题目录监听线程退出失败");
    }

    #[test]
    fn missing_directory_returns_error() {
        let directory = tempfile::tempdir().unwrap();
        assert!(DirectoryWatcher::new(&directory.path().join("missing")).is_err());
    }
}
