use std::{
    backtrace::Backtrace,
    borrow::Cow,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, RwLock},
};

use tracing_appender::non_blocking::{ErrorCounter, NonBlocking, WorkerGuard};
use tracing_subscriber::{EnvFilter, filter::filter_fn, fmt::writer::MakeWriter, prelude::*};

const LOG_FILE_PREFIX: &str = "qcclip";
const LOG_FILE_SUFFIX: &str = "log";
const DEFAULT_FILTER: &str = "off,QcClip=info";
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
const LOG_RETENTION_DAYS: i64 = 14;
const LOG_ARCHIVES: usize = 3;
static LOG_DIRECTORY: OnceLock<Arc<RwLock<PathBuf>>> = OnceLock::new();
static SHARED_LOG_FILES: Mutex<Vec<tempfile::NamedTempFile>> = Mutex::new(Vec::new());

pub(crate) struct LoggingGuard {
    worker: Option<WorkerGuard>,
    files: Arc<LogFiles>,
}

impl Drop for LoggingGuard {
    fn drop(&mut self) {
        // 先排空普通日志，再同步保存队列丢弃统计。
        drop(self.worker.take());
        let message = format!("{} INFO 应用日志写入结束\n", chrono::Local::now());
        self.files.write(message.as_bytes(), false);
        SHARED_LOG_FILES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }
}

pub(crate) fn fallback_directory() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .unwrap_or_else(std::env::temp_dir)
        .join("QcClip")
        .join("log-fallback")
}

pub(crate) fn init(log_directory: &Path) -> LoggingGuard {
    let directory = Arc::new(RwLock::new(log_directory.to_path_buf()));
    let _ = LOG_DIRECTORY.set(directory.clone());
    let files = Arc::new(LogFiles {
        directory,
        fallback: fallback_directory(),
        state: Mutex::new(FileState::default()),
        dropped: OnceLock::new(),
    });
    let (ordinary, worker) = tracing_appender::non_blocking(DynamicLogWriter(files.clone()));
    let _ = files.dropped.set(ordinary.error_counter());
    let writer = LogMakeWriter {
        ordinary,
        files: files.clone(),
    };
    if let Err(error) = tracing_subscriber::registry()
        .with(log_filter())
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_ansi(false)
                .with_target(true)
                .with_thread_ids(true)
                .with_file(true)
                .with_line_number(true)
                .with_filter(filter_fn(is_project_metadata)),
        )
        .try_init()
    {
        let message = format!(
            "{} ERROR 安装日志订阅器失败：{error}\n",
            chrono::Local::now()
        );
        write_stderr_redacted(&message);
        files.write(message.as_bytes(), false);
    }
    install_panic_hook(files.clone());
    // 实际打开文件，避免仅创建目录成功却无法写入日志。
    let message = format!(
        "{} INFO 日志文件写入检查完成，版本={}，系统={}，架构={}，进程={}\n",
        chrono::Local::now(),
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::process::id()
    );
    files.write(message.as_bytes(), false);
    LoggingGuard {
        worker: Some(worker),
        files,
    }
}

pub(crate) fn switch_directory(log_directory: &Path) -> Result<(), io::Error> {
    prepare_directory(log_directory)?;
    let Some(directory) = LOG_DIRECTORY.get() else {
        return Ok(());
    };
    let mut current = directory
        .write()
        .map_err(|_| io::Error::other("日志目录锁状态异常"))?;
    *current = log_directory.to_path_buf();
    Ok(())
}

pub(crate) fn latest_log_file() -> io::Result<Option<PathBuf>> {
    let directory = LOG_DIRECTORY
        .get()
        .ok_or_else(|| io::Error::other("日志目录尚未初始化"))?
        .read()
        .map_err(|_| io::Error::other("日志目录锁状态异常"))?
        .clone();
    let primary = latest_log_file_in(&directory);
    let fallback = latest_log_file_in(&fallback_directory());
    match (primary, fallback) {
        (Ok(Some(primary)), Ok(Some(fallback))) => {
            if fs::metadata(&fallback)?.modified()? > fs::metadata(&primary)?.modified()? {
                Ok(Some(fallback))
            } else {
                Ok(Some(primary))
            }
        }
        (Ok(Some(primary)), _) => Ok(Some(primary)),
        (_, Ok(Some(fallback))) => Ok(Some(fallback)),
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(None), Ok(None)) => Ok(None),
    }
}

