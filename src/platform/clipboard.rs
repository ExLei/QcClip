use std::{
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
        mpsc::Receiver,
    },
    time::{Duration, Instant},
};

use crate::domain::capture::CapturedClipboard;

#[cfg(target_os = "windows")]
mod transient;

pub type ClipboardEventReceiver = Receiver<Result<CapturedClipboard, String>>;

const SELF_WRITE_SUPPRESSION_TTL: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardPayload {
    pub format_kind: String,
    pub format_namespace: String,
    pub format_key: String,
    pub format_name: String,
    pub native_format_id: Option<u32>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct ClipboardWriteSuppressor {
    pending: Arc<Mutex<Vec<PendingClipboardWrite>>>,
    next_id: Arc<AtomicU64>,
}

impl Default for ClipboardWriteSuppressor {
    fn default() -> Self {
        Self {
            pending: Arc::new(Mutex::new(Vec::new())),
            next_id: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ClipboardWriteTicket {
    id: u64,
}

#[derive(Clone, Copy, Debug)]
struct PendingClipboardWrite {
    id: u64,
    start_sequence: u32,
    end_sequence: Option<u32>,
    expires_at: Instant,
}

impl ClipboardWriteSuppressor {
    fn begin_write(&self, start_sequence: u32) -> ClipboardWriteTicket {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let now = Instant::now();
        let mut pending = self.lock_pending();
        pending.retain(|write| write.expires_at > now);
        pending.push(PendingClipboardWrite {
            id,
            start_sequence,
            end_sequence: None,
            expires_at: now + SELF_WRITE_SUPPRESSION_TTL,
        });
        ClipboardWriteTicket { id }
    }

    fn complete_write(&self, ticket: ClipboardWriteTicket, end_sequence: u32) {
        let mut pending = self.lock_pending();
        if let Some(write) = pending.iter_mut().find(|write| write.id == ticket.id) {
            write.end_sequence = Some(end_sequence);
        }
    }

    fn cancel_write(&self, ticket: ClipboardWriteTicket) {
        let mut pending = self.lock_pending();
        pending.retain(|write| write.id != ticket.id);
    }

    fn suppress_event(&self, sequence: u32) -> bool {
        let now = Instant::now();
        let mut pending = self.lock_pending();
        pending.retain(|write| write.expires_at > now);
        let mut suppressed = false;
        pending.retain(|write| match write.end_sequence {
            Some(end_sequence) if sequence > end_sequence => false,
            Some(_) | None => {
                if sequence > write.start_sequence {
                    suppressed = true;
                }
                true
            }
        });
        suppressed
    }

    fn lock_pending(&self) -> MutexGuard<'_, Vec<PendingClipboardWrite>> {
        match self.pending.lock() {
            Ok(pending) => pending,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suppressor_ignores_multi_event_self_writes() {
        let suppressor = ClipboardWriteSuppressor::default();
        let ticket = suppressor.begin_write(10);
        assert!(suppressor.suppress_event(11));
        suppressor.complete_write(ticket, 12);

        assert!(suppressor.suppress_event(12));
        assert!(!suppressor.suppress_event(13));
    }

    #[test]
    fn suppressor_cancels_pending_write_on_failure() {
        let suppressor = ClipboardWriteSuppressor::default();
        let ticket = suppressor.begin_write(10);
        suppressor.cancel_write(ticket);

        assert!(!suppressor.suppress_event(11));
    }
}

#[cfg(target_os = "windows")]
mod windows_clipboard {
    use std::{
        mem,
        path::Path,
        ptr, slice,
        sync::mpsc::{self, Sender},
        thread,
        time::{Duration, Instant},
    };

    use clipboard_rs::{
        ClipboardHandler, ClipboardWatcher, ClipboardWatcherContext, WatcherShutdown,
    };
    use thiserror::Error;
    use windows::{
        Win32::{
            Foundation::{CloseHandle, GlobalFree, HANDLE, HGLOBAL, HWND},
            System::{
                DataExchange::{
                    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
                    GetClipboardFormatNameW, GetClipboardOwner, GetClipboardSequenceNumber,
                    OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
                },
                Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
                Threading::{
                    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
                    QueryFullProcessImageNameW,
                },
            },
            UI::{
                Shell::{DragQueryFileW, HDROP},
                WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
            },
        },
        core::{PCWSTR, PWSTR},
    };

    use super::transient::{ClipboardObservation, filter_events};
    use super::{ClipboardPayload, ClipboardWriteSuppressor};
    use crate::domain::capture::{
        CapturedClipboard, FormatKind, FormatRole, RawFormat, SourceApplication,
        classify_format_role, normalize_text_bytes,
    };

    use crate::platform::image_temp_file::{
        create_image_temp_file, hdrop_bytes, retain_image_temp_file,
    };

    const READ_ATTEMPTS: usize = 3;
    const READ_RETRY_DELAY: Duration = Duration::from_millis(20);
    const PASTE_DELAY: Duration = Duration::from_millis(60);

    fn clipboard_sequence_number() -> u32 {
        // SAFETY: GetClipboardSequenceNumber 不需要指针或外部内存，仅读取系统剪贴板序列号。
        unsafe { GetClipboardSequenceNumber() }
    }

    #[derive(Debug, Error)]
    pub enum SystemClipboardError {
        #[error("无法初始化系统剪贴板：{0}")]
        Initialize(String),
        #[error("无法读取系统剪贴板格式列表：{0}")]
        Enumerate(String),
    }

    pub struct WindowsClipboardSource;

    impl WindowsClipboardSource {
        pub fn new() -> Self {
            Self
        }

        pub fn capture_current(&self) -> Result<Option<CapturedClipboard>, SystemClipboardError> {
            // 枚举、原始格式和文件摘要共用一次打开，避免外部工具在读取中途替换内容。
            let clipboard = ClipboardGuard::open().map_err(SystemClipboardError::Enumerate)?;
            let native_formats = clipboard.formats();
            if should_ignore_clipboard(&native_formats, |format_id| {
                clipboard.read_buffer(format_id)
            }) {
                tracing::debug!("剪贴板忽略标记已跳过本次捕获");
                return Ok(None);
            }
            let source_application = source_application();
            let mut formats = Vec::with_capacity(native_formats.len());
            for (order, native_format) in native_formats.into_iter().enumerate() {
                let kind = classify_format(native_format.id, &native_format.name);
                let key = stable_format_key(native_format.id, &native_format.name);
                if classify_format_role(kind, &key, &native_format.name) == FormatRole::Internal {
                    continue;
                }
                let unsupported = uses_non_global_handle(native_format.id);
                let read_result = if unsupported {
                    None
                } else {
                    Some(clipboard.read_buffer(native_format.id))
                };

                let mut format = match read_result {
                    None => RawFormat::unsupported(
                        "windows",
                        key,
                        native_format.name,
                        kind,
                        order as u32,
                        "native_handle_unsupported",
                        "该格式使用平台句柄，当前不能保存为可恢复字节",
                    ),
                    Some(Ok(bytes)) => {
                        let bytes = normalize_text_bytes("windows", &key, &bytes).to_vec();
                        RawFormat::captured(
                            "windows",
                            key,
                            native_format.name,
                            kind,
                            bytes,
                            order as u32,
                            is_restorable(native_format.id, kind),
                        )
                    }
                    Some(Err(error)) => {
                        tracing::warn!(format_id = native_format.id, format_key = %key,
                            source = ?source_application, %error, "无法读取剪贴板格式");
                        RawFormat::failed(
                            "windows",
                            key,
                            native_format.name,
                            kind,
                            order as u32,
                            "clipboard_read_failed",
                            format!("无法读取该格式：{error}"),
                        )
                    }
                };
                format.native_format_id = Some(native_format.id);
                format.mime_type = mime_type(kind).map(str::to_owned);
                if native_format.id == 13 {
                    format.encoding = Some("utf-16le".to_owned());
                }
                formats.push(format);
            }

            let primary_format_identity = select_primary_identity(&formats);
            let preview_candidate = if formats.iter().any(|format| {
                format.format_kind == FormatKind::FileList && format.capture_state.has_data()
            }) {
                clipboard
                    .file_list_preview()
                    .map_err(|error| {
                        tracing::warn!(%error, "读取剪贴板文件列表摘要失败");
                        error
                    })
                    .ok()
            } else {
                None
            };

            Ok(Some(CapturedClipboard {
                formats,
                primary_format_identity,
                preview_candidate,
                source_application,
            }))
        }
    }

    struct ClipboardChangeHandler {
        source: WindowsClipboardSource,
        sender: Sender<Result<ClipboardObservation, String>>,
        suppressor: ClipboardWriteSuppressor,
    }

    impl ClipboardHandler for ClipboardChangeHandler {
        fn on_clipboard_change(&mut self) {
            let recordable = !self.suppressor.suppress_event(clipboard_sequence_number());
            let observed_at = Instant::now();
            let result = match self.source.capture_current() {
                // 自身写入仍参与还原比对，但不会进入历史记录或触发复制提示。
                Ok(Some(captured)) => Ok(ClipboardObservation {
                    captured,
                    recordable,
                    observed_at,
                }),
                Ok(None) => return,
                Err(error) => Err(error.to_string()),
            };
            if self.sender.send(result).is_err() {
                tracing::debug!("剪贴板监听接收端已关闭");
            }
        }
    }

    pub struct ClipboardMonitor {
        shutdown: Option<WatcherShutdown>,
        worker: Option<thread::JoinHandle<()>>,
        filter_worker: Option<thread::JoinHandle<()>>,
    }

    impl ClipboardMonitor {
        pub fn start(
            suppressor: ClipboardWriteSuppressor,
        ) -> Result<(Self, super::ClipboardEventReceiver), SystemClipboardError> {
            let source = WindowsClipboardSource::new();
            let initial = match source.capture_current() {
                Ok(initial) => initial,
                Err(error) => {
                    tracing::warn!(%error, "读取剪贴板初始内容失败，继续启动监听");
                    None
                }
            };
            let (sender, receiver) = mpsc::channel();
            let (observation_sender, observation_receiver) = mpsc::channel();
            let mut watcher =
                ClipboardWatcherContext::new_with_interval(Duration::from_millis(100))
                    .map_err(|error| SystemClipboardError::Initialize(error.to_string()))?;
            let shutdown = watcher
                .add_handler(ClipboardChangeHandler {
                    source,
                    sender: observation_sender,
                    suppressor,
                })
                .get_shutdown_channel();
            let filter_worker = thread::Builder::new()
                .name("qcclip-clipboard-filter".to_owned())
                .spawn(move || {
                    filter_events(observation_receiver, sender, initial);
                    tracing::debug!("剪贴板瞬时内容过滤线程已退出");
                })
                .map_err(|error| SystemClipboardError::Initialize(error.to_string()))?;
            let worker = match thread::Builder::new()
                .name("qcclip-clipboard-watcher".to_owned())
                .spawn(move || {
                    watcher.start_watch();
                    tracing::info!("剪贴板监听线程已退出");
                }) {
                Ok(worker) => worker,
                Err(error) => {
                    if filter_worker.join().is_err() {
                        tracing::error!("剪贴板瞬时内容过滤线程异常退出");
                    }
                    return Err(SystemClipboardError::Initialize(error.to_string()));
                }
            };
            Ok((
                Self {
                    shutdown: Some(shutdown),
                    worker: Some(worker),
                    filter_worker: Some(filter_worker),
                },
                receiver,
            ))
        }
    }

    impl Drop for ClipboardMonitor {
        fn drop(&mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                shutdown.stop();
            }
            if let Some(worker) = self.worker.take()
                && worker.join().is_err()
            {
                tracing::error!("剪贴板监听线程异常退出");
            }
            if let Some(worker) = self.filter_worker.take()
                && worker.join().is_err()
            {
                tracing::error!("剪贴板瞬时内容过滤线程异常退出");
            }
        }
    }

    pub fn paste_into_active_window(
        payloads: &[ClipboardPayload],
        suppressor: &ClipboardWriteSuppressor,
    ) -> Result<(), String> {
        if payloads.is_empty() {
            return Err("没有可恢复的剪贴板格式".to_owned());
        }
        let ticket = suppressor.begin_write(clipboard_sequence_number());
        if let Err(error) = set_native_payloads(payloads) {
            suppressor.cancel_write(ticket);
            return Err(error);
        }
        suppressor.complete_write(ticket, clipboard_sequence_number());
        thread::sleep(PASTE_DELAY);
        let result = send_ctrl_v();
        if result.is_ok() {
            crate::services::sound::play(crate::services::sound::SoundEvent::Paste);
        }
        result
    }

    pub fn copy_to_clipboard(
        payloads: &[ClipboardPayload],
        suppressor: &ClipboardWriteSuppressor,
    ) -> Result<(), String> {
        if payloads.is_empty() {
            return Err("没有可恢复的剪贴板格式".to_owned());
        }
        let ticket = suppressor.begin_write(clipboard_sequence_number());
        match set_native_payloads(payloads) {
            Ok(()) => {
                suppressor.complete_write(ticket, clipboard_sequence_number());
                Ok(())
            }
            Err(error) => {
                suppressor.cancel_write(ticket);
                Err(error)
            }
        }
    }

    fn set_native_payloads(payloads: &[ClipboardPayload]) -> Result<(), String> {
        // 非文件类型的图片在写入前生成临时文件，写入成功后保留一段时间供外部程序读取。
        let image_temp_file = create_image_temp_file(payloads)?;
        let _guard = ClipboardGuard::open()?;
        // SAFETY: 剪贴板已由 ClipboardGuard 打开，EmptyClipboard 只清空当前剪贴板内容。
        unsafe { EmptyClipboard() }.map_err(|error| format!("无法清空系统剪贴板：{error}"))?;

        let mut payloads = payloads.to_vec();
        if let Some(file) = image_temp_file.as_ref() {
            payloads.insert(
                0,
                ClipboardPayload {
                    format_kind: FormatKind::FileList.as_str().to_owned(),
                    format_namespace: "windows".to_owned(),
                    format_key: "CF_HDROP".to_owned(),
                    format_name: "CF_HDROP".to_owned(),
                    native_format_id: Some(15),
                    bytes: hdrop_bytes(file.path()),
                },
            );
        }

        for payload in &payloads {
            let format_id = clipboard_format_id(payload)?;
            let memory = GlobalMemory::new(&payload.bytes)?;
            // SAFETY: memory 是可移动的 HGLOBAL，SetClipboardData 成功后所有权转移给 Windows。
            match unsafe { SetClipboardData(format_id, Some(memory.as_handle())) } {
                Ok(_) => mem::forget(memory),
                Err(error) => {
                    return Err(format!(
                        "无法写入剪贴板格式：{} ({})：{error}",
                        payload.format_name, payload.format_key
                    ));
                }
            }
        }
        if let Some(file) = image_temp_file {
            retain_image_temp_file(file);
        }
        Ok(())
    }

    fn clipboard_format_id(payload: &ClipboardPayload) -> Result<u32, String> {
        match payload.format_key.as_str() {
            "CF_TEXT" => Ok(1),
            "CF_OEMTEXT" => Ok(7),
            "CF_DIB" => Ok(8),
            "CF_UNICODETEXT" => Ok(13),
            "CF_HDROP" => Ok(15),
            "CF_DIBV5" => Ok(17),
            "custom-text"
                if crate::domain::capture::is_custom_text_format(
                    &payload.format_namespace,
                    &payload.format_key,
                ) =>
            {
                Ok(13)
            }
            key if key.starts_with("registered:") => {
                let name = payload
                    .format_name
                    .encode_utf16()
                    .chain([0])
                    .collect::<Vec<_>>();
                // SAFETY: name 是以 NUL 结尾的本地 UTF-16 缓冲区，仅在调用期间使用。
                let format_id = unsafe { RegisterClipboardFormatW(PCWSTR(name.as_ptr())) };
                if format_id == 0 {
                    Err(format!("无法注册剪贴板格式：{}", payload.format_name))
                } else {
                    Ok(format_id)
                }
            }
            key if key.starts_with("native:") => key
                .strip_prefix("native:")
                .and_then(|value| value.parse::<u32>().ok())
                .or(payload.native_format_id)
                .ok_or_else(|| format!("原生剪贴板格式 ID 无效：{}", payload.format_key)),
            _ => payload.native_format_id.ok_or_else(|| {
                format!(
                    "当前格式暂不支持恢复：{} ({}, {})",
                    payload.format_name, payload.format_key, payload.format_kind
                )
            }),
        }
    }

    struct GlobalMemory {
        handle: HGLOBAL,
    }

    impl GlobalMemory {
        fn new(bytes: &[u8]) -> Result<Self, String> {
            let allocation_size = bytes.len().max(1);
            // SAFETY: GMEM_MOVEABLE 是 SetClipboardData 要求的全局内存类型。
            let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, allocation_size) }
                .map_err(|error| format!("无法分配剪贴板内存：{error}"))?;
            let pointer = unsafe { GlobalLock(handle) };
            if pointer.is_null() {
                let _ = unsafe { GlobalFree(Some(handle)) };
                return Err("无法锁定剪贴板全局内存".to_owned());
            }
            // SAFETY: pointer 指向刚分配的 allocation_size 字节内存，复制长度不超过该大小。
            unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len()) };
            let _ = unsafe { GlobalUnlock(handle) };
            Ok(Self { handle })
        }

        fn as_handle(&self) -> HANDLE {
            HANDLE(self.handle.0)
        }
    }

    impl Drop for GlobalMemory {
        fn drop(&mut self) {
            // SAFETY: 只有 SetClipboardData 失败或尚未转移所有权时才会进入这里。
            let _ = unsafe { GlobalFree(Some(self.handle)) };
        }
    }

    fn send_ctrl_v() -> Result<(), String> {
        crate::platform::input::simulate_ctrl_v();
        Ok(())
    }

    struct NativeFormat {
        id: u32,
        name: String,
    }

    fn should_ignore_clipboard(
        formats: &[NativeFormat],
        mut read_buffer: impl FnMut(u32) -> Result<Vec<u8>, String>,
    ) -> bool {
        // 排除标记只需存在，即使没有可读取的数据也应跳过整次捕获。
        if formats.iter().any(|format| {
            format
                .name
                .eq_ignore_ascii_case("ExcludeClipboardContentFromMonitorProcessing")
        }) {
            return true;
        }

        formats
            .iter()
            .filter(|format| {
                format
                    .name
                    .eq_ignore_ascii_case("CanIncludeInClipboardHistory")
            })
            .any(|format| match read_buffer(format.id) {
                // Windows 使用小端 DWORD，后续字节可能是全局内存分配时的填充。
                Ok(bytes) => bytes.get(..4).is_some_and(|value| value == [0; 4]),
                Err(error) => {
                    tracing::warn!(format_id = format.id, %error, "无法读取剪贴板历史控制标记，继续捕获");
                    false
                }
            })
    }

    struct ClipboardGuard;

    impl ClipboardGuard {
        fn open() -> Result<Self, String> {
            let mut last_error = None;
            for attempt in 0..READ_ATTEMPTS {
                match unsafe { OpenClipboard(None) } {
                    Ok(()) => return Ok(Self),
                    Err(error) => last_error = Some(error.to_string()),
                }
                if attempt + 1 < READ_ATTEMPTS {
                    thread::sleep(READ_RETRY_DELAY);
                }
            }
            Err(last_error.unwrap_or_else(|| "未知错误".to_owned()))
        }

        fn formats(&self) -> Vec<NativeFormat> {
            let mut formats = Vec::new();
            let mut current = 0_u32;
            loop {
                // SAFETY: 当前守卫持有剪贴板，格式枚举始终属于同一次打开。
                current = unsafe { EnumClipboardFormats(current) };
                if current == 0 {
                    break;
                }
                formats.push(NativeFormat {
                    id: current,
                    name: native_format_name(current),
                });
            }
            formats
        }

        fn read_buffer(&self, format_id: u32) -> Result<Vec<u8>, String> {
            // SAFETY: 剪贴板在守卫的整个生命周期内保持打开，句柄仅在此期间读取。
            let handle =
                unsafe { GetClipboardData(format_id) }.map_err(|error| error.to_string())?;
            let memory = HGLOBAL(handle.0);
            let byte_length = unsafe { GlobalSize(memory) };
            if byte_length == 0 {
                return Ok(Vec::new());
            }
            let pointer = unsafe { GlobalLock(memory) };
            if pointer.is_null() {
                return Err("无法锁定剪贴板全局内存".to_owned());
            }
            // SAFETY: pointer 指向已锁定的全局内存，长度由 GlobalSize 获取。
            let bytes =
                unsafe { slice::from_raw_parts(pointer.cast::<u8>(), byte_length) }.to_vec();
            let _ = unsafe { GlobalUnlock(memory) };
            Ok(bytes)
        }

        fn file_list_preview(&self) -> Result<String, String> {
            // SAFETY: CF_HDROP 句柄及其查询均在当前剪贴板守卫存活期间使用。
            let handle = unsafe { GetClipboardData(15) }.map_err(|error| error.to_string())?;
            let drop_handle = HDROP(handle.0);
            let count = unsafe { DragQueryFileW(drop_handle, u32::MAX, None) };
            if count == 0 {
                return Ok("0 个文件".to_owned());
            }
            let length = unsafe { DragQueryFileW(drop_handle, 0, None) };
            let mut buffer = vec![0_u16; length as usize + 1];
            let copied = unsafe { DragQueryFileW(drop_handle, 0, Some(&mut buffer)) };
            if copied == 0 {
                return Ok(format!("{count} 个文件"));
            }
            let path = String::from_utf16_lossy(&buffer[..copied as usize]);
            let name = Path::new(&path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&path);
            Ok(format!("{count} 个文件 · {name}"))
        }
    }

    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            let _ = unsafe { CloseClipboard() };
        }
    }

    fn native_format_name(format_id: u32) -> String {
        if let Some(name) = standard_format_name(format_id) {
            return name.to_owned();
        }
        let mut buffer = [0_u16; 256];
        let length = unsafe { GetClipboardFormatNameW(format_id, &mut buffer) };
        if length > 0 {
            String::from_utf16_lossy(&buffer[..length as usize])
        } else {
            format!("Windows 格式 {format_id}")
        }
    }

    fn standard_format_name(format_id: u32) -> Option<&'static str> {
        Some(match format_id {
            1 => "CF_TEXT",
            2 => "CF_BITMAP",
            3 => "CF_METAFILEPICT",
            4 => "CF_SYLK",
            5 => "CF_DIF",
            6 => "CF_TIFF",
            7 => "CF_OEMTEXT",
            8 => "CF_DIB",
            9 => "CF_PALETTE",
            10 => "CF_PENDATA",
            11 => "CF_RIFF",
            12 => "CF_WAVE",
            13 => "CF_UNICODETEXT",
            14 => "CF_ENHMETAFILE",
            15 => "CF_HDROP",
            16 => "CF_LOCALE",
            17 => "CF_DIBV5",
            0x80 => "CF_OWNERDISPLAY",
            0x81 => "CF_DSPTEXT",
            0x82 => "CF_DSPBITMAP",
            0x83 => "CF_DSPMETAFILEPICT",
            0x8e => "CF_DSPENHMETAFILE",
            _ => return None,
        })
    }

    fn stable_format_key(format_id: u32, name: &str) -> String {
        standard_format_name(format_id).map_or_else(
            || {
                if format_id >= 0xc000 {
                    format!("registered:{name}")
                } else {
                    format!("native:{format_id}")
                }
            },
            str::to_owned,
        )
    }

    fn classify_format(format_id: u32, name: &str) -> FormatKind {
        let lower_name = name.to_ascii_lowercase();
        match format_id {
            1 | 7 | 13 => FormatKind::Text,
            2 | 8 | 17 => FormatKind::Image,
            15 => FormatKind::FileList,
            _ if lower_name.contains("html") => FormatKind::Html,
            _ if lower_name.contains("rich text") || lower_name.contains("rtf") => FormatKind::Rtf,
            _ if lower_name == "png" || lower_name.contains("image/") => FormatKind::Image,
            _ if format_id >= 0xc000 => FormatKind::Custom,
            _ => FormatKind::Unknown,
        }
    }

    fn uses_non_global_handle(format_id: u32) -> bool {
        matches!(format_id, 2 | 3 | 9 | 14 | 0x80 | 0x82 | 0x83 | 0x8e)
    }

    fn is_restorable(format_id: u32, _kind: FormatKind) -> bool {
        !uses_non_global_handle(format_id)
    }

    fn mime_type(kind: FormatKind) -> Option<&'static str> {
        match kind {
            FormatKind::Text => Some("text/plain"),
            FormatKind::Html => Some("text/html"),
            FormatKind::Rtf => Some("text/rtf"),
            FormatKind::Image => Some("application/octet-stream"),
            FormatKind::FileList => Some("application/x-file-list"),
            FormatKind::Custom | FormatKind::Unknown => None,
        }
    }

    fn select_primary_identity(formats: &[RawFormat]) -> Option<(String, String)> {
        const KEY_PRIORITY: &[&str] = &[
            "CF_UNICODETEXT",
            "registered:HTML Format",
            "CF_HDROP",
            "registered:PNG",
            "CF_DIBV5",
            "CF_DIB",
            "CF_TEXT",
        ];
        let is_restorable = |format: &&RawFormat| {
            format.capture_state.has_data()
                && format.restore_state == crate::domain::capture::RestoreState::Restorable
        };
        if let Some(format) = KEY_PRIORITY.iter().find_map(|key| {
            formats
                .iter()
                .filter(is_restorable)
                .find(|format| format.format_key == *key)
        }) {
            return Some((format.format_namespace.clone(), format.format_key.clone()));
        }
        const PRIORITY: &[FormatKind] = &[
            FormatKind::Text,
            FormatKind::Html,
            FormatKind::FileList,
            FormatKind::Image,
            FormatKind::Rtf,
            FormatKind::Custom,
            FormatKind::Unknown,
        ];
        PRIORITY.iter().find_map(|kind| {
            formats
                .iter()
                .filter(is_restorable)
                .find(|format| format.format_kind == *kind)
                .map(|format| (format.format_namespace.clone(), format.format_key.clone()))
        })
    }

    fn source_application() -> SourceApplication {
        let path = source_path_with_fallback(source_process_path(), foreground_process_path());
        source_application_from_path(path)
    }

    fn source_path_with_fallback(
        owner_path: Option<String>,
        foreground_path: Option<String>,
    ) -> Option<String> {
        owner_path.or(foreground_path)
    }

    fn source_application_from_path(path: Option<String>) -> SourceApplication {
        let name = path.as_deref().and_then(|path| {
            Path::new(path)
                .file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        });
        SourceApplication { name, path }
    }

    fn source_process_path() -> Option<String> {
        let owner = unsafe { GetClipboardOwner() }.ok()?;
        process_path_from_window(owner)
    }

    fn foreground_process_path() -> Option<String> {
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.is_invalid() || foreground.0 as usize == 0 {
            return None;
        }
        process_path_from_window(foreground)
    }

    fn process_path_from_window(window: HWND) -> Option<String> {
        let mut process_id = 0_u32;
        unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
        if process_id == 0 {
            return None;
        }
        let process =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
        let mut buffer = vec![0_u16; 32_768];
        let mut length = buffer.len() as u32;
        let result = unsafe {
            QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut length,
            )
        };
        let _ = unsafe { CloseHandle(HANDLE(process.0)) };
        result.ok()?;
        Some(String::from_utf16_lossy(&buffer[..length as usize]))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn native_format(name: &str) -> NativeFormat {
            NativeFormat {
                id: 49_152,
                name: name.to_owned(),
            }
        }

        #[test]
        fn exclusion_marker_skips_capture_without_reading_marker_data() {
            for name in [
                "ExcludeClipboardContentFromMonitorProcessing",
                "excludeclipboardcontentfrommonitorprocessing",
            ] {
                let formats = [
                    native_format("CanIncludeInClipboardHistory"),
                    native_format(name),
                ];
                assert!(should_ignore_clipboard(&formats, |_| {
                    panic!("排除标记存在时不应读取剪贴板数据")
                }));
            }
        }

        #[test]
        fn history_marker_skips_capture_only_for_a_complete_zero_dword() {
            let formats = [native_format("CanIncludeInClipboardHistory")];
            for (bytes, should_ignore) in [
                (vec![0; 4], true),
                (vec![0, 0, 0, 0, 255], true),
                (vec![1, 0, 0, 0], false),
                (vec![0, 0, 0, 1], false),
                (vec![0; 3], false),
                (vec![], false),
            ] {
                assert_eq!(
                    should_ignore_clipboard(&formats, |format_id| {
                        assert_eq!(format_id, 49_152);
                        Ok(bytes.clone())
                    }),
                    should_ignore,
                    "历史控制标记值：{bytes:?}"
                );
            }
        }

        #[test]
        fn unreadable_history_marker_does_not_block_normal_capture() {
            let formats = [native_format("CanIncludeInClipboardHistory")];
            assert!(!should_ignore_clipboard(&formats, |_| {
                Err("标记数据不可读取".to_owned())
            }));
        }

        #[test]
        fn normal_formats_and_cloud_only_restrictions_do_not_skip_capture() {
            for formats in [
                vec![],
                vec![native_format("CF_UNICODETEXT")],
                vec![
                    native_format("CF_UNICODETEXT"),
                    native_format("CanUploadToCloudClipboard"),
                ],
            ] {
                assert!(!should_ignore_clipboard(&formats, |_| {
                    panic!("不应读取与本地历史过滤无关的格式")
                }));
            }
        }

        #[test]
        fn source_path_falls_back_to_foreground_application() {
            let path = source_path_with_fallback(None, Some(r"C:\前台程序.exe".to_owned()));
            let source = source_application_from_path(path);
            assert_eq!(source.name.as_deref(), Some("前台程序"));
            assert_eq!(source.path.as_deref(), Some(r"C:\前台程序.exe"));
        }

        #[test]
        fn clipboard_owner_path_takes_precedence_over_foreground_path() {
            assert_eq!(
                source_path_with_fallback(
                    Some(r"C:\所有者程序.exe".to_owned()),
                    Some(r"C:\前台程序.exe".to_owned()),
                ),
                Some(r"C:\所有者程序.exe".to_owned())
            );
        }

        #[test]
        fn windows_format_classification_and_stable_key_ignore_registration_id() {
            assert_eq!(classify_format(13, "CF_UNICODETEXT"), FormatKind::Text);
            assert_eq!(classify_format(49_152, "HTML Format"), FormatKind::Html);
            assert_eq!(stable_format_key(13, "CF_UNICODETEXT"), "CF_UNICODETEXT");
            assert_eq!(
                stable_format_key(49_152, "HTML Format"),
                "registered:HTML Format"
            );
        }

        #[test]
        fn handle_formats_are_not_read_as_regular_bytes() {
            assert!(uses_non_global_handle(2));
            assert!(uses_non_global_handle(14));
            assert!(!uses_non_global_handle(13));
        }

        #[test]
        fn unknown_native_formats_use_captured_format_id() {
            let payload = ClipboardPayload {
                format_kind: "unknown".to_owned(),
                format_namespace: "windows".to_owned(),
                format_key: "CF_LOCALE".to_owned(),
                format_name: "CF_LOCALE".to_owned(),
                native_format_id: Some(16),
                bytes: vec![0; 4],
            };
            assert_eq!(
                clipboard_format_id(&payload).expect("无法解析原生格式 ID"),
                16
            );
        }
    }
}

