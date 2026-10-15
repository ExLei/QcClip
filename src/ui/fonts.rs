use std::sync::OnceLock;

use gpui::{App, FontId, FontWeight, Pixels, Window, px};
use gpui_component::ActiveTheme as _;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AppFontMetrics {
    font_id: FontId,
    line_height_scale: f32,
    scale_factor: f32,
}

impl AppFontMetrics {
    pub(crate) fn from_window(window: &Window, cx: &App) -> Self {
        let mut style = window.text_style();
        style.font_family = cx.theme().font_family.clone();
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&style.font());
        let font_size = window.rem_size();
        // 使用实际排版高度，包含中文回退字体及字重变化，避免只按字号估算。
        let sample = "Ag国";
        let mut line_height = px(0.0);
        for weight in [FontWeight::NORMAL, FontWeight::MEDIUM, FontWeight::BOLD] {
            style.font_weight = weight;
            let line = text_system.shape_line(
                sample.into(), font_size, &[style.to_run(sample.len())], None,
            );
            line_height = line_height.max(line.ascent + line.descent);
        }
        Self {
            font_id,
            line_height_scale: f32::from(line_height) / f32::from(font_size),
            scale_factor: window.scale_factor(),
        }
    }

    pub(crate) fn line_height(self, font_size: Pixels, minimum: Pixels) -> Pixels {
        let height = minimum.max(font_size * self.line_height_scale);
        // 与 GPUI 的设备像素对齐一致，向上取整防止最后一行被裁掉。
        px((f32::from(height) * self.scale_factor).ceil() / self.scale_factor)
    }

    #[cfg(test)]
    pub(crate) fn for_test(line_height_scale: f32, scale_factor: f32) -> Self {
        Self { font_id: FontId(0), line_height_scale, scale_factor }
    }
}

pub(crate) fn system_font_names(cx: &App) -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names = enumerate_system_fonts(cx);
        sort_font_names(&mut names);
        names
    })
}

fn sort_font_names(names: &mut Vec<String>) {
    names.retain(|name| !name.trim().is_empty());
    names.sort_by_cached_key(|name| (font_priority(name), name.to_lowercase(), name.clone()));
    names.dedup();
}

fn font_priority(name: &str) -> u8 {
    match name.to_lowercase().as_str() {
        "microsoft yahei ui" | "微软雅黑 ui" | "microsoft yahei" | "微软雅黑"
        | "pingfang sc" | "苹方-简" | "noto sans cjk sc" | "noto sans sc"
        | "source han sans sc" | "思源黑体" | "思源黑体 cn"
        | "segoe ui" | "segoe ui variable" | "arial" => return 0,
        _ => {}
    }
    if !name.chars().next().is_some_and(char::is_alphabetic) {
        return 3;
    }
    if name.chars().any(|ch| matches!(ch as u32,
        0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x323af
    )) {
        return 1;
    }
    2
}

#[cfg(not(target_os = "windows"))]
fn enumerate_system_fonts(cx: &App) -> Vec<String> {
    cx.text_system().all_font_names().into_iter()
        .filter(|name| !name.starts_with('.'))
        .collect()
}

#[cfg(target_os = "windows")]
fn enumerate_system_fonts(_cx: &App) -> Vec<String> {
    match windows_font_names() {
        Ok(names) => names,
        Err(error) => {
            tracing::error!("读取系统字体列表失败：{error}");
            Vec::new()
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_font_names() -> windows::core::Result<Vec<String>> {
    use windows::{
        core::{BOOL, HSTRING, w},
        Win32::{
            Globalization::GetUserDefaultLocaleName,
            Graphics::DirectWrite::{DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED},
        },
    };

    // 仅查询字体族名称，不创建字体实例，也不读取或注册字体文件。
    // 所有缓冲区均由本函数持有，并为 Windows 返回的字符串保留结尾空字符。
    unsafe {
        let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let mut collection = None;
        factory.GetSystemFontCollection(&mut collection, false)?;
        let Some(collection) = collection else { return Ok(Vec::new()); };
        let mut locale = [0u16; 85];
        let locale_len = GetUserDefaultLocaleName(&mut locale);
        let locale = if locale_len > 1 {
            HSTRING::from_wide(&locale[..locale_len as usize - 1])
        } else {
            HSTRING::from("en-US")
        };
        let mut result = Vec::new();
        for index in 0..collection.GetFontFamilyCount() {
            let name = (|| -> windows::core::Result<String> {
                let names = collection.GetFontFamily(index)?.GetFamilyNames()?;
                let mut index = 0;
                let mut exists = BOOL(0);
                names.FindLocaleName(&locale, &mut index, &mut exists)?;
                if !exists.as_bool() {
                    names.FindLocaleName(w!("en-US"), &mut index, &mut exists)?;
                    if !exists.as_bool() { index = 0; }
                }
                let length = names.GetStringLength(index)? as usize;
                let mut buffer = vec![0u16; length + 1];
                names.GetString(index, &mut buffer)?;
                Ok(String::from_utf16_lossy(&buffer[..length]))
            })();
            match name {
                Ok(name) => result.push(name),
                Err(error) => tracing::warn!("读取字体族名称失败：{error}"),
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_font_line_height_fits_tall_fonts_and_aligns_to_device_pixels() {
        let metrics = super::AppFontMetrics::for_test(1.6, 1.25);
        assert_eq!(metrics.line_height(gpui::px(14.0), gpui::px(17.5)), gpui::px(22.4));
        let height = metrics.line_height(gpui::px(11.0), gpui::px(20.0));
        assert_eq!(height, gpui::px(20.0));
        let fractional = metrics.line_height(gpui::px(9.0), gpui::px(17.5));
        assert!(fractional >= gpui::px(17.5));
        assert_eq!(fractional, gpui::px(17.6));
    }

    #[test]
    fn app_font_order_prioritizes_readable_families_and_keeps_numbered_fonts() {
        let mut names = ["04b03", "Arial", "叶根友微宋", "00Antix", "微软雅黑",
            "宋体", "zebra", "Calibri", "@宋体", "Arial", ""].map(str::to_owned).to_vec();
        super::sort_font_names(&mut names);
        assert_eq!(names, ["Arial", "微软雅黑", "叶根友微宋", "宋体", "Calibri",
            "zebra", "00Antix", "04b03", "@宋体"]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn app_font_enumeration_returns_installed_family_names() {
        let names = super::windows_font_names().expect("无法读取系统字体列表");
        assert!(!names.is_empty(), "系统字体列表不应为空");
        assert!(names.iter().all(|name| !name.trim().is_empty()));
        assert!(!names.iter().any(|name| name == ".SystemUIFont"));
    }
}
