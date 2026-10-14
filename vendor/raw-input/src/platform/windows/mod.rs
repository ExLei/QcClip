mod common;
mod core;
mod display;
mod keycode;
mod listen;
mod simulate;

// SendInput 会把该值带到 RAWKEYBOARD::ExtraInformation，用于区分程序注入的按键。
pub(crate) const SIMULATED_KEYBOARD_EXTRA_INFO: u32 = 0x5143_4C50;

pub(crate) use keycode::get_win_key;