pub(crate) fn latest_log_for_sharing() -> io::Result<Option<PathBuf>> {
    latest_log_file()?
        .map(|path| create_log_share_file(&path))
        .transpose()
}

fn create_log_share_file(source: &Path) -> io::Result<PathBuf> {
    let bytes = fs::read(source)?;
    let content = String::from_utf8_lossy(&bytes);
    let mut file = tempfile::Builder::new()
        .prefix("qcclip-shared-")
        .suffix(".log")
        .tempfile()?;
    file.write_all(redact_paths(&content).as_bytes())?;
    file.flush()?;
    let path = file.path().to_path_buf();
    // 剪贴板接收方可能延迟读取，脱敏副本保留到应用退出。
    SHARED_LOG_FILES
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(file);
    Ok(path)
}

pub(crate) fn with_directory_switch<T, E: From<io::Error>>(
    target: &Path,
    operation: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let Some(directory) = LOG_DIRECTORY.get() else {
        return operation();
    };
    // 重置期间的关键错误写入备用目录，避免重入目录锁。
    let mut current = directory
        .write()
        .map_err(|_| io::Error::other("日志目录锁状态异常"))?;
    let result = operation();
    if result.is_ok() {
        *current = target.to_path_buf();
    }
    result
}

fn log_date(name: &str) -> Option<chrono::NaiveDate> {
    let date = name.strip_prefix("qcclip-")?.split('.').next()?;
    let suffix = name.strip_prefix(&format!("qcclip-{date}"))?;
    if suffix != ".log"
        && suffix != ".panic.log"
        && !(1..=LOG_ARCHIVES).any(|index| suffix == format!(".log.{index}"))
    {
        return None;
    }
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

fn latest_log_file_in(directory: &Path) -> io::Result<Option<PathBuf>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut latest = None;
    for entry in entries {
        let entry = entry?;
        if entry.file_name().to_str().and_then(log_date).is_none() || !entry.file_type()?.is_file()
        {
            continue;
        }
        let candidate = (entry.metadata()?.modified()?, entry.path());
        if latest.as_ref().is_none_or(|current| &candidate > current) {
            latest = Some(candidate);
        }
    }
    Ok(latest.map(|(_, path)| path))
}

fn daily_path(directory: &Path, panic: bool) -> PathBuf {
    let date = chrono::Local::now().format("%Y-%m-%d");
    directory.join(format!(
        "{LOG_FILE_PREFIX}-{date}{}.{}",
        if panic { ".panic" } else { "" },
        LOG_FILE_SUFFIX
    ))
}

fn prepare_directory(directory: &Path) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(daily_path(directory, false))?;
    Ok(())
}

