use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt, AnyElement, Context, ElementId, IntoElement,
    MouseButton, SpringAnimation, SpringConfig, Window, deferred, div, img, px,
};
use gpui_component::input::Input;
use gpui_component::Sizable as _;
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{
    ActiveTheme as _, InteractiveElementExt as _, h_flex, v_flex,
};

use super::{
    SETTINGS_TITLE_BAR_HEIGHT, SettingsPage, SettingsWindowView,
};
use crate::{config_runtime, i18n};
use crate::ui::hover_icon_path;
use crate::ui::common::window_title_bar;
use gpui::ColorExt as _;

const SETTINGS_NAV_BUTTON_HEIGHT: f32 = 34.0;
const SETTINGS_NAV_GAP: f32 = 4.0;
const SETTINGS_NAV_PADDING: f32 = 12.0;
const SETTINGS_APP_INFO_ICON_SIZE: f32 = 40.0;

fn spring<T: gpui::SpringTarget>(target: T) -> SpringAnimation<T> {
    SpringAnimation::new(SpringConfig::new(420.0, 36.0, 1.0)).to(target)
}

fn navigation_active_indicator(
    id: impl Into<ElementId>,
    index: usize,
    color: gpui::Hsla,
    animations_enabled: bool,
) -> AnyElement {
    let top = SETTINGS_NAV_PADDING
        + index as f32 * (SETTINGS_NAV_BUTTON_HEIGHT + SETTINGS_NAV_GAP);
    let id = id.into();
    let indicator = div()
        .absolute()
        .top(px(top))
        .left(px(SETTINGS_NAV_PADDING))
        .right(px(SETTINGS_NAV_PADDING))
        .h(px(SETTINGS_NAV_BUTTON_HEIGHT))
        .child({
            let bar = div()
                .size_full()
                .rounded(px(6.0))
                .bg(color)
                .shadow_sm();
            if animations_enabled {
                bar.with_animation(
                    (id.clone(), format!("stretch-{index}")),
                    Animation::new(Duration::from_millis(150)).with_easing(gpui::ease_in_out),
                    |this, progress| {
                        let expansion = (std::f32::consts::PI * progress).sin() * 0.9;
                        let vertical_margin = (2.0 * (1.0 - progress) - expansion).max(0.0);
                        this.my(px(vertical_margin))
                    },
                )
                .into_any_element()
            } else {
                bar.into_any_element()
            }
        });
    if animations_enabled {
        indicator
            .with_spring((id, "top"), spring(px(top)), |this, value| this.top(value))
            .into_any_element()
    } else {
        indicator.into_any_element()
    }
}

impl SettingsWindowView {
    pub(super) fn navigation_button(
        &self,
        page: SettingsPage,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.active_page == page;
        let view = cx.entity().clone();
        let translator = i18n::translator(cx);
        let theme = cx.theme().colors;
        let icon_color = if active {
            theme.sidebar_primary_foreground
        } else {
            theme.sidebar_foreground
        };

        div()
            .id(page.id())
            .group(format!("settings-page-{}", page.id()))
            .h(px(SETTINGS_NAV_BUTTON_HEIGHT))
            .w_full()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_3()
            .rounded(px(6.0))
            .text_sm()
            .cursor_pointer()
            .when(active, |this| this.text_color(theme.sidebar_primary_foreground))
            .when(!active, |this| {
                this.text_color(theme.sidebar_foreground).hover(|style| {
                    style
                        .bg(theme.sidebar_accent)
                        .text_color(theme.sidebar_accent_foreground)
                })
            })
            .on_click(move |_, _, cx| {
                let _ = view.update(cx, |view, cx| {
                    if view.active_page != page {
                        view.active_page = page;
                        view.clear_search_highlight();
                        cx.notify();
                    }
                });
            })
            .child(
                hover_icon_path(
                    format!("settings-page-{}", page.id()),
                    format!("assets/icons/{}.svg", page.icon_name()),
                    16.0,
                    icon_color,
                ),
            )
            .child(translator.translate(page.title_key()))
            .when(page == SettingsPage::About && crate::services::updates::UpdateService::available(cx).is_some(), |view| {
                view.child(div().text_xs().text_color(theme.success)
                    .bg(theme.success.opacity(0.12)).rounded(px(3.0)).px_1()
                    .child(translator.translate("updates.tag")))
            })
            .into_any_element()
    }

