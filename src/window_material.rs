use std::{collections::HashMap, path::PathBuf, sync::Arc};

use gpui::{
    AnyView, AnyWindowHandle, App, AppContext as _, BorrowAppContext as _, Context, Global, Hsla,
    IntoElement, ParentElement, Render, RenderImage, Styled, StyledImage, Window,
    WindowBackgroundAppearance, WindowId, div, img, px,
};
use gpui_component::{ActiveTheme as _, Root, Theme, ThemeColor};

use crate::{
    config::{
        AppearanceConfig, BACKGROUND_IMAGE_BLUR_DEFAULT, WINDOW_MATERIAL_BRIGHTNESS_DEFAULT,
        WINDOW_MATERIAL_BRIGHTNESS_MAX, WINDOW_MATERIAL_BRIGHTNESS_MIN,
        WINDOW_MATERIAL_OPACITY_MAX, WINDOW_MATERIAL_OPACITY_MIN, WindowMaterial,
    },
    platform::{
        os_version::{self, WindowsVersion},
        window as platform_window,
    },
};

/// `appearance.background_image_blur` 非有限值时的兜底。
const BACKGROUND_IMAGE_BLUR_FALLBACK: f32 = BACKGROUND_IMAGE_BLUR_DEFAULT;
/// 背景图解码后允许的最长边。模糊是低通滤波，降采样视觉无损，
/// 但能降低 CPU 内存与每窗口 GPU 纹理占用。
const BACKGROUND_IMAGE_MAX_DIMENSION: u32 = 1280;

#[derive(Default)]
struct WindowMaterialRuntime {
    base_colors: [ThemeColor; 2],
    windows: HashMap<WindowId, AnyWindowHandle>,
    background_image: Option<Arc<RenderImage>>,
}

impl Global for WindowMaterialRuntime {}

pub(crate) fn install(cx: &mut App) {
    let appearance = crate::config_runtime::appearance_config(cx);
    let background_image = (appearance.window_material == WindowMaterial::CustomImage)
        .then(|| {
            appearance
                .background_image_path
                .map(PathBuf::from)
                .filter(|path| path.is_file())
        })
        .flatten()
        .as_deref()
        .and_then(load_background_image);
    cx.set_global(WindowMaterialRuntime {
        base_colors: [ThemeColor::default(); 2],
        windows: HashMap::new(),
        background_image,
    });
    cx.on_window_closed(|cx, window_id| {
        cx.update_global::<WindowMaterialRuntime, _>(|runtime, _| {
            runtime.windows.remove(&window_id);
        });
    })
    .detach();
}

pub(crate) fn new_root(
    view: impl Into<AnyView>,
    window: &mut Window,
    cx: &mut Context<Root>,
) -> Root {
    // 窗口材质由业务根容器绘制，Root 只负责叠加层，避免半透明背景重复覆盖桌面。
    let background_view = cx.new(|_| BackgroundRoot { view: view.into() });
    Root::new(background_view, window, cx).bg(gpui::transparent_black())
}

struct BackgroundRoot {
    view: AnyView,
}

impl Render for BackgroundRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = background_element(window, cx);
        div()
            .relative()
            .size_full()
            .child(background)
            .child(self.view.clone())
            .children(
                Root::render_sheet_layer(window, cx)
                    .map(|layer| div().absolute().inset_0().size_full().child(layer)),
            )
            .children(
                Root::render_dialog_layer(window, cx)
                    .map(|layer| div().absolute().inset_0().size_full().child(layer)),
            )
            .children(
                Root::render_notification_layer(window, cx)
                    .map(|layer| div().absolute().inset_0().size_full().child(layer)),
            )
    }
}

/// 供不走 [`new_root`] 的窗口（如菜单弹窗）复用同一套背景。
pub(crate) fn with_background(
    view: impl IntoElement,
    window: &Window,
    cx: &App,
) -> impl IntoElement {
    let background = background_element(window, cx);
    div()
        .relative()
        .size_full()
        .font_family(cx.theme().font_family.clone())
        .child(background)
        .child(view)
}

