use std::{mem::size_of, sync::atomic::{AtomicU32, Ordering}};

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::{
            Input::{RAWINPUTDEVICE, RIDEV_INPUTSINK, RegisterRawInputDevices},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
                HWND_MESSAGE, MSG, PostThreadMessageW, RegisterClassW, WM_INPUT, WM_QUIT,
                WNDCLASSW,
            },
        },
    },
    core::w,
};

use crate::platform::{
    CoreError, CoreImpl, PlatformCore, PlatformListen,
    windows::common::{GLOBAL_HWND, IS_CORE_RUNNING},
};

/// Stores the ID of the thread running the message loop to allow remote shutdown.
static CORE_THREAD_ID: AtomicU32 = AtomicU32::new(0);

impl CoreImpl for PlatformCore {
    /// Starts the core engine and blocks the current thread with a Windows message loop.
    ///
    /// Because this function is blocking, you should typically call it in a dedicated thread.
    ///
    /// # Example
    ///
    /// ```
    /// use std::thread;
    /// use raw_input::Core;
    ///
    /// thread::spawn(|| {
    ///     if let Err(e) = Core::start() {
    ///         eprintln!("Core error: {:?}", e);
    ///     }
    /// });
    /// ```
    fn start() -> Result<(), CoreError> {
        if Self::is_run() {
            return Ok(());
        }

        let hwnd: HWND = match Self::setup_raw_input_window() {
            Ok(hwnd) => hwnd,
            Err(error) => {
                IS_CORE_RUNNING.store(false, Ordering::SeqCst);
                return Err(error);
            }
        };
        GLOBAL_HWND.store(hwnd.0, Ordering::SeqCst);

        unsafe {
            // Save current thread ID so stop() can send WM_QUIT to this thread
            CORE_THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);
        }

        let mut msg = MSG::default();
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                DispatchMessageW(&msg);
            }
        }

        Self::cleanup_window();
        CORE_THREAD_ID.store(0, Ordering::SeqCst);
        IS_CORE_RUNNING.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn is_runing() -> bool {
        IS_CORE_RUNNING.load(Ordering::SeqCst)
    }

    fn pause() {
        IS_CORE_RUNNING.store(false, Ordering::SeqCst);
    }

    fn resume() {
        IS_CORE_RUNNING.store(true, Ordering::SeqCst);
    }

    /// Stops the Raw Input message loop.
    fn stop() {
        Self::pause();
        let thread_id = CORE_THREAD_ID.load(Ordering::SeqCst);
        if thread_id != 0 {
            unsafe {
                let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
    }
}

impl PlatformCore {
    /// Atomic check-and-set to ensure the core starts only once.
    #[inline]
    fn is_run() -> bool {
        IS_CORE_RUNNING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
    }

    /// 创建隐藏消息窗口，注册鼠标和键盘 Raw Input 设备。
    fn setup_raw_input_window() -> Result<HWND, CoreError> {
        unsafe {
            let instance = GetModuleHandleW(None).unwrap_or_default();
            let class_name = w!("RawInputMouseDeltaClass");

            let wc = WNDCLASSW {
                lpfnWndProc: Some(raw_input_window_proc),
                hInstance: instance.into(),
                lpszClassName: class_name,
                ..Default::default()
            };
            RegisterClassW(&wc);

            // HWND_MESSAGE makes this a "Message-Only" window, invisible to the user
            let hwnd = CreateWindowExW(
                Default::default(),
                class_name,
                w!("RawInputMouseDeltaWindow"),
                Default::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|e| {
                CoreError::WindowsRegisterRawInputError(format!("CreateWindowExW failed: {:?}", e))
            })?;

            // RIDEV_INPUTSINK allows receiving input even when the window is not focused.
            let devices = [
                // Mouse (Usage: 0x02).
                RAWINPUTDEVICE {
                    usUsagePage: 0x01,
                    usUsage: 0x02,
                    dwFlags: RIDEV_INPUTSINK,
                    hwndTarget: hwnd,
                },
                // Keyboard (Usage: 0x06).
                RAWINPUTDEVICE {
                    usUsagePage: 0x01,
                    usUsage: 0x06,
                    dwFlags: RIDEV_INPUTSINK,
                    hwndTarget: hwnd,
                },
            ];

            RegisterRawInputDevices(&devices, size_of::<RAWINPUTDEVICE>() as u32).map_err(|e| {
                CoreError::WindowsRegisterRawInputError(format!("Registration failed: {:?}", e))
            })?;

            Ok(hwnd)
        }
    }

    fn cleanup_window() {
        let hwnd = GLOBAL_HWND.swap(std::ptr::null_mut(), Ordering::SeqCst);
        if !hwnd.is_null() {
            unsafe {
                let _ = DestroyWindow(HWND(hwnd));
            }
        }
    }
}

/// 隐藏消息窗口处理 WM_INPUT。
extern "system" fn raw_input_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_INPUT {
        let is_handle = PlatformListen::handle_raw_input(lparam);
        if is_handle {
            return LRESULT(0);
        }
    }

    // Pass unhandled messages to the default window procedure
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
