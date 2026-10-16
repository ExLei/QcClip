use crate::config::WindowMaterial;

#[cfg(any(target_os = "windows", test))]
const CORNER_RADIUS: f32 = 8.0;

/// 原生圆角从 Windows 11 21H2 开始支持，与窗口材质的版本分档独立。
#[cfg(target_os = "windows")]
pub(super) fn supports_native_rounding() -> bool {
    static CACHED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| native_rounding_for_build(super::os_version::build_number()))
}

#[cfg(any(target_os = "windows", test))]
fn native_rounding_for_build(build: Option<u32>) -> bool {
    matches!(build, Some(22000..))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FrameStyle {
    pub(crate) corner_radius: f32,
}

#[cfg(any(target_os = "windows", test))]
fn frame_policy(
    native_rounding: bool,
    material: WindowMaterial,
    decorations_visible: bool,
    maximized: bool,
    fullscreen: bool,
) -> Option<FrameStyle> {
    if native_rounding || !decorations_visible || fullscreen {
        return None;
    }
    let square = maximized || matches!(material, WindowMaterial::Acrylic | WindowMaterial::Blur);
    Some(FrameStyle {
        corner_radius: if square { 0.0 } else { CORNER_RADIUS },
    })
}

pub(crate) fn apply(window: &gpui::Window, material: WindowMaterial) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    if !supports_native_rounding() {
        return native::apply(window, material);
    }
    let _ = (window, material);
    Ok(())
}

pub(crate) fn frame_style(window: &gpui::Window) -> Option<FrameStyle> {
    #[cfg(target_os = "windows")]
    if !supports_native_rounding() {
        return native::frame_style(window);
    }
    let _ = window;
    None
}

#[cfg(target_os = "windows")]
pub(super) fn set_decorations_visible(
    hwnd: windows::Win32::Foundation::HWND,
    visible: bool,
) -> Result<(), String> {
    native::set_decorations_visible(hwnd, visible)
}