fn cleanup_old_logs(directory: &Path, today: chrono::NaiveDate) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let Some(date) = entry.file_name().to_str().and_then(log_date) else {
            continue;
        };
        if today.signed_duration_since(date).num_days() >= LOG_RETENTION_DAYS
            && entry.file_type()?.is_file()
        {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn append_log(directory: &Path, bytes: &[u8], panic: bool) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let path = daily_path(directory, panic);
    if !panic
        && fs::metadata(&path)
            .map(|meta| meta.len() >= MAX_LOG_BYTES)
            .unwrap_or(false)
    {
        match fs::remove_file(path.with_extension(format!("log.{LOG_ARCHIVES}"))) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        for index in (1..LOG_ARCHIVES).rev() {
            let source = path.with_extension(format!("log.{index}"));
            if source.exists() {
                fs::rename(source, path.with_extension(format!("log.{}", index + 1)))?;
            }
        }
        fs::rename(&path, path.with_extension("log.1"))?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let content = String::from_utf8_lossy(bytes);
    file.write_all(redact_paths(&content).as_bytes())?;
    if panic {
        file.sync_all()?;
    }
    Ok(())
}

#[derive(Default)]
struct FileState {
    failed: bool,
    reported_dropped: usize,
    cleaned: Option<(PathBuf, chrono::NaiveDate)>,
}

struct LogFiles {
    directory: Arc<RwLock<PathBuf>>,
    fallback: PathBuf,
    state: Mutex<FileState>,
    dropped: OnceLock<ErrorCounter>,
}

impl LogFiles {
    fn write(&self, bytes: &[u8], panic: bool) {
        // 同步日志可能来自持有目录写锁的操作，不能等待该锁。
        let directory = self.directory.try_read().ok();
        let primary = directory.as_deref().unwrap_or(&self.fallback);
        let mut state = if panic {
            match self.state.try_lock() {
                Ok(state) => state,
                Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    // 日志写入自身发生 panic 或其他线程卡住时，仍能独立保存崩溃记录。
                    if let Err(error) = append_log(&self.fallback, bytes, true) {
                        write_stderr_redacted(&format!("紧急崩溃日志写入失败：{error}\n"));
                    }
                    return;
                }
            }
        } else {
            self.state.lock().unwrap_or_else(|error| error.into_inner())
        };
        let destination = match append_log(primary, bytes, panic) {
            Ok(()) => {
                if state.failed {
                    let message = format!("{} INFO 日志文件写入已恢复\n", chrono::Local::now());
                    let _ = append_log(primary, message.as_bytes(), false);
                    state.failed = false;
                }
                primary
            }
            Err(error) => {
                if !state.failed {
                    let message = format!(
                        "{} ERROR 日志文件写入失败，尝试备用目录：{primary:?}：{error}\n",
                        chrono::Local::now()
                    );
                    write_stderr_redacted(&message);
                    let _ = append_log(&self.fallback, message.as_bytes(), false);
                }
                state.failed = true;
                if let Err(error) = append_log(&self.fallback, bytes, panic) {
                    write_stderr_redacted(&format!("备用日志写入失败：{error}\n"));
                }
                &self.fallback
            }
        };
        if panic {
            // 独立文件已同步落盘，普通日志中也保留完整错误入口。
            let _ = append_log(destination, bytes, false);
        }
        if let Some(counter) = self.dropped.get() {
            let dropped = counter.dropped_lines();
            if dropped > state.reported_dropped {
                let message = format!(
                    "{} WARN 普通日志队列已丢弃 {} 条记录，关键错误使用同步写入\n",
                    chrono::Local::now(),
                    dropped - state.reported_dropped
                );
                if append_log(destination, message.as_bytes(), false).is_ok() {
                    state.reported_dropped = dropped;
                }
            }
        }
        let today = chrono::Local::now().date_naive();
        if state
            .cleaned
            .as_ref()
            .is_none_or(|(path, date)| path != destination || *date != today)
        {
            if let Err(error) = cleanup_old_logs(destination, today) {
                let message = format!("{} WARN 清理过期日志失败：{error}\n", chrono::Local::now());
                write_stderr_redacted(&message);
                let _ = append_log(destination, message.as_bytes(), false);
            }
            state.cleaned = Some((destination.clone(), today));
        }
    }
}

struct DynamicLogWriter(Arc<LogFiles>);
impl Write for DynamicLogWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes, false);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct LogMakeWriter {
    ordinary: NonBlocking,
    files: Arc<LogFiles>,
}
struct EventWriter {
    bytes: Vec<u8>,
    ordinary: Option<NonBlocking>,
    files: Arc<LogFiles>,
}

