//! Windows 当前用户登录自启。

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::{env, io, path::Path};

    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "QcClip";

    pub(crate) fn set_enabled(enabled: bool) -> io::Result<()> {
        let current_user = RegKey::predef(HKEY_CURRENT_USER);
        let (run_key, _) = current_user.create_subkey(RUN_KEY_PATH)?;
        if enabled {
            let executable = env::current_exe()?;
            run_key.set_value(VALUE_NAME, &startup_command(&executable))
        } else {
            match run_key.delete_value(VALUE_NAME) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            }
        }
    }

    fn startup_command(executable: &Path) -> String {
        format!(r#""{}""#, executable.display())
    }
}

#[cfg(target_os = "windows")]
pub(crate) use windows_impl::set_enabled;

#[cfg(not(target_os = "windows"))]
pub(crate) fn set_enabled(_enabled: bool) -> std::io::Result<()> {
    Ok(())
}