pub(crate) fn background_element(window: &Window, cx: &App) -> gpui::AnyElement {
    let appearance = crate::config_runtime::appearance_config(cx);
    let custom_image = uses_custom_image_background(cx);
    let blur_radius = if appearance.background_image_blur.is_finite() {
        appearance.background_image_blur
    } else {
        BACKGROUND_IMAGE_BLUR_FALLBACK
    };
    let brightness = effective_brightness(&appearance);
    let material = supported_material(appearance.window_material);
    if !custom_image {
        if material == WindowMaterial::Blur {
            let mut container = div().absolute().inset_0();
            if let Some(color) = brightness_overlay(brightness).map(gpui::rgba) {
                container = container.child(div().absolute().inset_0().bg(color));
            }
            return container.into_any_element();
        }
        return div().absolute().inset_0().into_any_element();
    }
    let Some(image) =
        cx.read_global::<WindowMaterialRuntime, _>(|runtime, _| runtime.background_image.clone())
    else {
        return div().absolute().inset_0().into_any_element();
    };
    let window_bounds = window.bounds();
    let window_scale = f64::from(window.scale_factor());
    // 像桌面壁纸：每台显示器各铺满一张完整图。GPUI 的 display.bounds() 按各自
    // DPI 归一化，多屏 scale 不同时逻辑 bounds 会重叠，故改用 raw-input 的物理
    // 坐标（rcMonitor，连续不重叠）对齐：段位置/尺寸 = 物理值 / 窗口 scale。
    let window_physical_left = f64::from(window_bounds.origin.x) * window_scale;
    let window_physical_top = f64::from(window_bounds.origin.y) * window_scale;
    let window_physical_right =
        window_physical_left + f64::from(window_bounds.size.width) * window_scale;
    let window_physical_bottom =
        window_physical_top + f64::from(window_bounds.size.height) * window_scale;
    let segments = raw_input::Display::get_available_monitors()
        .into_iter()
        .filter(|m| {
            // 只渲染与窗口物理 bounds 相交的显示器。
            let m_right = m.offset.0 + m.size.0;
            let m_bottom = m.offset.1 + m.size.1;
            m.offset.0 < window_physical_right
                && m_right > window_physical_left
                && m.offset.1 < window_physical_bottom
                && m_bottom > window_physical_top
        })
        .map(|m| {
            let x = ((m.offset.0 - window_physical_left) / window_scale) as f32;
            let y = ((m.offset.1 - window_physical_top) / window_scale) as f32;
            let width = (m.size.0 / window_scale) as f32;
            let height = (m.size.1 / window_scale) as f32;
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(width))
                .h(px(height))
                .overflow_hidden()
                .child(
                    img(image.clone())
                        .size_full()
                        .object_fit(gpui::ObjectFit::Fill)
                        .blur(px(blur_radius)),
                )
                .into_any_element()
        })
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return div().absolute().inset_0().into_any_element();
    }
    let mut container = div().absolute().inset_0().children(segments);
    if let Some(color) = brightness_overlay(brightness).map(gpui::rgba) {
        container = container.child(div().absolute().inset_0().bg(color));
    }
    container.into_any_element()
}

/// 图片材质通过窗口背景层的黑/白半透明遮罩实现。
fn brightness_overlay(brightness: f32) -> Option<u32> {
    if brightness < 1.0 {
        let alpha = ((1.0 - brightness).clamp(0.0, 1.0) * 255.0).round() as u32;
        Some(alpha)
    } else if brightness > 1.0 {
        let alpha = ((brightness - 1.0).clamp(0.0, 1.0) * 255.0).round() as u32;
        Some(0xFFFFFF00 | alpha)
    } else {
        None
    }
}

pub(crate) fn uses_custom_image_background(cx: &App) -> bool {
    crate::config_runtime::appearance_config(cx).window_material == WindowMaterial::CustomImage
        && cx.read_global::<WindowMaterialRuntime, _>(|runtime, _| {
            runtime.background_image.is_some()
        })
}

fn load_background_image(path: &std::path::Path) -> Option<Arc<RenderImage>> {
    let image = match image::open(path) {
        Ok(image) => image.to_rgba8(),
        Err(error) => {
            tracing::warn!(%error, "加载背景图片失败，已使用纯色背景");
            return None;
        }
    };
    // 模糊交给 GPU 的 `blur()`，这里只降采样到最长边限制。
    let (w, h) = image.dimensions();
    let longest = w.max(h);
    let mut image = if longest > BACKGROUND_IMAGE_MAX_DIMENSION {
        let scale = BACKGROUND_IMAGE_MAX_DIMENSION as f32 / longest as f32;
        image::imageops::resize(
            &image,
            (w as f32 * scale).round().max(1.0) as u32,
            (h as f32 * scale).round().max(1.0) as u32,
            image::imageops::FilterType::Triangle,
        )
    } else {
        image
    };
    for pixel in image.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(image)])))
}

