#[cfg(target_os = "windows")]
use std::process::Command;
#[cfg(target_os = "windows")]
use std::time::Duration;

#[cfg(target_os = "windows")]
use winreg::RegKey;
#[cfg(target_os = "windows")]
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};

#[cfg(target_os = "windows")]
const EXPLORER_ADVANCED_PATH: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Advanced";
#[cfg(target_os = "windows")]
const DISABLED_HOTKEYS_VALUE: &str = "DisabledHotkeys";

#[cfg(target_os = "windows")]
pub(crate) fn disable_win_v_hotkey() -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (reg_key, _) = hkcu
        .create_subkey(EXPLORER_ADVANCED_PATH)
        .map_err(|error| format!("无法打开注册表项：{error}"))?;
    let current_value = read_disabled_hotkeys(&reg_key)?;

    if !contains_key(&current_value, 'V') {
        reg_key
            .set_value(DISABLED_HOTKEYS_VALUE, &format!("{current_value}V"))
            .map_err(|error| format!("无法设置注册表值：{error}"))?;
    }

    restart_explorer()
}

#[cfg(target_os = "windows")]
pub(crate) fn is_win_v_hotkey_disabled() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let reg_key = match hkcu.open_subkey_with_flags(EXPLORER_ADVANCED_PATH, KEY_READ) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(error) => {
            tracing::warn!(%error, "读取系统 Win+V 注册表项失败");
            return false;
        }
    };
    match read_disabled_hotkeys(&reg_key) {
        Ok(value) => contains_key(&value, 'V'),
        Err(error) => {
            tracing::warn!(%error, "读取系统 Win+V 状态失败");
            false
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn enable_win_v_hotkey() -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let reg_key = match hkcu.open_subkey_with_flags(EXPLORER_ADVANCED_PATH, KEY_READ | KEY_WRITE) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("无法打开系统 Win+V 注册表项：{error}")),
    };
    let current_value = read_disabled_hotkeys(&reg_key)?;
    let new_value: String = current_value
        .chars()
        .filter(|current| !current.eq_ignore_ascii_case(&'V'))
        .collect();

    if new_value == current_value {
        return Ok(());
    }
    if new_value.is_empty() {
        match reg_key.delete_value(DISABLED_HOTKEYS_VALUE) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("无法删除系统 Win+V 禁用设置：{error}")),
        }
    } else {
        reg_key
            .set_value(DISABLED_HOTKEYS_VALUE, &new_value)
            .map_err(|error| format!("无法恢复注册表值：{error}"))?;
    }

    restart_explorer()
}

#[cfg(target_os = "windows")]
fn read_disabled_hotkeys(key: &RegKey) -> Result<String, String> {
    match key.get_value(DISABLED_HOTKEYS_VALUE) {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("无法读取系统快捷键禁用设置：{error}")),
    }
}

#[cfg(target_os = "windows")]
fn contains_key(value: &str, key: char) -> bool {
    value
        .chars()
        .any(|current| current.eq_ignore_ascii_case(&key))
}

#[cfg(target_os = "windows")]
fn restart_explorer() -> Result<(), String> {
    let output = Command::new("taskkill")
        .args(["/F", "/IM", "explorer.exe"])
        .output()
        .map_err(|error| format!("无法执行 Explorer 停止命令：{error}"))?;
    if !output.status.success() {
        tracing::warn!(status = ?output.status.code(), "Explorer 停止命令未成功，Win+V 设置可能尚未生效");
    }
    std::thread::sleep(Duration::from_millis(1000));
    Command::new("explorer.exe")
        .spawn()
        .map_err(|error| format!("无法启动 Explorer：{error}"))?;
    std::thread::sleep(Duration::from_millis(1000));
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn disable_win_v_hotkey() -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn is_win_v_hotkey_disabled() -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn enable_win_v_hotkey() -> Result<(), String> {
    Ok(())
}
