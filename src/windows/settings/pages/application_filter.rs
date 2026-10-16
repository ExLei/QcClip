use crate::ui::scale::ui_rem;

use super::super::{SettingsPage, notify_settings_error};
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, FontWeight, RenderImage, div, px};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::Input;
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::switch::Switch;
use gpui_component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use std::sync::Arc;

use super::super::{
    SettingsWindowView,
    components::{settings_error_row, settings_page_header},
};
use crate::{
    config::{ApplicationFilterApplication, ApplicationFilterConfig},
    config_runtime, i18n,
    services::application_filter::{self, ApplicationInfo},
    ui::icon,
};

impl SettingsWindowView {
    pub(in crate::windows::settings) fn render_application_filter(
        &self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().colors;
        let translator = i18n::translator(cx);
        let config = config_runtime::application_filter_config(cx);
        let view = cx.entity().clone();

        let enabled = Switch::new("settings-application-filter-enabled")
            .large()
            .checked(config.enabled)
            .accessibility_label(translator.translate("settings.application_filter.enabled"))
            .on_click({
                let view = view.clone();
                move |checked, _, cx| {
                    let mut config = config_runtime::application_filter_config(cx);
                    config.enabled = *checked;
                    persist_application_filter(view.clone(), config, cx);
                }
            });

        let disable_clipboard = filter_checkbox(
            "settings-application-filter-clipboard",
            config.disable_clipboard_monitor,
            translator.translate("settings.application_filter.disable_clipboard"),
            view.clone(),
            |config, checked| config.disable_clipboard_monitor = checked,
        );
        let disable_hotkeys = filter_checkbox(
            "settings-application-filter-hotkeys",
            config.disable_global_hotkeys,
            translator.translate("settings.application_filter.disable_hotkeys"),
            view.clone(),
            |config, checked| config.disable_global_hotkeys = checked,
        );
        let disable_edge_hide = filter_checkbox(
            "settings-application-filter-edge",
            config.disable_edge_hide,
            translator.translate("settings.application_filter.disable_edge_hide"),
            view.clone(),
            |config, checked| config.disable_edge_hide = checked,
        );

        let manual_input = h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2()
            .min_h(ui_rem(48.0))
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme.border)
            .px_4()
            .py_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.application_filter_input).small().w_full()),
            )
            .child({
                let view = view.clone();
                Button::new("settings-application-filter-add-manual")
                    .primary()
                    .small()
                    .label(translator.translate("settings.application_filter.add"))
                    .on_click(move |_, window, cx| {
                        let query = view
                            .read(cx)
                            .application_filter_input
                            .read(cx)
                            .value()
                            .to_string();
                        let result = application_filter::resolve_manual_application(&query)
                            .and_then(|application| add_application(application, cx));
                        view.update(cx, |view, cx| {
                            view.application_filter_error = notify_settings_error(
                                SettingsPage::ApplicationFilter,
                                result.err().map(|_| {
                                    i18n::translator(cx)
                                        .translate("settings.application_filter.manual_error")
                                }),
                                cx,
                            );
                            if view.application_filter_error.is_none() {
                                view.application_filter_input.update(cx, |input, cx| {
                                    input.set_value("", window, cx);
                                });
                            }
                            cx.notify();
                        });
                    })
            });

        let refresh_view = view.clone();
        let refresh_button = Button::new("settings-application-filter-refresh")
            .ghost()
            .small()
            .icon(icon("refresh", 14.0, theme.muted_foreground))
            .accessibility_label(translator.translate("settings.application_filter.refresh"))
            .disabled(self.application_filter_loading)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |view, cx| view.refresh_application_filter_apps(cx));
            });

        let filtered = config.applications.clone();
        let available_rows = self
            .application_filter_apps
            .iter()
            .filter(|application| {
                !filtered.iter().any(|filter| {
                    application_filter::same_application(&application.application, filter)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        let filtered_rows = filtered
            .iter()
            .map(|filter| {
                self.application_filter_apps
                    .iter()
                    .find(|application| {
                        application_filter::same_application(&application.application, filter)
                    })
                    .cloned()
                    .unwrap_or_else(|| ApplicationInfo::from_application(filter.clone()))
            })
            .collect::<Vec<_>>();

        let mut section = v_flex()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .overflow_hidden()
            .rounded(px(8.0))
            .border_1()
            .border_color(theme.border)
            .bg(theme.title_bar)
            .child(settings_page_header(
                theme,
                translator.translate("settings.page.application_filter"),
                translator.translate("settings.application_filter.description"),
                div(),
            ))
            .child(
                self.setting_row(
                    theme,
                    translator.translate("settings.application_filter.enabled"),
                    Some(
                        translator
                            .translate("settings.application_filter.enabled_description")
                            .into(),
                    ),
                    enabled,
                ),
            )
            .child(
                self.highlight_element(
                    theme,
                    translator.translate("settings.application_filter.scope"),
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .flex_shrink_0()
                        .gap_2()
                        .border_b_1()
                        .border_color(theme.border)
                        .px_4()
                        .py_3()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.foreground)
                                .child(translator.translate("settings.application_filter.scope")),
                        )
                        .child(div().text_xs().text_color(theme.muted_foreground).child(
                            translator.translate("settings.application_filter.scope_description"),
                        ))
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .flex_wrap()
                                .items_center()
                                .gap_4()
                                .children([
                                    self.highlight_element(
                                        theme,
                                        translator.translate(
                                            "settings.application_filter.disable_clipboard",
                                        ),
                                        disable_clipboard,
                                    ),
                                    self.highlight_element(
                                        theme,
                                        translator.translate(
                                            "settings.application_filter.disable_hotkeys",
                                        ),
                                        disable_hotkeys,
                                    ),
                                    self.highlight_element(
                                        theme,
                                        translator.translate(
                                            "settings.application_filter.disable_edge_hide",
                                        ),
                                        disable_edge_hide,
                                    ),
                                ]),
                        ),
                ),
            )
            .child(self.highlight_element(
                theme,
                translator.translate("settings.application_filter.manual"),
                manual_input,
            ))
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .p_3()
                    .items_start()
                    .child(application_list_panel(
                        "settings-application-filter-available",
                        translator.translate("settings.application_filter.available"),
                        available_rows,
                        view.clone(),
                        false,
                        Some(refresh_button.into()),
                        translator.translate("settings.application_filter.available_empty"),
                        theme,
                    ))
                    .child(application_list_panel(
                        "settings-application-filter-filtered",
                        translator.translate("settings.application_filter.filtered"),
                        filtered_rows,
                        view,
                        true,
                        None,
                        translator.translate("settings.application_filter.filtered_empty"),
                        theme,
                    )),
            );

        if self.application_filter_loading {
            section = section.child(
                div()
                    .px_4()
                    .pb_3()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(translator.translate("settings.application_filter.loading")),
            );
        }
        if let Some(error) = &self.application_filter_error {
            section = section.child(settings_error_row(theme, error));
        }
        section.into_any_element()
    }
}