#[cfg(target_os = "windows")]
mod native {
    use super::{FrameStyle, WindowMaterial, frame_policy};
    use windows::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn},
        UI::{
            HiDpi::GetDpiForWindow,
            Shell::{DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                GetWindowRect, IsIconic, IsZoomed, WM_DPICHANGED, WM_NCDESTROY, WM_SIZE,
                WM_WINDOWPOSCHANGED,
            },
        },
    };

    const SUBCLASS_ID: usize = 1;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct RoundedRegion {
        width: i32,
        height: i32,
        diameter: i32,
    }

    impl RoundedRegion {
        fn new(width: i32, height: i32, dpi: u32) -> Self {
            let width = width.max(1);
            let height = height.max(1);
            let diameter = (super::CORNER_RADIUS * 2.0 * dpi.max(96) as f32 / 96.0).round() as i32;
            Self {
                width,
                height,
                diameter: diameter.min(width).min(height),
            }
        }
    }

    #[derive(Clone, Copy)]
    struct AppearanceState {
        material: WindowMaterial,
        decorations_visible: bool,
        fullscreen: bool,
        region: Option<RoundedRegion>,
        updating: bool,
        rounding_failed: bool,
    }

    fn state(hwnd: HWND) -> Option<*mut AppearanceState> {
        let mut data = 0;
        // SAFETY: 只查询本模块在当前窗口线程安装的子类状态。
        unsafe {
            GetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, Some(&mut data))
                .as_bool()
                .then_some(data as *mut AppearanceState)
        }
    }

    fn install(hwnd: HWND, material: WindowMaterial, fullscreen: bool) -> Result<(), String> {
        let data = Box::into_raw(Box::new(AppearanceState {
            material,
            decorations_visible: true,
            fullscreen,
            region: None,
            updating: false,
            rounding_failed: false,
        }));
        // SAFETY: 状态地址保持稳定，由 WM_NCDESTROY 回收；子类仅处理当前线程的窗口。
        if !unsafe { SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, data as usize) }
            .as_bool()
        {
            // SAFETY: 安装失败时系统未取得状态所有权。
            unsafe { drop(Box::from_raw(data)) };
            return Err("安装窗口外观处理失败".to_owned());
        }
        Ok(())
    }

    pub(super) fn apply(window: &gpui::Window, material: WindowMaterial) -> Result<(), String> {
        let hwnd = super::super::window::window_hwnd(window)?;
        apply_to_hwnd(hwnd, material, window.is_fullscreen())
    }

    fn apply_to_hwnd(hwnd: HWND, material: WindowMaterial, fullscreen: bool) -> Result<(), String> {
        if let Some(data) = state(hwnd) {
            // SAFETY: 状态由当前窗口线程独占，修改期间不调用可能重入的系统接口。
            unsafe {
                if (*data).material != material {
                    (*data).rounding_failed = false;
                }
                (*data).material = material;
                (*data).fullscreen = fullscreen;
            }
        } else {
            install(hwnd, material, fullscreen)?;
        }
        refresh(hwnd)
    }

    pub(super) fn set_decorations_visible(hwnd: HWND, visible: bool) -> Result<(), String> {
        if let Some(data) = state(hwnd) {
            // SAFETY: 仅在窗口线程修改仍有效的状态，随后才调用可能重入的系统接口。
            unsafe { (*data).decorations_visible = visible };
            refresh(hwnd)?;
        }
        Ok(())
    }

    fn current_style(hwnd: HWND) -> Option<FrameStyle> {
        let data = state(hwnd)?;
        // SAFETY: 复制状态后再查询系统，避免跨窗口消息持有 Rust 引用。
        let snapshot = unsafe { *data };
        // SAFETY: hwnd 来自当前窗口，仅读取最大化状态。
        let maximized = unsafe { IsZoomed(hwnd).as_bool() };
        let mut style = frame_policy(
            false,
            snapshot.material,
            snapshot.decorations_visible,
            maximized,
            snapshot.fullscreen,
        )?;
        if snapshot.rounding_failed {
            style.corner_radius = 0.0;
        }
        Some(style)
    }

    pub(super) fn frame_style(window: &gpui::Window) -> Option<FrameStyle> {
        let hwnd = super::super::window::window_hwnd(window).ok()?;
        let fullscreen = window.is_fullscreen();
        let Some(data) = state(hwnd) else {
            // 子类安装失败时仍保留直角边框，保证窗口与桌面有清晰边界。
            return (!fullscreen).then_some(FrameStyle { corner_radius: 0.0 });
        };
        // SAFETY: 在当前窗口线程读取状态，不持有跨系统调用的引用。
        if unsafe { (*data).fullscreen != fullscreen } {
            unsafe { (*data).fullscreen = fullscreen };
            if let Err(error) = refresh(hwnd) {
                tracing::warn!("同步全屏窗口外观失败：{error}");
            }
        }
        current_style(hwnd)
    }

    fn refresh(hwnd: HWND) -> Result<(), String> {
        let Some(data) = state(hwnd) else {
            return Ok(());
        };
        // SAFETY: 只复制当前窗口线程的有效状态，不跨系统调用持有引用。
        let snapshot = unsafe { *data };
        // SAFETY: 仅读取当前窗口是否处于最小化状态。
        if snapshot.updating || unsafe { IsIconic(hwnd).as_bool() } {
            return Ok(());
        }
        let target = if current_style(hwnd).is_some_and(|style| style.corner_radius > 0.0) {
            let mut rect = RECT::default();
            // SAFETY: rect 是有效的栈上结构，窗口区域坐标以整个窗口左上角为原点。
            unsafe { GetWindowRect(hwnd, &mut rect) }
                .map_err(|error| format!("读取窗口圆角区域尺寸失败：{error}"))?;
            // SAFETY: hwnd 来自当前窗口，仅读取其物理 DPI。
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            Some(RoundedRegion::new(
                rect.right - rect.left,
                rect.bottom - rect.top,
                dpi,
            ))
        } else {
            None
        };
        if target == snapshot.region {
            return Ok(());
        }

        // SetWindowRgn 会同步触发位置消息；必须在调用前阻止重入更新。
        // SAFETY: 当前窗口线程独占状态，赋值期间没有窗口消息调用。
        unsafe { (*data).updating = true };
        let result = set_region(hwnd, target);
        let actual_region = if result.is_err() && target.is_some() {
            // 创建或应用圆角失败后移除旧区域，避免旧尺寸继续截断窗口内容。
            if snapshot.region.is_some() {
                set_region(hwnd, None).map(|_| None)
            } else {
                Ok(None)
            }
        } else {
            result.as_ref().map(|_| target).map_err(Clone::clone)
        };
        if let Some(data) = state(hwnd) {
            // SAFETY: 系统调用后重新查询状态，避免窗口销毁后使用旧地址。
            unsafe {
                (*data).updating = false;
                (*data).rounding_failed |= result.is_err() && target.is_some();
                if let Ok(region) = actual_region.as_ref() {
                    (*data).region = *region;
                }
            }
        }
        actual_region?;
        result
    }

    fn set_region(hwnd: HWND, region: Option<RoundedRegion>) -> Result<(), String> {
        let handle = if let Some(region) = region {
            // SAFETY: 区域尺寸为正，直径已限制在窗口尺寸内，坐标相对整个窗口。
            let handle = unsafe {
                CreateRoundRectRgn(
                    0,
                    0,
                    region.width.saturating_add(1),
                    region.height.saturating_add(1),
                    region.diameter,
                    region.diameter,
                )
            };
            if handle.is_invalid() {
                return Err("创建窗口圆角区域失败，已回退直角边框".to_owned());
            }
            Some(handle)
        } else {
            None
        };
        // SAFETY: 成功后区域所有权交给系统；None 用于清除本模块设置的裁剪。
        if unsafe { SetWindowRgn(hwnd, handle, true) } == 0 {
            if let Some(handle) = handle {
                // SAFETY: 设置失败时区域仍归调用方所有，必须主动释放。
                unsafe {
                    let _ = DeleteObject(handle.into());
                }
            }
            return Err("更新窗口圆角区域失败".to_owned());
        }
        Ok(())
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        subclass_id: usize,
        data: usize,
    ) -> LRESULT {
        if message == WM_NCDESTROY {
            // SAFETY: 状态由 install 分配，仅在移除子类后回收一次。
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(window_proc), subclass_id);
                drop(Box::from_raw(data as *mut AppearanceState));
                return DefSubclassProc(hwnd, message, wparam, lparam);
            }
        }
        // SAFETY: 原样转发消息，待 GPUI 和已有窗口钩子更新尺寸及 DPI 后再更新裁剪。
        let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
        if matches!(message, WM_SIZE | WM_WINDOWPOSCHANGED | WM_DPICHANGED)
            && let Err(error) = refresh(hwnd)
        {
            tracing::warn!("更新窗口外观失败：{error}");
        }
        result
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::Win32::{
            Graphics::Gdi::{CreateRectRgn, GetWindowRgn, PtInRegion},
            UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
                WS_POPUP,
            },
        };

        struct TestWindow(HWND);

        impl TestWindow {
            fn new() -> Self {
                // SAFETY: 创建当前测试线程专用的隐藏窗口，由 Drop 在同一线程销毁。
                Self(unsafe {
                    CreateWindowExW(
                        Default::default(),
                        windows::core::w!("STATIC"),
                        windows::core::w!("窗口外观测试"),
                        WS_POPUP,
                        0,
                        0,
                        320,
                        200,
                        None,
                        None,
                        None,
                        None,
                    )
                    .expect("创建窗口外观测试窗口失败")
                })
            }

            fn region_contains(&self, x: i32, y: i32) -> bool {
                // SAFETY: 创建查询结果的接收区域，始终由本函数释放。
                unsafe {
                    let region = CreateRectRgn(0, 0, 0, 0);
                    assert!(!region.is_invalid());
                    let kind = GetWindowRgn(self.0, region);
                    let contains = kind.0 != 0 && PtInRegion(region, x, y).as_bool();
                    let _ = DeleteObject(region.into());
                    contains
                }
            }

            fn has_rounded_region(&self) -> bool {
                self.region_contains(160, 100) && !self.region_contains(0, 0)
            }
        }

        impl Drop for TestWindow {
            fn drop(&mut self) {
                // SAFETY: 窗口由当前线程创建，销毁消息同时回收子类状态。
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }

        #[test]
        fn native_material_switches_remove_and_restore_the_window_region() {
            let window = TestWindow::new();
            for material in [
                WindowMaterial::Disabled,
                WindowMaterial::Transparent,
                WindowMaterial::CustomImage,
            ] {
                apply_to_hwnd(window.0, material, false).unwrap();
                assert!(window.has_rounded_region());
                for system_material in [WindowMaterial::Acrylic, WindowMaterial::Blur] {
                    apply_to_hwnd(window.0, system_material, false).unwrap();
                    assert!(!window.has_rounded_region());
                    assert!(!window.region_contains(160, 100));
                    assert_eq!(current_style(window.0).unwrap().corner_radius, 0.0);
                    apply_to_hwnd(window.0, material, false).unwrap();
                    assert!(window.has_rounded_region());
                }
            }
        }

        #[test]
        fn resizing_and_visibility_changes_keep_the_region_in_sync() {
            let window = TestWindow::new();
            apply_to_hwnd(window.0, WindowMaterial::Disabled, false).unwrap();
            // SAFETY: 只改变本测试窗口尺寸，验证同步位置消息会更新区域。
            unsafe {
                SetWindowPos(
                    window.0,
                    None,
                    0,
                    0,
                    480,
                    300,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )
                .expect("调整测试窗口尺寸失败");
            }
            // SAFETY: 子类状态仍由当前测试线程拥有。
            let region = unsafe { (*state(window.0).unwrap()).region.unwrap() };
            assert_eq!((region.width, region.height), (480, 300));
            assert!(window.region_contains(479, 150));
            assert!(!window.region_contains(480, 300));
            set_decorations_visible(window.0, false).unwrap();
            assert!(!window.has_rounded_region());
            assert!(current_style(window.0).is_none());
            set_decorations_visible(window.0, true).unwrap();
            assert!(window.has_rounded_region());
            apply_to_hwnd(window.0, WindowMaterial::Blur, false).unwrap();
            set_decorations_visible(window.0, false).unwrap();
            set_decorations_visible(window.0, true).unwrap();
            assert!(!window.has_rounded_region());
            assert_eq!(current_style(window.0).unwrap().corner_radius, 0.0);
        }

        #[test]
        fn fullscreen_clears_the_region_until_restored() {
            let window = TestWindow::new();
            apply_to_hwnd(window.0, WindowMaterial::Disabled, false).unwrap();
            assert!(window.has_rounded_region());
            apply_to_hwnd(window.0, WindowMaterial::Disabled, true).unwrap();
            assert!(!window.has_rounded_region());
            assert!(!window.region_contains(160, 100));
            assert!(current_style(window.0).is_none());
            apply_to_hwnd(window.0, WindowMaterial::Disabled, false).unwrap();
            assert!(window.has_rounded_region());
        }

        #[test]
        fn rounded_region_scales_with_dpi_and_fits_small_windows() {
            assert_eq!(RoundedRegion::new(320, 200, 96).diameter, 16);
            assert_eq!(RoundedRegion::new(320, 200, 144).diameter, 24);
            assert_eq!(RoundedRegion::new(320, 200, 192).diameter, 32);
            assert_eq!(RoundedRegion::new(4, 1, 192).diameter, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_rounding_starts_at_windows11_21h2() {
        for build in [None, Some(7601), Some(19045), Some(21999)] {
            assert!(!native_rounding_for_build(build));
        }
        for build in [22000, 22621, 26100] {
            assert!(native_rounding_for_build(Some(build)));
        }
    }

    #[test]
    fn windows11_keeps_native_decorations_for_every_material() {
        for material in WindowMaterial::ALL {
            assert!(frame_policy(true, material, true, false, false).is_none());
        }
    }

    #[test]
    fn legacy_system_materials_are_square_and_ui_backgrounds_are_rounded() {
        for material in WindowMaterial::ALL {
            let style = frame_policy(false, material, true, false, false).unwrap();
            let expected = match material {
                WindowMaterial::Acrylic | WindowMaterial::Blur => 0.0,
                _ => CORNER_RADIUS,
            };
            assert_eq!(style.corner_radius, expected);
        }
    }

    #[test]
    fn maximized_windows_are_square_and_hidden_or_fullscreen_windows_have_no_frame() {
        for material in WindowMaterial::ALL {
            assert_eq!(
                frame_policy(false, material, true, true, false)
                    .unwrap()
                    .corner_radius,
                0.0
            );
            assert!(frame_policy(false, material, false, false, false).is_none());
            assert!(frame_policy(false, material, true, false, true).is_none());
        }
    }
}