pub(crate) fn refresh_background_image(cx: &mut App) {
    let configured_path = crate::config_runtime::appearance_config(cx)
        .background_image_path
        .map(PathBuf::from);
    if configured_path.as_ref().is_some_and(|path| !path.is_file()) {
        tracing::warn!("背景图片文件不存在，已使用纯色背景");
    }
    let image = configured_path
        .filter(|path| path.is_file())
        .as_deref()
        .and_then(load_background_image);
    cx.update_global::<WindowMaterialRuntime, _>(|runtime, _| {
        runtime.background_image = image;
    });
    cx.refresh_windows();
}

pub(crate) fn register_window(window: &Window, cx: &mut App) {
    let window_id = window.window_handle().window_id();
    let handle = window.window_handle().into();
    cx.update_global::<WindowMaterialRuntime, _>(|runtime, _| {
        runtime.windows.insert(window_id, handle);
    });
    apply_to_window(window, cx);
}

pub(crate) fn apply_from_config(cx: &mut App) {
    if !cx.has_global::<WindowMaterialRuntime>() {
        return;
    }

    let windows = cx.read_global::<WindowMaterialRuntime, _>(|runtime, _| {
        runtime.windows.values().copied().collect::<Vec<_>>()
    });
    let retry = apply_material_to_windows(cx, windows);
    if !retry.is_empty() {
        // 触发变更的窗口此刻仍在处理自身事件，窗口对象被借出后无法重入更新，
        // 延后到本轮 effect 结束再补一次，避免它的材质要等下次打开才生效。
        cx.defer(move |cx| {
            apply_material_to_windows(cx, retry);
            cx.refresh_windows();
        });
    }
    cx.refresh_windows();
}

/// 向给定窗口下发材质，返回因为窗口正在被更新而没能处理的窗口。
fn apply_material_to_windows(cx: &mut App, windows: Vec<AnyWindowHandle>) -> Vec<AnyWindowHandle> {
    let mut pending = Vec::new();
    for handle in windows {
        if handle
            .update(cx, |_, window, cx| apply_to_window(window, cx))
            .is_err()
        {
            pending.push(handle);
        }
    }
    pending
}

/// 记录当前模式未经材质处理的主题色，供材质推导与黑名单窗口回溯使用。
pub(crate) fn capture_base_colors(cx: &mut App) {
    if !cx.has_global::<WindowMaterialRuntime>() {
        return;
    }
    let mode = cx.read_global::<Theme, _>(|theme, _| theme.mode);
    let base_colors = cx.read_global::<Theme, _>(|theme, _| theme.colors);
    cx.update_global::<WindowMaterialRuntime, _>(|runtime, _| {
        runtime.base_colors[usize::from(mode.is_dark())] = base_colors;
    });
    apply_theme_colors(cx);
}

pub(crate) fn excluded_window_theme(cx: &impl gpui::AppContext) -> ThemeColor {
    let mode = cx.read_global::<Theme, _>(|theme, _| theme.mode);
    cx.read_global::<WindowMaterialRuntime, _>(|runtime, _| {
        runtime.base_colors[usize::from(mode.is_dark())]
    })
}

/// 当前系统可选的窗口材质，用于设置页下拉选项。
pub(crate) fn available_materials() -> Vec<WindowMaterial> {
    let version = os_version::current();
    WindowMaterial::ALL
        .into_iter()
        .filter(|material| is_supported(*material, version))
        .collect()
}

/// 把当前系统不支持的材质回退成可用材质，避免窗口失去背景。
pub(crate) fn supported_material(material: WindowMaterial) -> WindowMaterial {
    let version = os_version::current();
    if is_supported(material, version) {
        return material;
    }
    match version {
        WindowsVersion::Earlier => WindowMaterial::Blur,
        _ => WindowMaterial::Acrylic,
    }
}

fn is_supported(material: WindowMaterial, version: WindowsVersion) -> bool {
    match material {
        WindowMaterial::Disabled | WindowMaterial::Transparent => true,
        WindowMaterial::Acrylic => version.supports_acrylic(),
        WindowMaterial::Blur => version.supports_blur(),
        WindowMaterial::CustomImage => true,
    }
}

