use std::{ffi::c_void, mem::size_of, sync::atomic::Ordering};

use windows::Win32::{
    Foundation::LPARAM,
    UI::{
        Input::{
            GetRawInputData, HRAWINPUT, MOUSE_MOVE_ABSOLUTE, RAWINPUT, RAWINPUTHEADER, RID_INPUT,
            RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
        },
        WindowsAndMessaging::{
            RI_MOUSE_BUTTON_1_DOWN, RI_MOUSE_BUTTON_1_UP, RI_MOUSE_BUTTON_2_DOWN,
            RI_MOUSE_BUTTON_2_UP, RI_MOUSE_BUTTON_3_DOWN, RI_MOUSE_BUTTON_3_UP,
            RI_MOUSE_BUTTON_4_DOWN, RI_MOUSE_BUTTON_4_UP, RI_MOUSE_BUTTON_5_DOWN,
            RI_MOUSE_BUTTON_5_UP, RI_MOUSE_HWHEEL, RI_MOUSE_WHEEL, RI_KEY_BREAK, RI_KEY_E0,
            RI_KEY_E1,
        },
    },
};

use crate::{
    dispatcher::{CALLBACKS, NEXT_ID, Status, Subscriber, dispatch, remove_all},
    event::{Event, MouseButton, Point},
    platform::{
        ListenImpl, PlatformListen,
        windows::common::{
            IS_LISTEN_RUNNING, LISTEN_FLAG, LISTEN_KEYBOARD, LISTEN_MOUSE_BUTTON,
            LISTEN_MOUSE_MOVE, LISTEN_MOUSE_WHEEL, LISTENS_ALL, update_state,
        },
        windows::{SIMULATED_KEYBOARD_EXTRA_INFO, get_win_key},
    },
    subscription::SubscriptionHandle,
};

impl ListenImpl for PlatformListen {
    fn start() {
        if Self::is_run() {
            return;
        }

        LISTEN_FLAG.store(LISTENS_ALL, Ordering::SeqCst);
    }

    fn is_runing() -> bool {
        IS_LISTEN_RUNNING.load(Ordering::SeqCst)
    }

    fn pause() {
        IS_LISTEN_RUNNING.store(false, Ordering::SeqCst);
    }

    fn resume() {
        IS_LISTEN_RUNNING.store(true, Ordering::SeqCst);
    }

    fn stop() {
        LISTEN_FLAG.store(0, Ordering::SeqCst);
        Self::pause();
        Self::unsubscribe_all();
    }

    fn mouse_move(enable: bool) {
        update_state(&LISTEN_FLAG, LISTEN_MOUSE_MOVE, enable);
    }

    fn mouse_wheel(enable: bool) {
        update_state(&LISTEN_FLAG, LISTEN_MOUSE_WHEEL, enable);
    }

    fn mouse_button(enable: bool) {
        update_state(&LISTEN_FLAG, LISTEN_MOUSE_BUTTON, enable);
    }

    fn keyboard(enable: bool) {
        update_state(&LISTEN_FLAG, LISTEN_KEYBOARD, enable);
    }

    fn subscribe<F>(callback: F) -> SubscriptionHandle
    where
        F: Fn(Event) + Send + Sync + 'static,
    {
        let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        CALLBACKS.insert(
            id,
            Subscriber {
                status: Status::Active,
                callback: Box::new(callback),
            },
        );
        SubscriptionHandle { id }
    }

    fn unsubscribe_all() {
        remove_all();
    }
}

impl PlatformListen {
    fn is_run() -> bool {
        IS_LISTEN_RUNNING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
    }

    pub(crate) fn handle_raw_input(lparam: LPARAM) -> bool {
        if !IS_LISTEN_RUNNING.load(Ordering::Relaxed) {
            return false;
        }

        let state = LISTEN_FLAG.load(Ordering::Relaxed);
        if state == 0 {
            return false;
        }

        let Some(raw) = raw_input_from_message(lparam) else {
            return false;
        };
        match raw.header.dwType {
            value if value == RIM_TYPEMOUSE.0 => handle_mouse(raw, state),
            value if value == RIM_TYPEKEYBOARD.0 => handle_keyboard(raw, state),
            _ => return false,
        }
        true
    }
}

