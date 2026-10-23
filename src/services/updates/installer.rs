use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::download::CachedUpdate;

pub(crate) const UPDATE_ARGUMENT: &str = "--apply-update";

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct InstallJob {
    pub parent_pid: u32,
    pub executable: PathBuf,
    pub cache_directory: PathBuf,
    pub cached: CachedUpdate,
}

pub(super) struct PreparedInstaller {
    pub child: Child,
    pub directory: PathBuf,
}

pub(super) fn prepare(
    cached: CachedUpdate,
    cache_directory: PathBuf,
) -> Result<PreparedInstaller, String> {
    if !cfg!(target_os = "windows") {
        return Err("当前平台暂不支持自动安装".into());
    }
    cached.verify(&cache_directory)?;
    let executable = std::env::current_exe().map_err(|e| format!("读取程序路径失败：{e}"))?;
    let source = executable.parent().ok_or("程序路径无效")?;
    cleanup_workers(&cache_directory);
    let directory = tempfile::Builder::new()
        .prefix(".worker-")
        .tempdir_in(&cache_directory)
        .map_err(|e| format!("创建更新进程目录失败：{e}"))?;
    let helper = directory.path().join("QcClip-updater.exe");
    fs::copy(&executable, &helper).map_err(|e| format!("准备更新进程失败：{e}"))?;
    for name in ["vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll"] {
        if source.join(name).is_file() {
            fs::copy(source.join(name), directory.path().join(name))
                .map_err(|e| format!("复制更新进程运行库失败：{e}"))?;
        }
    }
    let job = InstallJob {
        parent_pid: std::process::id(),
        executable,
        cache_directory,
        cached,
    };
    let job_path = directory.path().join("job.json");
    fs::write(
        &job_path,
        serde_json::to_vec(&job).map_err(|e| format!("编码更新任务失败：{e}"))?,
    )
    .map_err(|e| format!("保存更新任务失败：{e}"))?;
    let mut command = Command::new(helper);
    command.arg(UPDATE_ARGUMENT).arg(&job_path);
    hide_console(&mut command);
    let child = command
        .spawn()
        .map_err(|e| format!("启动更新进程失败：{e}"))?;
    Ok(PreparedInstaller {
        child,
        directory: directory.keep(),
    })
}

pub(super) fn cleanup_workers(directory: &Path) {
    let Ok(root) = directory.canonicalize() else {
        return;
    };
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(".worker-") {
            continue;
        }
        let Ok(metadata) = entry.path().symlink_metadata() else {
            continue;
        };
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata
                .modified()
                .ok()
                .and_then(|time| time.elapsed().ok())
                .is_none_or(|age| age <= Duration::from_secs(86400))
        {
            continue;
        }
        let Ok(path) = entry.path().canonicalize() else {
            continue;
        };
        if path.parent() == Some(root.as_path())
            && path.join("job.json").is_file()
            && let Err(error) = fs::remove_dir_all(&path)
        {
            tracing::debug!("清理旧更新进程失败：{error}");
        }
    }
}

pub(crate) fn hide_console(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = command;
}

pub(crate) fn apply(
    job: &InstallJob,
    directory: &Path,
    mut progress: impl FnMut(String, Option<f32>),
) -> Result<(), String> {
    job.cached.verify(&job.cache_directory)?;
    progress("正在等待主程序保存数据并退出…".into(), None);
    wait_for_parent(job.parent_pid, directory)?;
    let base_dirs = directories::BaseDirs::new().ok_or("无法确定运行目录")?;
    let _instance = crate::platform::single_instance::SingleInstance::acquire_at(
        &base_dirs.data_local_dir().join("QcClip/runtime"),
        Duration::from_secs(5),
    )
    .map_err(|e| format!("锁定更新期间的程序实例失败：{e}"))?
    .ok_or("另一个程序实例正在运行，请退出后重试")?;
    let target = job.executable.parent().ok_or("安装目录无效")?;
    if job.cached.portable {
        apply_portable(
            &job.cached.path(&job.cache_directory),
            target,
            &mut progress,
        )?;
    } else {
        progress("正在安装新版本，请稍候…（NSIS 自动安装）".into(), None);
        let mut command = Command::new(job.cached.path(&job.cache_directory));
        // 被动模式直接显示安装进度并自动关闭，/D 必须是最后一个未加引号的参数。
        command.arg("/P").arg("/NS");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.raw_arg(format!("/D={}", target.display()));
        }
        #[cfg(not(windows))]
        command.arg(format!("/D={}", target.display()));
        hide_console(&mut command);
        let status = command
            .status()
            .map_err(|e| format!("运行安装程序失败：{e}"))?;
        if !status.success() {
            return Err(format!(
                "安装程序未成功完成（退出码：{status}），可重启原程序后重试"
            ));
        }
    }
    progress("安装完成，正在重新启动 QcClip…".into(), Some(1.0));
    restart(&job.executable)?;
    // 只有成功安装且已启动新版本，才清理已消费的下载缓存。
    let _ = fs::remove_file(job.cache_directory.join("ready.json"));
    let _ = fs::remove_file(job.cached.path(&job.cache_directory));
    Ok(())
}

