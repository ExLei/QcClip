//! 启动时记录系统与硬件概况，便于根据用户日志排查性能和兼容性问题。
//!
//! 只采集与排查相关的非隐私信息：系统版本、CPU 型号、内存容量、显卡与显示器参数等，
//! 不记录计算机名、用户名、IP、序列号等可识别个人或设备的信息。

/// 在后台线程采集并写入日志，避免 DXGI/D3D 探测拖慢启动。
pub(crate) fn log_in_background() {
    let result = std::thread::Builder::new()
        .name("system-info".into())
        .spawn(|| {
            #[cfg(target_os = "windows")]
            windows_impl::log_system_info();
            #[cfg(not(target_os = "windows"))]
            tracing::info!(
                os = std::env::consts::OS,
                arch = std::env::consts::ARCH,
                logical_cpus = logical_cpus(),
                "系统信息"
            );
        });
    if let Err(error) = result {
        tracing::warn!(%error, "无法启动系统信息采集线程");
    }
}

#[cfg(target_os = "windows")]
pub(crate) use windows_impl::{adapters, is_remote_session};

/// 一块显卡的概况；同一硬件的重复枚举已合并到 `enumerated`。
#[derive(Clone, Debug)]
pub(crate) struct AdapterInfo {
    pub(crate) name: String,
    pub(crate) vendor_id: u32,
    pub(crate) device_id: u32,
    pub(crate) dedicated_video_mib: u64,
    pub(crate) shared_system_mib: u64,
    /// DXGI 标记的软件适配器，即 Microsoft Basic Render Driver（WARP）。
    pub(crate) software: bool,
    pub(crate) driver_version: Option<String>,
    /// 可创建的最高 D3D 特性级别；`Err` 为设备创建失败的原因。
    pub(crate) feature_level: Result<FeatureLevel, String>,
    pub(crate) enumerated: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FeatureLevel {
    pub(crate) major: u8,
    pub(crate) minor: u8,
}

impl FeatureLevel {
    /// 渲染器着色器使用 Shader Model 5.0，要求 D3D 11.0 及以上。
    pub(crate) const RENDERER_MINIMUM: Self = Self {
        major: 11,
        minor: 0,
    };
}

impl std::fmt::Display for FeatureLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

fn logical_cpus() -> usize {
    std::thread::available_parallelism().map_or(0, |count| count.get())
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use windows::Win32::{
        Foundation::{HMODULE, LPARAM, RECT},
        Graphics::{
            Direct3D::{
                D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_9_1,
                D3D_FEATURE_LEVEL_9_2, D3D_FEATURE_LEVEL_9_3, D3D_FEATURE_LEVEL_10_0,
                D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
            },
            Direct3D11::{D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice},
            Dxgi::{
                CreateDXGIFactory1, DXGI_ADAPTER_DESC1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIAdapter1,
                IDXGIDevice, IDXGIFactory1,
            },
            Gdi::{
                DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplayMonitors, EnumDisplaySettingsW,
                GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
            },
        },
        System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION},
        },
    };
    use windows::core::{BOOL, Interface, PCWSTR};
    use winreg::{RegKey, enums::HKEY_LOCAL_MACHINE};

    use super::{AdapterInfo, FeatureLevel};

    const MONITORINFOF_PRIMARY: u32 = 1;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: u64 = 1024 * 1024;

    pub(super) fn log_system_info() {
        let os = os_description();
        let (memory_total_gib, memory_available_gib) = memory_gib();
        tracing::info!(
            os = %os,
            arch = std::env::consts::ARCH,
            cpu = %cpu_name().unwrap_or_else(|| "未知".into()),
            logical_cpus = super::logical_cpus(),
            memory_total_gib = format_args!("{memory_total_gib:.1}"),
            memory_available_gib = format_args!("{memory_available_gib:.1}"),
            remote_session = is_remote_session(),
            elevated = crate::platform::elevation::is_elevated().ok(),
            "系统信息"
        );
        log_adapters();
        log_monitors();
    }

    /// 例如 `Windows 11 Pro 24H2 (build 26100.4061)`。
    fn os_description() -> String {
        let key = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
            .ok();
        let read = |name: &str| {
            key.as_ref()
                .and_then(|key| key.get_value::<String, _>(name).ok())
        };
        let build = crate::platform::os_version::build_number();
        let mut product = read("ProductName").unwrap_or_else(|| "Windows".into());
        // Windows 11 的注册表 ProductName 仍写作 "Windows 10"，按 build 号纠正。
        if build.is_some_and(|build| build >= 22000) {
            product = product.replacen("Windows 10", "Windows 11", 1);
        }
        let display_version = read("DisplayVersion").or_else(|| read("ReleaseId"));
        let revision = key
            .as_ref()
            .and_then(|key| key.get_value::<u32, _>("UBR").ok());
        let mut description = product;
        if let Some(version) = display_version {
            description.push(' ');
            description.push_str(&version);
        }
        match (build, revision) {
            (Some(build), Some(revision)) => {
                description.push_str(&format!(" (build {build}.{revision})"))
            }
            (Some(build), None) => description.push_str(&format!(" (build {build})")),
            _ => {}
        }
        description
    }

    fn cpu_name() -> Option<String> {
        RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0")
            .ok()?
            .get_value::<String, _>("ProcessorNameString")
            .ok()
            .map(|name| name.trim().to_string())
    }

    fn memory_gib() -> (f64, f64) {
        let mut status = MEMORYSTATUSEX {
            dwLength: size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // SAFETY: status 已按要求设置 dwLength，指针指向本栈帧上的有效结构。
        if unsafe { GlobalMemoryStatusEx(&mut status) }.is_err() {
            return (0.0, 0.0);
        }
        (
            status.ullTotalPhys as f64 / GIB,
            status.ullAvailPhys as f64 / GIB,
        )
    }

    pub(crate) fn is_remote_session() -> bool {
        // SAFETY: 无前置条件的系统指标查询。
        unsafe { GetSystemMetrics(SM_REMOTESESSION) != 0 }
    }

    /// 枚举全部显卡及其可创建的最高 D3D 特性级别。
    /// 渲染器要求 11.0 及以上；所有硬件显卡都不满足时会退回软件渲染。
    pub(crate) fn adapters() -> Vec<AdapterInfo> {
        // SAFETY: 无前置条件的 COM 工厂创建。
        let factory: IDXGIFactory1 = match unsafe { CreateDXGIFactory1() } {
            Ok(factory) => factory,
            Err(error) => {
                tracing::warn!(%error, "无法创建 DXGI 工厂，跳过显卡信息");
                return Vec::new();
            }
        };
        // 同一块物理显卡可能以不同 LUID 重复出现（混合显卡、虚拟显示驱动等），
        // 按硬件描述合并，只探测一次并记录出现次数。
        let mut adapters: Vec<(IDXGIAdapter1, DXGI_ADAPTER_DESC1, usize)> = Vec::new();
        for index in 0.. {
            // SAFETY: 越界时返回 DXGI_ERROR_NOT_FOUND，以此结束枚举。
            let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
                break;
            };
            // SAFETY: adapter 为有效的 COM 对象。
            let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
                continue;
            };
            let same_hardware = |other: &DXGI_ADAPTER_DESC1| {
                other.Description == desc.Description
                    && other.VendorId == desc.VendorId
                    && other.DeviceId == desc.DeviceId
                    && other.SubSysId == desc.SubSysId
                    && other.Revision == desc.Revision
                    && other.Flags == desc.Flags
            };
            match adapters
                .iter_mut()
                .find(|(_, other, _)| same_hardware(other))
            {
                Some((_, _, count)) => *count += 1,
                None => adapters.push((adapter, desc, 1)),
            }
        }
        adapters
            .into_iter()
            .map(|(adapter, desc, enumerated)| AdapterInfo {
                name: String::from_utf16_lossy(&desc.Description)
                    .trim_matches(char::from(0))
                    .to_string(),
                vendor_id: desc.VendorId,
                device_id: desc.DeviceId,
                dedicated_video_mib: desc.DedicatedVideoMemory as u64 / MIB,
                shared_system_mib: desc.SharedSystemMemory as u64 / MIB,
                software: desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0,
                driver_version: driver_version(&adapter),
                feature_level: max_feature_level(&adapter),
                enumerated,
            })
            .collect()
    }

    fn log_adapters() {
        for (index, adapter) in adapters().iter().enumerate() {
            tracing::info!(
                index,
                name = %adapter.name,
                vendor_id = format_args!("{:#06X}", adapter.vendor_id),
                device_id = format_args!("{:#06X}", adapter.device_id),
                dedicated_video_mib = adapter.dedicated_video_mib,
                shared_system_mib = adapter.shared_system_mib,
                software = adapter.software,
                driver_version = adapter.driver_version.as_deref().unwrap_or("未知"),
                max_feature_level = %match &adapter.feature_level {
                    Ok(level) => level.to_string(),
                    Err(error) => format!("不可用 ({error})"),
                },
                enumerated = adapter.enumerated,
                "显卡"
            );
        }
    }

    fn driver_version(adapter: &IDXGIAdapter1) -> Option<String> {
        // SAFETY: 传入有效的接口 GUID；返回值为驱动的 UMD 版本号。
        let version = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) }.ok()? as u64;
        Some(format!(
            "{}.{}.{}.{}",
            version >> 48,
            (version >> 32) & 0xFFFF,
            (version >> 16) & 0xFFFF,
            version & 0xFFFF
        ))
    }

    fn max_feature_level(adapter: &IDXGIAdapter1) -> Result<FeatureLevel, String> {
        let levels = [
            D3D_FEATURE_LEVEL_11_1,
            D3D_FEATURE_LEVEL_11_0,
            D3D_FEATURE_LEVEL_10_1,
            D3D_FEATURE_LEVEL_10_0,
            D3D_FEATURE_LEVEL_9_3,
            D3D_FEATURE_LEVEL_9_2,
            D3D_FEATURE_LEVEL_9_1,
        ];
        let mut level = D3D_FEATURE_LEVEL::default();
        // SAFETY: 只探测特性级别，不需要设备与上下文对象，传 None 时 D3D 不会创建它们。
        let result = unsafe {
            D3D11CreateDevice(
                adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                None,
                Some(&mut level),
                None,
            )
        };
        match result {
            Ok(()) => Ok(FeatureLevel {
                major: ((level.0 >> 12) & 0xF) as u8,
                minor: ((level.0 >> 8) & 0xF) as u8,
            }),
            Err(error) => Err(error.to_string()),
        }
    }

    fn log_monitors() {
        unsafe extern "system" fn collect(
            monitor: HMONITOR,
            _: HDC,
            _: *mut RECT,
            data: LPARAM,
        ) -> BOOL {
            // SAFETY: data 是 log_monitors 栈上 Vec 的指针，枚举在本函数返回前完成。
            let monitors = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
            monitors.push(monitor);
            true.into()
        }

        let mut monitors: Vec<HMONITOR> = Vec::new();
        // SAFETY: 回调只在本调用期间执行，指针在此期间有效。
        unsafe {
            let _ = EnumDisplayMonitors(
                None,
                None,
                Some(collect),
                LPARAM(&mut monitors as *mut _ as isize),
            );
        }
        for (index, monitor) in monitors.into_iter().enumerate() {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            // SAFETY: cbSize 标明为 MONITORINFOEXW，系统按扩展结构写入。
            if !unsafe { GetMonitorInfoW(monitor, &mut info as *mut _ as *mut MONITORINFO) }
                .as_bool()
            {
                continue;
            }
            let mut mode = DEVMODEW {
                dmSize: size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            // SAFETY: szDevice 是系统写入的以 NUL 结尾的设备名。
            let has_mode = unsafe {
                EnumDisplaySettingsW(
                    PCWSTR(info.szDevice.as_ptr()),
                    ENUM_CURRENT_SETTINGS,
                    &mut mode,
                )
            }
            .as_bool();
            let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
            // SAFETY: 输出参数指向本栈帧上的有效变量。
            let scale =
                unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
                    .ok()
                    .map(|()| dpi_x * 100 / 96);
            tracing::info!(
                index,
                primary = info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                resolution = %if has_mode {
                    format!("{}x{}", mode.dmPelsWidth, mode.dmPelsHeight)
                } else {
                    "未知".into()
                },
                refresh_hz = has_mode.then_some(mode.dmDisplayFrequency),
                scale_percent = scale,
                "显示器"
            );
        }
    }
}