fn apply_to_window(window: &Window, cx: &mut App) {
    let appearance = crate::config_runtime::appearance_config(cx);
    let material = supported_material(appearance.window_material);
    let mode = cx.read_global::<Theme, _>(|theme, _| theme.mode);
    let brightness = if matches!(
        material,
        WindowMaterial::Acrylic | WindowMaterial::Blur | WindowMaterial::CustomImage
    ) {
        effective_brightness(&appearance)
    } else {
        1.0
    };
    window.set_background_appearance(background_appearance(material));
    platform_window::apply_window_material(window, material, brightness, !mode.is_dark());
}

fn background_appearance(material: WindowMaterial) -> WindowBackgroundAppearance {
    match material {
        WindowMaterial::Disabled | WindowMaterial::Transparent => {
            WindowBackgroundAppearance::Transparent
        }
        WindowMaterial::Acrylic => WindowBackgroundAppearance::Blurred,
        WindowMaterial::Blur => {
            #[cfg(target_os = "windows")]
            {
                WindowBackgroundAppearance::Transparent
            }
            #[cfg(not(target_os = "windows"))]
            {
                WindowBackgroundAppearance::Blurred
            }
        }
        WindowMaterial::CustomImage => WindowBackgroundAppearance::Transparent,
    }
}

fn apply_theme_colors(cx: &mut App) {
    let appearance = crate::config_runtime::appearance_config(cx);
    let material = supported_material(appearance.window_material);
    let mode = cx.read_global::<Theme, _>(|theme, _| theme.mode);
    let base_colors = cx.read_global::<WindowMaterialRuntime, _>(|runtime, _| {
        runtime.base_colors[usize::from(mode.is_dark())]
    });
    let colors = if material == WindowMaterial::Disabled {
        base_colors
    } else {
        material_colors(base_colors, appearance.window_material_opacity)
    };

    cx.update_global::<Theme, _>(|theme, _| {
        theme.colors = colors;
        theme.tokens = colors.into();
    });
    Theme::sync_base(cx);
}

/// 将界面不透明度直接乘到主题表面原有的 Alpha 上，保留主题各角色的颜色和层次。
fn material_colors(base: ThemeColor, window_opacity: f32) -> ThemeColor {
    let window_alpha =
        window_opacity.clamp(WINDOW_MATERIAL_OPACITY_MIN, WINDOW_MATERIAL_OPACITY_MAX);
    let mut colors = base;

    // 界面不透明度直接作用于主题表面原色的 Alpha，不再先生成另一套材质色再缩放。
    // 边框和输入框边界保留主题自身的 Alpha，避免低透明度时失去轮廓。
    let scale_alpha = |color: Hsla| with_alpha(color, color.alpha * window_alpha);
    // 菜单和下拉菜单的悬停背景使用 accent，也属于窗口表面的一部分。
    colors.accent = scale_alpha(base.accent);
    colors.background = scale_alpha(base.background);
    colors.title_bar = scale_alpha(base.title_bar);
    colors.sidebar = scale_alpha(base.sidebar);
    colors.tab_bar = scale_alpha(base.tab_bar);
    colors.popover = scale_alpha(base.popover);
    colors.secondary = scale_alpha(base.secondary);
    colors.input = base.input;
    colors.secondary_hover = scale_alpha(base.secondary_hover);
    colors.secondary_active = scale_alpha(base.secondary_active);
    colors.button = scale_alpha(base.button);
    colors.button_hover = scale_alpha(base.button_hover);
    colors.button_active = scale_alpha(base.button_active);
    colors.button_secondary = scale_alpha(base.button_secondary);
    colors.button_secondary_hover = scale_alpha(base.button_secondary_hover);
    colors.button_secondary_active = scale_alpha(base.button_secondary_active);
    colors.list = scale_alpha(base.list);
    colors.list_even = scale_alpha(base.list_even);
    colors.list_head = scale_alpha(base.list_head);
    colors.list_hover = scale_alpha(base.list_hover);
    colors.list_active = scale_alpha(base.list_active);
    colors.table = scale_alpha(base.table);
    colors.table_even = scale_alpha(base.table_even);
    colors.table_head = scale_alpha(base.table_head);
    colors.table_foot = scale_alpha(base.table_foot);
    colors.table_hover = scale_alpha(base.table_hover);
    colors.table_active = scale_alpha(base.table_active);
    colors.tab = scale_alpha(base.tab);
    colors.tab_active = scale_alpha(base.tab_active);
    colors.tab_bar_segmented = scale_alpha(base.tab_bar_segmented);
    colors.accordion = scale_alpha(base.accordion);
    colors.group_box = scale_alpha(base.group_box);
    colors.skeleton = scale_alpha(base.skeleton);
    colors.progress_bar = scale_alpha(base.progress_bar);
    colors.slider_bar = scale_alpha(base.slider_bar);
    colors.switch = scale_alpha(base.switch);
    colors.scrollbar = scale_alpha(base.scrollbar);
    colors.scrollbar_thumb = scale_alpha(base.scrollbar_thumb);
    colors.scrollbar_thumb_hover = scale_alpha(base.scrollbar_thumb_hover);
    colors.border = base.border;
    colors.window_border = base.window_border;
    colors.title_bar_border = base.title_bar_border;
    colors.sidebar_border = base.sidebar_border;
    colors.table_row_border = base.table_row_border;
    colors.table_active_border = base.table_active_border;
    colors.list_active_border = base.list_active_border;

    colors
}