fn filter_checkbox<F>(
    id: &'static str,
    checked: bool,
    label: String,
    view: gpui::Entity<SettingsWindowView>,
    update: F,
) -> impl IntoElement
where
    F: Fn(&mut ApplicationFilterConfig, bool) + 'static,
{
    Checkbox::new(id)
        .label(label.clone())
        .checked(checked)
        .accessibility_label(label)
        .on_click(move |checked, _, cx| {
            let mut config = config_runtime::application_filter_config(cx);
            update(&mut config, *checked);
            persist_application_filter(view.clone(), config, cx);
        })
}

fn persist_application_filter(
    view: gpui::Entity<SettingsWindowView>,
    config: ApplicationFilterConfig,
    cx: &mut App,
) {
    let error = config_runtime::set_application_filter_config(config, cx)
        .err()
        .map(|error| {
            tracing::error!("保存应用过滤设置失败：{error}");
            i18n::translator(cx).translate("settings.application_filter.save_error")
        });
    view.update(cx, |view, cx| {
        view.application_filter_error =
            notify_settings_error(SettingsPage::ApplicationFilter, error, cx);
        cx.notify();
    });
}

fn add_application(application: ApplicationFilterApplication, cx: &mut App) -> Result<(), String> {
    let mut config = config_runtime::application_filter_config(cx);
    if config
        .applications
        .iter()
        .any(|filter| application_filter::same_application(&application, filter))
    {
        return Ok(());
    }
    config.applications.push(application);
    config_runtime::set_application_filter_config(config, cx)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn application_list_panel(
    id: &'static str,
    title: String,
    applications: Vec<ApplicationInfo>,
    view: gpui::Entity<SettingsWindowView>,
    remove: bool,
    header_action: Option<AnyElement>,
    empty_label: String,
    theme: gpui_component::ThemeColor,
) -> AnyElement {
    let rows = if applications.is_empty() {
        v_flex()
            .w_full()
            .h(ui_rem(150.0))
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(empty_label)
            .into_any_element()
    } else {
        v_flex()
            .w_full()
            .h(ui_rem(150.0))
            .gap_1()
            .overflow_y_scrollbar()
            .children(
                applications
                    .into_iter()
                    .enumerate()
                    .map(|(index, application)| {
                        application_row(index, application, view.clone(), remove, theme)
                    }),
            )
            .into_any_element()
    };
    v_flex()
        .id(id)
        .flex_1()
        .min_w_0()
        .gap_2()
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .p_3()
        .child({
            let mut header = h_flex().w_full().items_center().child(
                div()
                    .flex_1()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.foreground)
                    .child(title),
            );
            if let Some(action) = header_action {
                header = header.child(action);
            }
            header
        })
        .child(rows)
        .into_any_element()
}

fn application_row(
    index: usize,
    application: ApplicationInfo,
    view: gpui::Entity<SettingsWindowView>,
    remove: bool,
    theme: gpui_component::ThemeColor,
) -> AnyElement {
    let identity = application.application.clone();
    let label = if identity.display_name.is_empty() {
        identity.process_name.clone()
    } else {
        identity.display_name.clone()
    };
    let icon_element = application
        .icon
        .map(|image: Arc<RenderImage>| {
            gpui::img(image)
                .size(ui_rem(22.0))
                .object_fit(gpui::ObjectFit::Contain)
                .into_any_element()
        })
        .unwrap_or_else(|| icon("app-window", 20.0, theme.muted_foreground).into_any_element());
    h_flex()
        .id(("settings-application-filter-row", index))
        .w_full()
        .h(ui_rem(32.0))
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .px_2()
        .rounded(px(5.0))
        .cursor_pointer()
        .hover(|style| style.bg(theme.accent))
        .on_click(move |_, _, cx| {
            let mut config = config_runtime::application_filter_config(cx);
            if remove {
                config
                    .applications
                    .retain(|filter| !application_filter::same_application(&identity, filter));
            } else if !config
                .applications
                .iter()
                .any(|filter| application_filter::same_application(&identity, filter))
            {
                config.applications.push(identity.clone());
            }
            persist_application_filter(view.clone(), config, cx);
        })
        .child(
            div()
                .size(ui_rem(24.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(icon_element),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .text_sm()
                .text_color(theme.foreground)
                .child(label),
        )
        .child(icon("chevron-right", 14.0, theme.muted_foreground))
        .into_any_element()
}