#[cfg(target_os = "windows")]
pub use windows_clipboard::{
    ClipboardMonitor, SystemClipboardError, WindowsClipboardSource, copy_to_clipboard,
    paste_into_active_window,
};

#[cfg(not(target_os = "windows"))]
mod unsupported {
    use thiserror::Error;

    use super::ClipboardEventReceiver;

    #[derive(Debug, Error)]
    pub enum SystemClipboardError {
        #[error("当前平台暂不支持系统剪贴板监听")]
        UnsupportedPlatform,
    }

    pub struct ClipboardMonitor;

    impl ClipboardMonitor {
        pub fn start(
            _suppressor: ClipboardWriteSuppressor,
        ) -> Result<(Self, ClipboardEventReceiver), SystemClipboardError> {
            Err(SystemClipboardError::UnsupportedPlatform)
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub use unsupported::{ClipboardMonitor, SystemClipboardError};

#[cfg(not(target_os = "windows"))]
pub fn paste_into_active_window(
    _payloads: &[ClipboardPayload],
    _suppressor: &ClipboardWriteSuppressor,
) -> Result<(), String> {
    Err("当前平台暂不支持恢复剪贴板并模拟粘贴".to_owned())
}

#[cfg(not(target_os = "windows"))]
pub fn copy_to_clipboard(
    _payloads: &[ClipboardPayload],
    _suppressor: &ClipboardWriteSuppressor,
) -> Result<(), String> {
    Err("当前平台暂不支持写回系统剪贴板".to_owned())
}
