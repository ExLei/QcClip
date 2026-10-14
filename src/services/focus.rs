use gpui::Window;

#[cfg(target_os = "windows")]
use std::sync::Mutex;

#[cfg(target_os = "windows")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::HWND,
    System::Threading::GetCurrentProcessId,
    UI::WindowsAndMessaging::{
        GA_ROOT, GWL_EXSTYLE, GetAncestor, GetClassNameW, GetForegroundWindow, GetWindowLongPtrW,
        GetWindowThreadProcessId, IsWindow, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
        WS_EX_NOACTIVATE,
    },
};

#[cfg(target_os = "windows")]
static FOCUS_SESSION: Mutex<Option<FocusSession>> = Mutex::new(None);

#[cfg(target_os = "windows")]
#[derive(Clone, Copy)]
struct FocusSession {
    target_window: Option<isize>,
    input_window: isize,
}

/// 在输入框获得焦点前保存当前前台窗口，并同步激活输入框所属窗口。
pub fn focus_input_window(window: &Window) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        focus_input_window_windows(window)?;
    }

    #[cfg(not(target_os = "windows"))]
    window.activate_window();

    Ok(())
}

/// 将焦点恢复到进入输入状态前保存的窗口，并恢复当前窗口的工具窗口行为。
pub fn restore_previous_focus() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let saved_focus = *FOCUS_SESSION
            .lock()
            .map_err(|_| "读取之前的窗口焦点失败".to_owned())?;
        let Some(saved_focus) = saved_focus else {
            return Ok(());
        };

        // 输入窗口已经不是前台窗口时，说明系统焦点已交给其他窗口，不能再抢回旧目标。
        if current_foreground_window() != Some(saved_focus.input_window) {
            return clear_previous_focus();
        }

        if let Some(target_window) = saved_focus.target_window {
            if !is_valid_window(target_window) {
                return clear_previous_focus();
            }
            activate_native_window(target_window)?;
        }

        set_window_non_activating(saved_focus.input_window, true)?;
        FOCUS_SESSION
            .lock()
            .map_err(|_| "清理已恢复的窗口焦点失败".to_owned())?
            .take();
    }

    Ok(())
}

/// 恢复 Raw Input 按下时捕获的外部窗口焦点，不保存焦点状态。
pub fn restore_captured_focus(target_window: Option<isize>) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        if let Some(target_window) = target_window.filter(|window| is_external_window(*window)) {
            activate_native_window(target_window)?;
        }
    }

    Ok(())
}

/// 丢弃当前输入会话保存的焦点，不抢回系统已经交给其他窗口的焦点。
pub fn clear_previous_focus() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let saved_focus = FOCUS_SESSION
            .lock()
            .map_err(|_| "清理之前的窗口焦点失败".to_owned())?
            .take();
        if let Some(saved_focus) = saved_focus
            && is_valid_window(saved_focus.input_window)
        {
            set_window_non_activating(saved_focus.input_window, true)?;
        }
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn focus_input_window_windows(window: &Window) -> Result<(), String> {
    let input_window = current_window_handle(window)
        .ok_or_else(|| "无法获取输入窗口句柄，无法同步窗口焦点".to_owned())?;
    if !is_valid_window(input_window) {
        return Err("输入窗口句柄已失效，无法同步窗口焦点".to_owned());
    }

    let previous_input_window = {
        let focus_session = FOCUS_SESSION
            .lock()
            .map_err(|_| "读取当前窗口焦点会话失败".to_owned())?;
        focus_session.map(|session| session.input_window)
    };

    if let Some(previous_input_window) = previous_input_window
        && previous_input_window != input_window
        && is_valid_window(previous_input_window)
    {
        set_window_non_activating(previous_input_window, true)?;
    }

    let new_session = previous_input_window != Some(input_window);
    if new_session {
        let target_window = current_foreground_window()
            .filter(|window| *window != input_window)
            .filter(|window| is_valid_window(*window));
        *FOCUS_SESSION
            .lock()
            .map_err(|_| "保存之前的窗口焦点失败".to_owned())? = Some(FocusSession {
            target_window,
            input_window,
        });
    }

    if let Err(error) = set_window_non_activating(input_window, false)
        .and_then(|_| activate_native_window(input_window))
    {
        if new_session {
            let _ = clear_previous_focus();
        }
        return Err(error);
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn activate_native_window(window: isize) -> Result<(), String> {
    let hwnd = hwnd_from_value(window);
    // SAFETY: hwnd 来自当前窗口或之前读取的前台窗口，并已通过 IsWindow 验证。
    let activated = unsafe { SetForegroundWindow(hwnd).as_bool() };
    if activated || current_foreground_window() == Some(window) {
        Ok(())
    } else {
        Err("激活窗口失败".to_owned())
    }
}

#[cfg(target_os = "windows")]
fn set_window_non_activating(window: isize, non_activating: bool) -> Result<(), String> {
    let hwnd = hwnd_from_value(window);
    if !is_valid_window(window) {
        return Ok(());
    }

    // SAFETY: hwnd 已通过 IsWindow 验证，只读取当前窗口的扩展样式。
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    let updated_style = if non_activating {
        ex_style | WS_EX_NOACTIVATE.0 as isize
    } else {
        ex_style & !(WS_EX_NOACTIVATE.0 as isize)
    };
    if updated_style == ex_style {
        return Ok(());
    }

    // SAFETY: hwnd 已验证，只切换当前窗口的 WS_EX_NOACTIVATE 样式，不改变位置和层级。
    unsafe {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, updated_style);
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
        )
        .map_err(|error| format!("更新窗口激活样式失败：{error}"))?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn is_valid_window(window: isize) -> bool {
    // SAFETY: 这里只验证系统窗口句柄是否仍有效。
    unsafe { IsWindow(Some(hwnd_from_value(window))).as_bool() }
}

#[cfg(target_os = "windows")]
fn is_external_window(window: isize) -> bool {
    if !is_valid_window(window) {
        return false;
    }

    let hwnd = hwnd_from_value(window);
    if is_current_process_window(window) {
        return false;
    }

    !is_shell_window(hwnd)
}

#[cfg(target_os = "windows")]
fn is_current_process_window(window: isize) -> bool {
    let hwnd = hwnd_from_value(window);
    let mut process_id = 0;
    if unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) } == 0 {
        return false;
    }
    process_id == unsafe { GetCurrentProcessId() }
}

#[cfg(target_os = "windows")]
fn is_shell_window(hwnd: HWND) -> bool {
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    let hwnd = if root.0.is_null() { hwnd } else { root };
    let mut class_name = [0u16; 128];
    let length = unsafe { GetClassNameW(hwnd, &mut class_name) };
    if length <= 0 {
        return false;
    }
    let class_name = String::from_utf16_lossy(&class_name[..length as usize]);
    matches!(
        class_name.as_str(),
        "#32768"
            | "Progman"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "TrayNotifyWnd"
            | "NotifyIconOverflowWindow"
            | "WorkerW"
    )
}

#[cfg(target_os = "windows")]
fn current_foreground_window() -> Option<isize> {
    // SAFETY: GetForegroundWindow 只读取系统当前前台窗口句柄。
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.0.is_null()).then_some(hwnd.0 as isize)
}

#[cfg(target_os = "windows")]
fn current_window_handle(window: &Window) -> Option<isize> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };
    Some(handle.hwnd.get())
}

#[cfg(target_os = "windows")]
fn hwnd_from_value(value: isize) -> HWND {
    HWND(value as *mut _)
}