impl<'a> MakeWriter<'a> for LogMakeWriter {
    type Writer = EventWriter;
    fn make_writer(&'a self) -> EventWriter {
        EventWriter {
            bytes: Vec::new(),
            ordinary: Some(self.ordinary.clone()),
            files: self.files.clone(),
        }
    }
    fn make_writer_for(&'a self, metadata: &tracing::Metadata<'_>) -> EventWriter {
        let mut writer = self.make_writer();
        if *metadata.level() <= tracing::Level::WARN {
            writer.ordinary = None;
        }
        writer
    }
}

impl Write for EventWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for EventWriter {
    fn drop(&mut self) {
        if self.bytes.is_empty() {
            return;
        }
        let content = String::from_utf8_lossy(&self.bytes);
        let redacted = redact_paths(&content);
        write_stderr_redacted(&redacted);
        if let Some(writer) = &mut self.ordinary {
            if let Err(error) = writer.write_all(redacted.as_bytes()) {
                write_stderr_redacted(&format!("提交普通日志失败：{error}\n"));
            }
        } else {
            self.files.write(redacted.as_bytes(), false);
        }
    }
}

fn install_panic_hook(files: Arc<LogFiles>) {
    std::panic::set_hook(Box::new(move |panic_info| {
        let message = format!(
            "{} ERROR 应用发生 panic，版本={}，进程={}，线程={:?}\n{panic_info}\n调用栈：\n{}\n",
            chrono::Local::now(),
            env!("CARGO_PKG_VERSION"),
            std::process::id(),
            std::thread::current().name(),
            Backtrace::force_capture()
        );
        files.write(message.as_bytes(), true);
        // 默认 panic hook 会再次输出未脱敏内容，因此统一使用本项目的输出入口。
        write_stderr_redacted(&message);
    }));
}

fn log_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER))
}

fn is_project_metadata(metadata: &tracing::Metadata<'_>) -> bool {
    metadata.target() == "QcClip" || metadata.target().starts_with("QcClip::")
}