pub(crate) fn restart(executable: &Path) -> Result<(), String> {
    let mut command = Command::new(executable);
    command.arg(crate::platform::single_instance::WAIT_FOR_INSTANCE_ARGUMENT);
    if let Some(parent) = executable.parent() {
        command.current_dir(parent);
    }
    hide_console(&mut command);
    command
        .spawn()
        .map_err(|e| format!("重新启动失败，请手动启动 QcClip：{e}"))?;
    Ok(())
}

fn wait_for_parent(pid: u32, directory: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        // 在握手前打开进程句柄，避免 PID 在等待期间被复用。
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }
            .map_err(|e| format!("无法等待主程序退出：{e}"))?;
        let result = (|| {
            fs::write(directory.join("ready"), b"ready")
                .map_err(|e| format!("确认更新进程启动失败：{e}"))?;
            let started = Instant::now();
            while !directory.join("proceed").exists() {
                if started.elapsed() > Duration::from_secs(45) {
                    return Err("主程序未确认安装，更新已停止".into());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if unsafe { WaitForSingleObject(handle, 60_000) } != WAIT_OBJECT_0 {
                return Err("等待主程序退出超时，更新已停止，请关闭程序后重试".into());
            }
            Ok(())
        })();
        unsafe {
            let _ = CloseHandle(handle);
        }
        result
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (pid, directory);
        Err("当前平台暂不支持自动安装".into())
    }
}

fn apply_portable(
    package: &Path,
    target: &Path,
    progress: &mut impl FnMut(String, Option<f32>),
) -> Result<(), String> {
    use std::io::Read;
    progress("正在解压并校验便携版文件…".into(), Some(0.0));
    let stage = tempfile::Builder::new()
        .prefix(".qcclip-update-")
        .tempdir_in(target)
        .map_err(|e| format!("无法写入程序目录：{e}"))?;
    let mut archive =
        zip::ZipArchive::new(fs::File::open(package).map_err(|e| format!("读取更新包失败：{e}"))?)
            .map_err(|e| format!("打开便携更新包失败：{e}"))?;
    let mut files = Vec::new();
    let mut total_size = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("读取压缩条目失败：{e}"))?;
        if entry.enclosed_name().is_none() {
            return Err("便携更新包包含非法路径".into());
        }
        if entry.is_dir() {
            continue;
        }
        let name = entry
            .name()
            .strip_prefix("QcClip/")
            .filter(|name| allowed_portable_file(name))
            .ok_or("便携更新包包含非程序文件，已停止更新")?
            .to_owned();
        if files.contains(&name) {
            return Err("便携更新包包含重复文件".into());
        }
        total_size = total_size.saturating_add(entry.size());
        if total_size > 1024 * 1024 * 1024 {
            return Err("便携更新包解压后过大".into());
        }
        let mut output = fs::File::create(stage.path().join(&name))
            .map_err(|e| format!("创建暂存文件失败：{e}"))?;
        let expected_size = entry.size();
        let written = std::io::copy(&mut entry.by_ref().take(512 * 1024 * 1024 + 1), &mut output)
            .map_err(|e| format!("解压并写入暂存文件失败：{e}"))?;
        if written != expected_size {
            return Err("便携更新文件大小异常".into());
        }
        output
            .sync_all()
            .map_err(|e| format!("保存暂存文件失败：{e}"))?;
        files.push(name);
    }
    if !files.iter().any(|s| s == "QcClip.exe") || !files.iter().any(|s| s == "QcClip.portable") {
        return Err("便携更新包缺少必要文件".into());
    }
    validate_executable(&stage.path().join("QcClip.exe"))?;
    for name in &files {
        match target.join(name).symlink_metadata() {
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                return Err(format!("目标程序文件不是普通文件，已停止更新：{name}"));
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("检查目标文件失败：{error}"));
            }
            _ => {}
        }
    }
    let backup = stage.path().join("backup");
    fs::create_dir(&backup).map_err(|e| format!("创建程序备份失败：{e}"))?;
    let mut applied = Vec::new();
    for (index, name) in files.iter().enumerate() {
        progress(
            format!("正在更新文件：{name}（{}/{}）", index + 1, files.len()),
            Some(index as f32 / files.len() as f32),
        );
        let destination = target.join(name);
        let existed = destination.exists();
        let result = (|| -> std::io::Result<()> {
            if existed {
                fs::rename(&destination, backup.join(name))?;
            }
            applied.push((name.clone(), existed));
            fs::rename(stage.path().join(name), &destination)
        })();
        if let Err(error) = result {
            let mut failures = Vec::new();
            for (name, existed) in applied.iter().rev() {
                let path = target.join(name);
                if path.exists()
                    && let Err(e) = fs::remove_file(&path)
                {
                    failures.push(e.to_string());
                }
                if *existed && let Err(e) = fs::rename(backup.join(name), path) {
                    failures.push(e.to_string());
                }
            }
            if !failures.is_empty() {
                let retained = stage.keep();
                return Err(format!(
                    "更新失败：{error}；恢复部分文件失败：{}；原文件保存在 {}",
                    failures.join("；"),
                    retained.display()
                ));
            }
            return Err(format!("更新失败，已恢复原程序：{error}"));
        }
    }
    Ok(())
}