fn raw_input_from_message(lparam: LPARAM) -> Option<RAWINPUT> {
    let h_raw_input = HRAWINPUT(lparam.0 as *mut c_void);
    let mut raw = RAWINPUT::default();
    let mut size = size_of::<RAWINPUT>() as u32;
    let raw_size = unsafe {
        GetRawInputData(
            h_raw_input,
            RID_INPUT,
            Some(&mut raw as *mut _ as *mut _),
            &mut size,
            size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    (raw_size != u32::MAX && raw_size != 0).then_some(raw)
}

fn handle_mouse(raw: RAWINPUT, state: u32) {
    let mouse = unsafe { raw.data.mouse };

    if state & LISTEN_MOUSE_MOVE != 0 && mouse.usFlags.0 & MOUSE_MOVE_ABSOLUTE.0 == 0 {
        let dx = mouse.lLastX as f64;
        let dy = mouse.lLastY as f64;
        if dx != 0.0 || dy != 0.0 {
            dispatch(Event::MouseMove {
                delta: Point { x: dx, y: dy },
            });
        }
    }

    let button_data = unsafe { mouse.Anonymous.Anonymous };
    let button_flags = button_data.usButtonFlags as u32;
    if state & LISTEN_MOUSE_BUTTON != 0 {
        dispatch_button(button_flags, RI_MOUSE_BUTTON_1_DOWN, RI_MOUSE_BUTTON_1_UP, MouseButton::Left);
        dispatch_button(button_flags, RI_MOUSE_BUTTON_2_DOWN, RI_MOUSE_BUTTON_2_UP, MouseButton::Right);
        dispatch_button(button_flags, RI_MOUSE_BUTTON_3_DOWN, RI_MOUSE_BUTTON_3_UP, MouseButton::Middle);
        dispatch_button(button_flags, RI_MOUSE_BUTTON_4_DOWN, RI_MOUSE_BUTTON_4_UP, MouseButton::Back);
        dispatch_button(button_flags, RI_MOUSE_BUTTON_5_DOWN, RI_MOUSE_BUTTON_5_UP, MouseButton::Forward);
    }

    if state & LISTEN_MOUSE_WHEEL != 0 {
        if button_flags & RI_MOUSE_WHEEL != 0 {
            dispatch(Event::MouseWheel {
                delta: Point {
                    x: 0.0,
                    y: button_data.usButtonData as i16 as f64 / 120.0,
                },
            });
        }
        if button_flags & RI_MOUSE_HWHEEL != 0 {
            dispatch(Event::MouseWheel {
                delta: Point {
                    x: button_data.usButtonData as i16 as f64 / 120.0,
                    y: 0.0,
                },
            });
        }
    }
}

fn handle_keyboard(raw: RAWINPUT, state: u32) {
    if state & LISTEN_KEYBOARD == 0 {
        return;
    }

    // SAFETY: RAWINPUT 的类型已确认是键盘，联合体中的 keyboard 字段有效。
    let keyboard = unsafe { raw.data.keyboard };
    if keyboard.ExtraInformation == SIMULATED_KEYBOARD_EXTRA_INFO {
        return;
    }
    let flags = keyboard.Flags as u32;
    let scan_code = match flags & (RI_KEY_E0 | RI_KEY_E1) {
        RI_KEY_E0 => 0xE000 | u32::from(keyboard.MakeCode),
        RI_KEY_E1 => 0xE100 | u32::from(keyboard.MakeCode),
        _ => u32::from(keyboard.MakeCode),
    };
    let key = get_win_key(u32::from(keyboard.VKey), scan_code);
    let event = if flags & RI_KEY_BREAK != 0 {
        Event::KeyUp {
            key,
            code: Some(u32::from(keyboard.VKey)),
        }
    } else {
        Event::KeyDown {
            key,
            code: Some(u32::from(keyboard.VKey)),
        }
    };
    dispatch(event);
}

fn dispatch_button(flags: u32, down_flag: u32, up_flag: u32, button: MouseButton) {
    if flags & down_flag != 0 {
        dispatch(Event::MouseDown { button });
    }
    if flags & up_flag != 0 {
        dispatch(Event::MouseUp { button });
    }
}