fn redact_paths(content: &str) -> Cow<'_, str> {
    static PATTERNS: OnceLock<(regex::Regex, regex::Regex, regex::Regex)> = OnceLock::new();
    let (paths, error_codes, source_lines) = PATTERNS.get_or_init(|| {
        (
            regex::Regex::new(
                r#"(?mx)
            (?:
                (?P<windows_prefix>^|[\s="'(:,，：])
                (?:\\{2,}[?.]\\+(?:[A-Za-z]:[\\/]+)?|[A-Za-z]:[\\/]+|\\{2,}|(?i:file://))
                |(?P<unix_prefix>^|[\s="'(,，：])/
            )
            [^\r\n"]*"#,
            )
            .expect("日志路径脱敏规则无效"),
            regex::Regex::new(r"\(os error [0-9]+\)").expect("日志错误代码规则无效"),
            regex::Regex::new(r"(?P<location>:[0-9]+(?::[0-9]+)?):?\s*$")
                .expect("日志源码行号规则无效"),
        )
    });
    // 路径可含空格和标点；未加双引号时隐藏该行余下文本，仅保留可识别的诊断数字。
    paths.replace_all(content, |captures: &regex::Captures<'_>| {
        let prefix = captures
            .name("windows_prefix")
            .or_else(|| captures.name("unix_prefix"))
            .unwrap()
            .as_str();
        let matched = captures.get(0).unwrap().as_str();
        let mut replacement = format!("{prefix}<路径>");
        if let Some(location) = source_lines
            .captures(matched)
            .and_then(|value| value.name("location"))
        {
            replacement.push_str(location.as_str());
        }
        for code in error_codes.find_iter(matched) {
            replacement.push(' ');
            replacement.push_str(code.as_str());
        }
        replacement
    })
}

fn write_stderr_redacted(content: &str) {
    let _ = io::stderr()
        .lock()
        .write_all(redact_paths(content).as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_logs_are_excluded_even_with_verbose_environment_filter() {
        let root = tempfile::tempdir().unwrap();
        let files = Arc::new(test_files(root.path(), &root.path().join("fallback")));
        let (ordinary, worker) = tracing_appender::non_blocking(DynamicLogWriter(files.clone()));
        let subscriber = tracing_subscriber::registry()
            .with(EnvFilter::new("trace"))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_file(true)
                    .with_line_number(true)
                    .with_writer(LogMakeWriter {
                        ordinary,
                        files: files.clone(),
                    })
                    .with_filter(filter_fn(is_project_metadata)),
            );
        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..185 {
                tracing::error!(target: "log", { log.target = "gpui::window", log.file = "window.rs", log.line = 1768u64 },
                    "RefCell already borrowed");
            }
            for _ in 0..2 {
                tracing::error!(target: "gpui::window", "window not found");
                tracing::error!(target: "gpui::window", "其他窗口错误");
                tracing::error!(target: "gpui_ce_windows", "渲染设备失败");
                tracing::error!(target: "QcClip", "RefCell already borrowed");
            }
            tracing::debug!(target: "QcClip::app", "应用调试日志");
            tracing::error!(target: "QcClip_other", "相似名称依赖错误");
        });
        drop(worker);
        let content = fs::read_to_string(daily_path(root.path(), false)).unwrap();
        assert_eq!(content.matches("RefCell already borrowed").count(), 2);
        assert!(content.contains("应用调试日志"));
        for message in [
            "window not found",
            "其他窗口错误",
            "渲染设备失败",
            "相似名称依赖错误",
            "window.rs",
        ] {
            assert!(!content.contains(message));
        }
    }

    #[test]
    fn absolute_paths_are_redacted_across_platforms_and_debug_formats() {
        for path in [
            r"C:\Users\private-user\AppData\Local\QcClip\QcClipData",
            r"D:/private-user/剪贴板数据库/qcclip.sqlite3",
            r"C:\\Users\\private-user\\Documents\\private-file.txt",
            r"\\private-server\private-share\private-file.txt",
            r"\\?\C:\private-user\private-file.txt",
            r"\\\\?\\C:\\private-user\\private-file.txt",
            r"\\?\UNC\private-server\private-share\private-file.txt",
            "/home/private-user/private-file.txt",
            "/Users/private-user/private-file.txt",
            r"C:\Users\Private User\Private Folder\private-file.txt",
            r"C:\Users\O'Brien\(private-user), [private-folder]; private-file.txt",
            "/home/private-user/folder:name/私密, [报告].txt",
            "file:///C:/Users/private-user/private-file.txt",
            "file:///home/private-user/private-file.txt",
        ] {
            let message =
                format!("打开失败 path=\"{path}\" (os error 5)\n调用栈：src/logging.rs:123");
            let redacted = redact_paths(&message);
            assert!(
                !redacted.contains(path)
                    && !redacted.contains("private-")
                    && !redacted.contains("Private"),
                "{redacted}"
            );
            assert!(redacted.contains("path=\"<路径>\"") && redacted.contains("os error 5"));
            assert!(redacted.contains("src/logging.rs:123"));
            assert_eq!(redact_paths(&redacted), redacted);
        }
        let content =
            "2026-10-14T04:04:21Z QcClip::app: version=0.1.0 https://github.com/mosheng1/QcClip";
        assert_eq!(redact_paths(content), content);
        for path in [
            r"C:\Users\O'Brien\(private-user), [private-folder]; private-file.txt",
            "/home/private-user/folder:name/private-file.txt",
        ] {
            assert_eq!(
                redact_paths(&format!("路径={path} (os error 5)")),
                "路径=<路径> (os error 5)"
            );
        }
        assert_eq!(
            redact_paths(r"路径=C:\Private User\Private Folder\file.txt (os error 5)"),
            "路径=<路径> (os error 5)"
        );
        assert_eq!(
            redact_paths("at /home/private-user/project/src/main.rs:123:45"),
            "at <路径>:123:45"
        );
    }

    #[test]
    fn shared_copy_redacts_old_logs_without_modifying_original() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("qcclip-2026-10-14.log");
        let original = "旧日志 data_root=\"C:\\Users\\private-user\\Documents\"\n错误代码=5\n";
        fs::write(&source, original).unwrap();
        let shared = create_log_share_file(&source).unwrap();
        assert_ne!(shared, source);
        assert_eq!(fs::read_to_string(source).unwrap(), original);
        let content = fs::read_to_string(shared).unwrap();
        assert!(
            !content.contains("private-user")
                && content.contains("<路径>")
                && content.contains("错误代码=5")
        );
    }

    #[test]
    fn ordinary_critical_and_panic_files_only_receive_redacted_paths() {
        let root = tempfile::tempdir().unwrap();
        let files = Arc::new(test_files(root.path(), &root.path().join("fallback")));
        let (ordinary, worker) = tracing_appender::non_blocking(DynamicLogWriter(files.clone()));
        let subscriber = tracing_subscriber::registry()
            .with(EnvFilter::new(DEFAULT_FILTER))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(LogMakeWriter {
                        ordinary,
                        files: files.clone(),
                    })
                    .with_filter(filter_fn(is_project_metadata)),
            );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "QcClip::app", data_root = ?Path::new(r"C:\Users\private-user\private-folder"), "普通日志");
            tracing::error!(target: "QcClip::app", error = %r"打开 C:\Users\private-user\private-file.txt (os error 5)", "关键错误");
        });
        let critical = fs::read_to_string(daily_path(root.path(), false)).unwrap();
        assert!(critical.contains("关键错误") && !critical.contains("private-"));
        drop(worker);
        files.write(
            "panic 路径=\"/home/private-user/private-file.txt\"\n调用栈：src/logging.rs:123\n"
                .as_bytes(),
            true,
        );
        for panic in [false, true] {
            let content = fs::read_to_string(daily_path(root.path(), panic)).unwrap();
            assert!(!content.contains("private-") && content.contains("<路径>"));
            assert!(content.contains("调用栈：src/logging.rs:123"));
        }
        assert!(
            fs::read_to_string(daily_path(root.path(), false))
                .unwrap()
                .contains("普通日志")
        );
    }

    #[test]
    fn latest_log_uses_modification_time_and_ignores_unrelated_entries() {
        let directory = tempfile::tempdir().expect("无法创建测试目录");
        let older = directory.path().join("qcclip-2026-10-13.log");
        let latest = directory.path().join("qcclip-2026-10-12.panic.log");
        let start = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1000);
        for (path, modified) in [
            (&older, start),
            (&latest, start + std::time::Duration::from_secs(1)),
        ] {
            let file = std::fs::File::create(path).expect("无法创建测试日志");
            file.set_times(std::fs::FileTimes::new().set_modified(modified))
                .expect("无法设置日志修改时间");
        }
        std::fs::write(directory.path().join("other.log"), "其他日志").expect("无法写入测试文件");
        std::fs::write(directory.path().join("qcclip-invalid.log"), "无效日志")
            .expect("无法写入测试文件");
        std::fs::create_dir(directory.path().join("qcclip-2026-10-14.log"))
            .expect("无法创建测试子目录");
        assert_eq!(
            latest_log_file_in(directory.path()).expect("无法查找日志"),
            Some(latest)
        );
    }

    #[test]
    fn latest_log_returns_none_for_empty_or_missing_directory() {
        let directory = tempfile::tempdir().expect("无法创建测试目录");
        assert_eq!(
            latest_log_file_in(directory.path()).expect("无法查找日志"),
            None
        );
        assert_eq!(
            latest_log_file_in(&directory.path().join("missing")).expect("无法查找日志"),
            None
        );
    }

    #[test]
    fn log_file_prefix_is_stable() {
        assert_eq!(LOG_FILE_PREFIX, "qcclip");
        assert_eq!(LOG_FILE_SUFFIX, "log");
    }

    #[test]
    fn critical_events_are_saved_synchronously_and_dependencies_are_filtered() {
        let root = tempfile::tempdir().unwrap();
        let files = Arc::new(test_files(root.path(), &root.path().join("fallback")));
        let (ordinary, _guard) = tracing_appender::non_blocking(DynamicLogWriter(files.clone()));
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(DEFAULT_FILTER)
            .with_writer(LogMakeWriter { ordinary, files })
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!(target: "gpui_ce_windows", "渲染设备失败");
            tracing::warn!(target: "QcClip", "关键警告");
            tracing::error!(target: "QcClip::app", "关键错误");
            tracing::info!(target: "dependency", "不应出现的普通依赖日志");
        });
        let content = fs::read_to_string(daily_path(root.path(), false)).unwrap();
        assert!(content.contains("关键错误") && content.contains("关键警告"));
        assert!(!content.contains("渲染设备失败"));
        assert!(!content.contains("不应出现"));
    }

    fn test_files(primary: &Path, fallback: &Path) -> LogFiles {
        LogFiles {
            directory: Arc::new(RwLock::new(primary.to_path_buf())),
            fallback: fallback.to_path_buf(),
            state: Mutex::new(FileState::default()),
            dropped: OnceLock::new(),
        }
    }

    #[test]
    fn failed_primary_uses_fallback_and_recovers() {
        let root = tempfile::tempdir().unwrap();
        let primary = root.path().join("primary");
        let fallback = root.path().join("fallback");
        fs::write(&primary, "阻止创建目录").unwrap();
        let files = test_files(&primary, &fallback);
        files.write(b"first error\n", false);
        let content = fs::read_to_string(daily_path(&fallback, false)).unwrap();
        assert!(content.contains("日志文件写入失败") && content.contains("first error"));
        fs::remove_file(&primary).unwrap();
        files.write(b"second error\n", false);
        let content = fs::read_to_string(daily_path(&primary, false)).unwrap();
        assert!(content.contains("写入已恢复") && content.contains("second error"));
    }

    #[test]
    fn existing_directory_with_unwritable_log_uses_fallback() {
        let root = tempfile::tempdir().unwrap();
        let primary = root.path().join("primary");
        let fallback = root.path().join("fallback");
        fs::create_dir_all(daily_path(&primary, false)).unwrap();
        assert!(prepare_directory(&primary).is_err());
        let files = test_files(&primary, &fallback);
        files.write(b"file open error\n", false);
        files.write(b"second file open error\n", false);
        let content = fs::read_to_string(daily_path(&fallback, false)).unwrap();
        assert_eq!(content.matches("日志文件写入失败").count(), 1);
        assert!(content.contains("second file open error"));
    }

    #[test]
    fn critical_write_during_directory_switch_uses_fallback() {
        let root = tempfile::tempdir().unwrap();
        let files = test_files(&root.path().join("primary"), &root.path().join("fallback"));
        let _lock = files.directory.write().unwrap();
        files.write(b"reset error\n", false);
        files.write(b"panic stack\n", true);
        assert!(
            fs::read_to_string(daily_path(&files.fallback, false))
                .unwrap()
                .contains("reset error")
        );
        assert!(
            fs::read_to_string(daily_path(&files.fallback, true))
                .unwrap()
                .contains("panic stack")
        );
    }

    #[test]
    fn rotation_and_retention_preserve_unrelated_files() {
        let root = tempfile::tempdir().unwrap();
        let path = daily_path(root.path(), false);
        fs::File::create(&path)
            .unwrap()
            .set_len(MAX_LOG_BYTES)
            .unwrap();
        append_log(root.path(), b"new event", false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new event");
        assert!(path.with_extension("log.1").exists());
        fs::write(root.path().join("qcclip-2000-01-01.panic.log"), "旧崩溃").unwrap();
        fs::write(root.path().join("qcclip-2000-01-01.log.2"), "旧轮转").unwrap();
        fs::write(root.path().join("other.log"), "其他日志").unwrap();
        cleanup_old_logs(root.path(), chrono::Local::now().date_naive()).unwrap();
        assert!(!root.path().join("qcclip-2000-01-01.panic.log").exists());
        assert!(!root.path().join("qcclip-2000-01-01.log.2").exists());
        assert!(root.path().join("other.log").exists() && path.exists());
    }

    #[test]
    fn panic_child() {
        let Some(root) = std::env::var_os("QCCLIP_LOG_PANIC_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let files = Arc::new(test_files(&root.join("primary"), &root.join("fallback")));
        install_panic_hook(files.clone());
        let mode = std::env::var("QCCLIP_LOG_PANIC_TEST_MODE").unwrap();
        let _directory_lock = (mode == "directory").then(|| files.directory.write().unwrap());
        let _file_lock = (mode == "file").then(|| files.state.lock().unwrap());
        let _ = std::panic::catch_unwind(|| {
            panic!(
                "测试立即退出时保存崩溃调用栈，路径=\"C:\\Users\\panic-private-user\\private-file.txt\""
            )
        });
        // 故意绕过析构，验证 panic 文件不依赖退出时排空队列。
        std::process::exit(0);
    }

    #[test]
    fn panic_survives_immediate_exit_and_held_locks() {
        for mode in ["normal", "directory", "file"] {
            let root = tempfile::tempdir().unwrap();
            let stderr_path = root.path().join("stderr.log");
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "logging::tests::panic_child", "--nocapture"])
                .env("QCCLIP_LOG_PANIC_TEST_ROOT", root.path())
                .env("QCCLIP_LOG_PANIC_TEST_MODE", mode)
                .stdout(std::process::Stdio::null())
                .stderr(fs::File::create(&stderr_path).unwrap())
                .spawn()
                .unwrap();
            // 子进程需采集调用栈，完整测试并行时可能明显变慢；死锁会一直挂起，60 秒足以区分。
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success());
                    break;
                }
                if std::time::Instant::now() > deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("崩溃日志子进程超时，可能发生死锁：{mode}");
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let destination = root.path().join(if mode == "normal" {
                "primary"
            } else {
                "fallback"
            });
            let content = fs::read_to_string(daily_path(&destination, true)).unwrap();
            assert!(content.contains("测试立即退出时保存崩溃调用栈"));
            assert!(content.contains("调用栈") && content.contains("logging.rs"));
            assert!(
                !content.contains("panic-private-user") && !content.contains("private-file.txt")
            );
            let stderr = fs::read_to_string(stderr_path).unwrap();
            assert!(!stderr.contains("panic-private-user") && !stderr.contains("private-file.txt"));
            assert!(stderr.contains("<路径>") && stderr.contains("测试立即退出时保存崩溃调用栈"));
        }
    }

    #[test]
    fn queue_loss_is_reported_after_worker_recovers() {
        struct BlockedWriter {
            started: Option<std::sync::mpsc::Sender<()>>,
            release: std::sync::mpsc::Receiver<()>,
        }
        impl Write for BlockedWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if let Some(started) = self.started.take() {
                    started.send(()).unwrap();
                    self.release.recv().unwrap();
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let root = tempfile::tempdir().unwrap();
        let files = test_files(root.path(), &root.path().join("fallback"));
        let (started, wait_started) = std::sync::mpsc::channel();
        let (release, wait_release) = std::sync::mpsc::channel();
        let (mut writer, guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
            .buffered_lines_limit(1)
            .finish(BlockedWriter {
                started: Some(started),
                release: wait_release,
            });
        files.dropped.set(writer.error_counter()).unwrap();
        writer.write_all(b"first").unwrap();
        wait_started
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        writer.write_all(b"queued").unwrap();
        writer.write_all(b"dropped").unwrap();
        release.send(()).unwrap();
        drop(guard);
        files.write(b"recovered\n", false);
        assert!(
            fs::read_to_string(daily_path(root.path(), false))
                .unwrap()
                .contains("已丢弃 1 条记录")
        );
    }
}