pub(crate) fn effective_brightness(appearance: &AppearanceConfig) -> f32 {
    appearance
        .window_material_brightness
        .filter(|brightness| brightness.is_finite())
        .unwrap_or(WINDOW_MATERIAL_BRIGHTNESS_DEFAULT)
        .clamp(
            WINDOW_MATERIAL_BRIGHTNESS_MIN,
            WINDOW_MATERIAL_BRIGHTNESS_MAX,
        )
}

pub(crate) fn current_brightness(cx: &App) -> f32 {
    let appearance = crate::config_runtime::appearance_config(cx);
    effective_brightness(&appearance)
}

fn with_alpha(mut color: Hsla, alpha: f32) -> Hsla {
    color.alpha = alpha.clamp(0.0, 1.0);
    color
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WINDOW_MATERIAL_OPACITY_DEFAULT;

    #[test]
    fn disabled_material_keeps_a_transparent_gpui_window() {
        assert_eq!(
            background_appearance(WindowMaterial::Disabled),
            WindowBackgroundAppearance::Transparent
        );
    }

    #[test]
    fn acrylic_uses_gpui_blurred_appearance() {
        assert_eq!(
            background_appearance(WindowMaterial::Acrylic),
            WindowBackgroundAppearance::Blurred
        );
    }

    #[test]
    fn material_colors_derive_surfaces_from_base_theme() {
        let base = ThemeColor::default();
        let colors = material_colors(base, WINDOW_MATERIAL_OPACITY_DEFAULT);

        assert_eq!(colors.background.alpha, WINDOW_MATERIAL_OPACITY_DEFAULT);
        assert_eq!(colors.title_bar.alpha, WINDOW_MATERIAL_OPACITY_DEFAULT);
        assert_eq!(colors.popover.alpha, WINDOW_MATERIAL_OPACITY_DEFAULT);
        assert_eq!(colors.input.alpha, base.input.alpha);
        assert_eq!(colors.border.alpha, base.border.alpha);
        assert_eq!(colors.window_border.alpha, base.window_border.alpha);
    }

    #[test]
    fn material_opacity_preserves_border_alpha() {
        let base = ThemeColor::default();
        let colors = material_colors(base, WINDOW_MATERIAL_OPACITY_DEFAULT);
        assert_eq!(colors.border.alpha, base.border.alpha);
        assert_eq!(colors.input.alpha, base.input.alpha);
    }

    #[test]
    fn material_opacity_is_the_window_surface_alpha() {
        let base = ThemeColor::default();
        let half = material_colors(base, 0.5);

        assert_eq!(half.background.alpha, 0.5);
        // 其余表面按同一比例缩放，保持各表面之间的层次关系。
        assert_eq!(half.title_bar.alpha, 0.5);
        assert_eq!(half.popover.alpha, 0.5);
        assert_eq!(half.input.alpha, base.input.alpha);
        assert_eq!(half.border.alpha, base.border.alpha);
    }

    #[test]
    fn full_material_opacity_makes_the_window_surface_opaque() {
        let colors = material_colors(ThemeColor::default(), 1.0);
        assert_eq!(colors.background.alpha, 1.0);
    }

    #[test]
    fn material_opacity_is_clamped_to_the_configured_range() {
        let base = ThemeColor::default();

        let lowest = material_colors(base, 0.0);
        assert_eq!(lowest.background.alpha, WINDOW_MATERIAL_OPACITY_MIN);

        let highest = material_colors(base, 10.0);
        assert_eq!(highest.background.alpha, WINDOW_MATERIAL_OPACITY_MAX);
    }
}
