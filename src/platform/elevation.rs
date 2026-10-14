//! Windows 管理员权限检测与 UAC 重启。

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::{env, io, mem::size_of};

    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{
            GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    };
    use windows::core::PCWSTR;

    pub(crate) fn is_elevated() -> io::Result<bool> {
        let mut token = HANDLE::default();
        unsafe {
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
                .map_err(to_io_error)?;
        }

        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned_size = 0;
        let result = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned_size,
            )
            .map_err(to_io_error)
        };
        unsafe {
            let _ = CloseHandle(token);
        }
        result.map(|()| elevation.TokenIsElevated != 0)
    }

    pub(crate) fn relaunch_elevated() -> io::Result<()> {
        let executable = env::current_exe()?;
        let executable = wide_string(executable.as_os_str());
        let operation = wide_string("runas");
        let result = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(operation.as_ptr()),
                PCWSTR(executable.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if (result.0 as usize) <= 32 {
            return Err(io::Error::from_raw_os_error(result.0 as i32));
        }
        Ok(())
    }

    fn wide_string(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt as _;
        value.as_ref().encode_wide().chain(Some(0)).collect()
    }

    fn to_io_error(error: windows::core::Error) -> io::Error {
        io::Error::new(io::ErrorKind::Other, error.to_string())
    }
}

#[cfg(target_os = "windows")]
pub(crate) use windows_impl::{is_elevated, relaunch_elevated};

#[cfg(not(target_os = "windows"))]
pub(crate) fn is_elevated() -> std::io::Result<bool> {
    Ok(false)
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn relaunch_elevated() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "当前平台不支持管理员权限启动",
    ))
}
