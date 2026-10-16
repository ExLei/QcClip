use std::time::Duration;

use gpui::{
    App, DurationWithEasing, Hsla, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, Styled, Window, div, px, svg,
};
use gpui_component::{ButtonAnimationConfig, Icon};

use crate::ui::scale::ui_rem;

pub fn icon(name: &str, size: f32, color: Hsla) -> Icon {
    Icon::empty()
        .path(format!("assets/icons/{name}.svg"))
        .size(ui_rem(size))
        .text_color(color)
}

/// 创建带悬停缩放动画的图标，外层槽位固定以避免影响按钮布局。
pub fn hover_icon(
    group: impl Into<SharedString>,
    name: &str,
    size: f32,
    color: Hsla,
) -> impl IntoElement {
    hover_icon_path(group, format!("assets/icons/{name}.svg"), size, color)
}

/// 创建带悬停缩放动画的指定路径图标。
pub fn hover_icon_path(
    group: impl Into<SharedString>,
    path: impl Into<SharedString>,
    size: f32,
    color: Hsla,
) -> impl IntoElement {
    HoverIcon {
        group: group.into(),
        path: path.into(),
        size,
        color,
    }
}

#[derive(gpui::IntoElement)]
struct HoverIcon {
    group: SharedString,
    path: SharedString,
    size: f32,
    color: gpui::Hsla,
}

impl RenderOnce for HoverIcon {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let config = ButtonAnimationConfig::current(cx);
        let icon_size = px(self.size * crate::ui::scale::theme_scale(cx));
        let icon = svg()
            .path(self.path)
            .text_color(self.color)
            .size(icon_size)
            .flex_none();
        // 独立菜单弹出后，主窗口中的 group_hover 命中状态可能滞留。
        // 菜单存在、菜单刚关闭且鼠标未移动，或鼠标已离开当前窗口时不创建缩放过渡，
        // 避免过渡任务持续刷新整个应用。
        let icon = if window.is_window_hovered()
            && crate::windows::menu::MenuService::hover_animation_allowed(cx)
            && config.icon_hover_enabled
            && crate::config_runtime::animation_enabled(
                cx,
                crate::config::AnimationKind::IconHoverScale,
            ) {
            let motion =
                Duration::from_millis(config.icon_hover_duration_ms).with_easing(gpui::ease_in_out);
            icon.id(format!("{}-icon", self.group))
                .transitions(|transitions| transitions.size(motion))
                .group_hover(self.group, |style| {
                    style.size(icon_size * config.icon_hover_scale)
                })
                .into_any_element()
        } else {
            icon.into_any_element()
        };

        div()
            .size(icon_size)
            .flex()
            .items_center()
            .justify_center()
            .child(icon)
    }
}