fn allowed_portable_file(name: &str) -> bool {
    matches!(
        name,
        "QcClip.exe"
            | "QcClip.portable"
            | "vcruntime140.dll"
            | "vcruntime140_1.dll"
            | "msvcp140.dll"
    )
}

fn validate_executable(path: &Path) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(path).map_err(|e| format!("校验程序架构失败：{e}"))?;
    let mut header = [0; 64];
    file.read_exact(&mut header).map_err(|_| "程序文件不完整")?;
    if &header[..2] != b"MZ" {
        return Err("程序文件格式错误".into());
    }
    let offset = u32::from_le_bytes(header[60..64].try_into().map_err(|_| "程序文件头错误")?);
    file.seek(SeekFrom::Start(offset as u64))
        .map_err(|_| "程序文件头错误")?;
    let mut pe = [0; 6];
    file.read_exact(&mut pe).map_err(|_| "程序文件头不完整")?;
    let machine = u16::from_le_bytes([pe[4], pe[5]]);
    let expected = if cfg!(target_arch = "aarch64") {
        0xaa64
    } else {
        0x8664
    };
    if &pe[..4] != b"PE\0\0" || machine != expected {
        return Err("更新包程序架构不匹配".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_update_preserves_user_data_and_rolls_back_locked_destination() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("update.zip");
        let target = root.path().join("app");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("QcClip.exe"), b"old").unwrap();
        fs::create_dir(target.join("QcClipData")).unwrap();
        fs::write(target.join("QcClipData/config.json"), b"user-data").unwrap();
        let mut pe = vec![0u8; 70];
        pe[..2].copy_from_slice(b"MZ");
        pe[60..64].copy_from_slice(&64u32.to_le_bytes());
        pe[64..68].copy_from_slice(b"PE\0\0");
        pe[68..70].copy_from_slice(
            &(if cfg!(target_arch = "aarch64") {
                0xaa64u16
            } else {
                0x8664u16
            })
            .to_le_bytes(),
        );
        write_zip(
            &package,
            &[("QcClip/QcClip.exe", &pe), ("QcClip/QcClip.portable", b"")],
        );
        apply_portable(&package, &target, &mut |_, _| {}).unwrap();
        assert_eq!(fs::read(target.join("QcClip.exe")).unwrap(), pe);
        assert_eq!(
            fs::read(target.join("QcClipData/config.json")).unwrap(),
            b"user-data"
        );

        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            fs::write(target.join("QcClip.exe"), b"old-again").unwrap();
            let _locked = fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(target.join("QcClip.portable"))
                .unwrap();
            assert!(
                apply_portable(&package, &target, &mut |_, _| {})
                    .unwrap_err()
                    .contains("已恢复")
            );
            assert_eq!(fs::read(target.join("QcClip.exe")).unwrap(), b"old-again");
        }
    }

    #[test]
    fn portable_update_rejects_data_files_and_path_traversal() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("update.zip");
        for name in [
            "QcClip/QcClipData/config.json",
            "QcClip/../../outside",
            "QcClip/evil.dll",
        ] {
            write_zip(&package, &[(name, b"bad")]);
            assert!(apply_portable(&package, root.path(), &mut |_, _| {}).is_err());
        }
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        use std::io::Write;
        let mut archive = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, content) in entries {
            archive
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(content).unwrap();
        }
        archive.finish().unwrap();
    }
}
