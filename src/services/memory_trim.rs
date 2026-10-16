use std::time::Duration;

use gpui::{App, BorrowAppContext as _, Task};

use crate::windows::main::MainWindowService;

const MIN_DELAY_SECONDS: u64 = 10;
const MAX_DELAY_SECONDS: u64 = 30;
const PRIVATE_WORKING_SET_LIMIT: usize = 30 * 1024 * 1024;

pub(crate) struct State {
    pub(crate) task: Option<Task<()>>,
    pub(crate) startup_task: Option<Task<()>>,
    pub(crate) generation: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            task: None,
            startup_task: None,
            generation: 0,
        }
    }
}

pub(crate) fn schedule(cx: &mut App) {
    #[cfg(target_os = "windows")]
    {
        let generation = cx.update_global::<MainWindowService, _>(|service, _| {
            service.memory_trim_state.task = None;
            service.memory_trim_state.generation =
                service.memory_trim_state.generation.wrapping_add(1);
            service.memory_trim_state.generation
        });
        let delay = random_delay();
        let task = cx.spawn(async move |cx| {
            cx.background_executor().timer(delay).await;
            cx.update(|cx| {
                let should_check = cx.update_global::<MainWindowService, _>(|service, _| {
                    if service.memory_trim_state.generation != generation {
                        return false;
                    }
                    service.memory_trim_state.task = None;
                    true
                });
                if !should_check || MainWindowService::is_logically_visible(cx) {
                    return;
                }

                let Some(private_working_set) = private_working_set_size() else {
                    tracing::warn!("资源清理失败");
                    return;
                };
                if private_working_set <= PRIVATE_WORKING_SET_LIMIT {
                    return;
                }

                if empty_working_set().is_err() {
                    tracing::warn!("资源清理失败");
                }
            });
        });
        cx.update_global::<MainWindowService, _>(|service, _| {
            if service.memory_trim_state.generation == generation {
                service.memory_trim_state.task = Some(task);
            }
        });
    }

    #[cfg(not(target_os = "windows"))]
    let _ = cx;
}

pub(crate) fn schedule_startup_cleanup(cx: &mut App) {
    #[cfg(target_os = "windows")]
    {
        let task = cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            cx.update(|cx| {
                cx.update_global::<MainWindowService, _>(|service, _| {
                    service.memory_trim_state.startup_task = None;
                });
                if empty_working_set().is_err() {
                    tracing::warn!("资源清理失败");
                }
            });
        });
        cx.update_global::<MainWindowService, _>(|service, _| {
            service.memory_trim_state.startup_task = Some(task);
        });
    }

    #[cfg(not(target_os = "windows"))]
    let _ = cx;
}

pub(crate) fn cancel(cx: &mut App) {
    #[cfg(target_os = "windows")]
    cx.update_global::<MainWindowService, _>(|service, _| {
        service.memory_trim_state.task = None;
        service.memory_trim_state.generation = service.memory_trim_state.generation.wrapping_add(1);
    });

    #[cfg(not(target_os = "windows"))]
    let _ = cx;
}

#[cfg(target_os = "windows")]
fn random_delay() -> Duration {
    let span = MAX_DELAY_SECONDS - MIN_DELAY_SECONDS + 1;
    let offset = (uuid::Uuid::new_v4().as_u128() % u128::from(span)) as u64;
    Duration::from_secs(MIN_DELAY_SECONDS + offset)
}

#[cfg(target_os = "windows")]
fn private_working_set_size() -> Option<usize> {
    use std::mem::size_of;

    use windows::Win32::System::{
        ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
        },
        Threading::GetCurrentProcess,
    };

    let size = size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: size,
        ..Default::default()
    };
    // SAFETY: 伪句柄无需释放，缓冲区在调用期间有效且通过 cb 传入了准确大小。
    let result = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            std::ptr::from_mut(&mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
            size,
        )
    };
    result.ok().map(|_| counters.PrivateWorkingSetSize)
}

#[cfg(target_os = "windows")]
fn empty_working_set() -> windows_core::Result<()> {
    use windows::Win32::System::{ProcessStatus::EmptyWorkingSet, Threading::GetCurrentProcess};

    // SAFETY: 伪句柄表示当前进程，EmptyWorkingSet 只会移除可回收的工作集页面。
    unsafe { EmptyWorkingSet(GetCurrentProcess()) }
}
