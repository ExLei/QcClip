//! 使用系统默认程序打开文件、路径或 URL。

use std::path::Path;

#[cfg(target_os = "windows")]
fn shell_execute(target: &str, operation: &str) -> Result<(), String> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::PCWSTR;

    let operation: Vec<u16> = operation
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let file: Vec<u16> = target
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: operation/file 均为以 NUL 结尾的 UTF-16 缓冲区，ShellExecuteW 不持有指针。
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(operation.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW 返回值大于 32 表示成功，失败时返回错误码。
    if (result.0 as usize) <= 32 {
        return Err(format!("无法打开 {target}（错误码 {}）", result.0 as usize));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn open_with_default_program(path: &Path) -> Result<(), String> {
    shell_execute(&path.to_string_lossy(), "open")
}

#[cfg(not(target_os = "windows"))]
pub fn open_with_default_program(_path: &Path) -> Result<(), String> {
    Err("当前平台暂不支持使用默认程序打开".to_owned())
}