    pub(super) fn render_title_bar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme().colors;
        let translator = i18n::translator(cx);
        let title = translator.translate("settings.window.title");
        let search_view = cx.entity().clone();
        let search_box = h_flex()
            .id("settings-title-search-box")
            .w(px(320.0))
            .min_w(px(320.0))
            .max_w(px(320.0))
            .flex_shrink_0()
            .h(px(30.0))
            .items_center()
            // 阻止标题栏下方的 Drag 命中框覆盖输入框的原生窗口命中测试。
            .occlude()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                let _ = search_view.update(cx, |view, cx| {
                    if !view.settings_search_input.read(cx).value().trim().is_empty() {
                        view.search_results_visible = true;
                        cx.notify();
                    }
                });
            })
            .child(
                Input::new(&self.settings_search_input)
                    .small()
                    .w_full()
                    .min_w(px(320.0))
                    .cleanable(true)
                    .aria_label(translator.translate("settings.search.placeholder"))
                    .prefix(hover_icon_path(
                        "settings-search-input-icon",
                        "assets/icons/search.svg",
                        14.0,
                        theme.muted_foreground,
                    )),
            );

        window_title_bar(title, window, cx)
            .child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    .left(px(0.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(search_box),
            )
    }

    pub(super) fn render_search_results(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().colors;
        let translator = i18n::translator(cx);
        let query = self.settings_search_input.read(cx).value().to_string();
        if !self.search_results_visible || query.trim().is_empty() {
            return div().into_any_element();
        }
        let results = super::search_targets(&translator, &query);
        let has_results = !results.is_empty();
        let view = cx.entity().clone();
        let panel_content = if results.is_empty() {
            div()
                .px_3()
                .py_2()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(translator.translate("settings.search.no_results"))
                .into_any_element()
        } else {
            v_flex()
                .id("settings-search-results-scroll")
                .w_full()
                .max_h(px(360.0))
                .min_h_0()
                .track_scroll(&self.search_results_scroll_handle)
                .overflow_y_scroll()
                .lock_scroll_axis()
                .children(results.into_iter().map(|target| {
                    let target_for_click = target.clone();
                    let view = view.clone();
                    h_flex()
                        .id(format!(
                            "settings-search-result-{}-{}",
                            target.page.id(),
                            target.title
                        ))
                        .w_full()
                        .min_h(px(54.0))
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_3()
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.accent))
                        .on_click(move |_, _window, cx| {
                            let _ = view.update(cx, |view, cx| {
                                view.select_search_result(target_for_click.clone(), cx);
                            });
                        })
                        .child(
                            v_flex()
                                .min_w_0()
                                .gap_0()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(theme.foreground)
                                        .truncate()
                                        .child(target.title),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .truncate()
                                        .child(if target.description.is_empty() {
                                            translator.translate(target.page.title_key())
                                        } else {
                                            target.description
                                        }),
                                ),
                        )
                        .into_any_element()
                }))
                .into_any_element()
        };
        let view_for_outside = view.clone();
        let panel = div()
            .relative()
            .w(px(320.0))
            .max_h(px(360.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(move |_, _, cx| {
                let _ = view_for_outside.update(cx, |view, cx| {
                    if view.search_results_visible {
                        view.search_results_visible = false;
                        cx.notify();
                    }
                });
            })
            .child(panel_content)
            .when(has_results, |this| {
                this.vertical_scrollbar(&self.search_results_scroll_handle)
            });
        deferred(
            div()
                .absolute()
                .top(px(SETTINGS_TITLE_BAR_HEIGHT))
                .left(px(0.0))
                .w_full()
                .flex()
                .justify_center()
                .child(panel),
        )
        .priority(3)
        .into_any_element()
    }

    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().colors;
        let animations_enabled = config_runtime::animation_enabled(
            cx,
            crate::config::AnimationKind::SingleSelectIndicator,
        );
        let active_index = SettingsPage::ALL
            .into_iter()
            .position(|page| page == self.active_page);
        v_flex()
            .h_full()
            .flex_none()
            .gap_1()
            .border_r_1()
            .border_color(theme.sidebar_border)
            .bg(theme.title_bar)
            .p_3()
            .relative()
            .when_some(active_index, |this, index| {
                this.child(navigation_active_indicator(
                    "settings-sidebar-active-indicator",
                    index,
                    theme.sidebar_primary,
                    animations_enabled,
                ))
            })
            .children(
                SettingsPage::ALL
                    .into_iter()
                    .map(|page| self.navigation_button(page, cx)),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(theme.sidebar_border)
                    .pt_3()
                    .child(
                        img("assets/icon1024.png")
                            .size(px(SETTINGS_APP_INFO_ICON_SIZE)),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_0()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.sidebar_foreground)
                                    .truncate()
                                    .child(env!("CARGO_PKG_NAME")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
                            ),
                    ),
            )
    }

    pub(super) fn render_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.active_page;
        let content = match self.active_page {
            SettingsPage::General => self.render_general(cx),
            SettingsPage::Clipboard => self.render_clipboard(cx),
            SettingsPage::ApplicationFilter => self.render_application_filter(cx),
            SettingsPage::QuickPaste => self.render_quick_paste(cx),
            SettingsPage::Sound => self.render_sound(cx),
            SettingsPage::Shortcuts => self.render_shortcuts(cx),
            SettingsPage::Personalization => self.render_personalization(cx),
            SettingsPage::DataManagement => self.render_data_management(cx),
            SettingsPage::About => self.render_about(cx),
        };

        let content = if page == SettingsPage::Shortcuts {
            v_flex()
                .id(format!("settings-page-content-{}", page.id()))
                .size_full()
                .min_w_0()
                .p(px(18.0))
                .child(content)
                .into_any_element()
        } else {
            let scroll_handle = &self.page_scroll_handles[page.index()];
            let scroll_content = v_flex()
                .id(format!("settings-page-content-{}", page.id()))
                .size_full()
                .min_w_0()
                .p(px(18.0))
                .track_scroll(scroll_handle)
                .overflow_y_scroll()
                .lock_scroll_axis()
                .child(content);
            div()
                .id(format!("settings-page-scroll-{}", page.id()))
                .relative()
                .size_full()
                .min_w_0()
                .overflow_hidden()
                .child(scroll_content)
                .vertical_scrollbar(scroll_handle)
                .into_any_element()
        };
        if config_runtime::animation_enabled(
            cx,
            crate::config::AnimationKind::SettingsPageTransition,
        ) {
            div()
                .size_full()
                .child(content)
                .with_animation(
                    format!("settings-page-transition-{}", page.id()),
                    Animation::new(Duration::from_millis(180))
                        .with_easing(gpui::ease_out_quint()),
                    |this, progress| {
                        this.left(px(-24.0 * (1.0 - progress))).opacity(progress)
                    },
                )
                .into_any_element()
        } else {
            content
        }
    }
}
