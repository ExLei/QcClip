use super::super::{SettingsPage, notify_settings_error};
use crate::ui::scale::ui_rem;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, PromptButton, PromptLevel, Window};

use super::super::{
    SettingsWindowView,
    components::{clickable_setting_row, setting_row, settings_page_header},
};
use std::{cell::Cell, rc::Rc};

use super::super::{SETTINGS_DROPDOWN_HEIGHT, SETTINGS_DROPDOWN_WIDTH};
use crate::{
    config::{
        InterfaceLanguage, NotificationConfig, NotificationKind, NotificationPosition,
        WindowMaterial,
    },
    config_runtime, i18n,
    platform::elevation,
    platform::window as platform_window,
    ui::hover_icon,
    windows::menu::{DropdownAnchor, DropdownItem, DropdownItemId, DropdownModel, MenuService},
};
use gpui::{Bounds, FontWeight, Pixels, div, px};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::slider::{Slider, SliderEvent, SliderState};
use gpui_component::switch::Switch;
use gpui_component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};

impl SettingsWindowView {
    pub(in crate::windows::settings) fn render_general(
        &self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().colors;
        let translator = i18n::translator(cx);
        let language = config_runtime::interface_language(cx);
        let launch_at_login = config_runtime::launch_at_login(cx);
        let run_as_administrator = config_runtime::run_as_administrator(cx);
        let notifications_enabled = config_runtime::global_notification_enabled(cx);
        let notification_preferences = config_runtime::notification_preferences(cx);
        let notification_summary = if notifications_enabled {
            translator.translate_with_args(
                "settings.notifications.summary",
                &[
                    (
                        "enabled",
                        notification_preferences
                            .iter()
                            .filter(|(_, enabled)| *enabled)
                            .count()
                            .to_string(),
                    ),
                    ("total", notification_preferences.len().to_string()),
                ],
            )
        } else {
            translator.translate("settings.notifications.paused")
        };
        let language_label = translator.translate(language_label_key(language));
        let view = cx.entity().clone();
        let reset_error = translator.translate("settings.general.reset_error");
        let reset_view = view.clone();
        let reset_button = Button::new("settings-general-reset")
            .warning()
            .small()
            .label(translator.translate("settings.action.reset_page"))
            .on_click(move |_, _, cx| {
                let result = config_runtime::reset_general_config(cx);
                let error_message = result.as_ref().err().map(|error| {
                    tracing::error!("重置常规设置失败：{error}");
                    reset_error.clone()
                });
                reset_view.update(cx, |view, cx| {
                    view.language_error =
                        notify_settings_error(SettingsPage::General, error_message, cx);
                    cx.notify();
                });
            });
        let language_options = InterfaceLanguage::ALL
            .into_iter()
            .map(|candidate| {
                (
                    candidate,
                    translator.translate(language_label_key(candidate)),
                )
            })
            .collect::<Vec<_>>();
        let language_menu = DropdownModel::new(
            language_options
                .iter()
                .map(|(candidate, label)| {
                    DropdownItem::option(DropdownItemId::new(candidate.code()), label.clone())
                        .selected(*candidate == language)
                })
                .collect(),
        );
        let notification_save_error =
            translator.translate("settings.general.notification_save_error");
        let notification_view = view.clone();
        let notification_switch = Switch::new("settings-general-global-notifications")
            .large()
            .checked(notifications_enabled)
            .accessibility_label(translator.translate("settings.general.global_notifications"))
            .on_click(move |checked, _, cx| {
                let error = config_runtime::set_global_notification_enabled(*checked, cx)
                    .err()
                    .map(|error| {
                        tracing::error!("保存全局通知设置失败：{error}");
                        notification_save_error.clone()
                    });
                notification_view.update(cx, |view, cx| {
                    view.language_error = notify_settings_error(SettingsPage::General, error, cx);
                    cx.notify();
                });
            });
        let launch_save_error = translator.translate("settings.general.launch_save_error");
        let launch_view = view.clone();
        let launch_switch = Switch::new("settings-general-launch-at-login")
            .large()
            .checked(launch_at_login)
            .accessibility_label(translator.translate("settings.general.launch_at_login"))
            .on_click(move |checked, _, cx| {
                let error = config_runtime::set_launch_at_login(*checked, cx)
                    .err()
                    .map(|error| {
                        tracing::error!("保存开机自启设置失败：{error}");
                        launch_save_error.clone()
                    });
                launch_view.update(cx, |view, cx| {
                    view.language_error = notify_settings_error(SettingsPage::General, error, cx);
                    cx.notify();
                });
            });
        let administrator_save_error =
            translator.translate("settings.general.administrator_save_error");
        let administrator_view = view.clone();
        let administrator_switch = Switch::new("settings-general-run-as-administrator")
            .large()
            .checked(run_as_administrator)
            .accessibility_label(translator.translate("settings.general.run_as_administrator"))
            .on_click(move |checked, window, cx| {
                handle_run_as_administrator_switch(
                    *checked,
                    administrator_view.clone(),
                    administrator_save_error.clone(),
                    window,
                    cx,
                );
            });
        let save_error = translator.translate("settings.general.save_error");
        let trigger_bounds = Rc::new(Cell::new(None::<Bounds<Pixels>>));
        let trigger_bounds_for_paint = trigger_bounds.clone();
        let trigger_bounds_for_click = trigger_bounds.clone();
        let selector = div()
            .w(ui_rem(SETTINGS_DROPDOWN_WIDTH))
            .on_children_prepainted(move |bounds, _, _| {
                if let Some(bounds) = bounds.first().copied() {
                    trigger_bounds_for_paint.set(Some(bounds));
                }
            })
            .child(
                div()
                    .id("settings-language-select")
                    .group("settings-language-select")
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(ui_rem(SETTINGS_DROPDOWN_HEIGHT))
                    .w_full()
                    .gap_2()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(theme.input)
                    .bg(theme.background)
                    .px_3()
                    .text_sm()
                    .cursor_pointer()
                    .hover(|style| style.border_color(theme.primary).bg(theme.accent))
                    .active(|style| style.border_color(theme.primary_active))
                    .on_click(move |_, window, cx| {
                        let Some(bounds) = trigger_bounds_for_click.get() else {
                            return;
                        };
                        // 设置窗口可能刚跨越 DPI 不同的显示器，先同步 GPUI 的窗口状态，
                        // 再把触发器逻辑尺寸转换为物理桌面坐标。
                        window.bounds_changed(cx);
                        let scale_factor = window.scale_factor().max(1.0);
                        let Some(origin) =
                            platform_window::logical_to_physical_desktop(window, bounds.origin)
                        else {
                            return;
                        };
                        let anchor = DropdownAnchor::new(
                            origin.x,
                            origin.y,
                            f32::from(bounds.size.width) * scale_factor,
                            f32::from(bounds.size.height) * scale_factor,
                            scale_factor,
                        );
                        let view = view.clone();
                        let save_error = save_error.clone();
                        MenuService::open_dropdown_menu(
                            language_menu.clone(),
                            anchor,
                            move |action_id, cx| {
                                let Some(candidate) = InterfaceLanguage::ALL
                                    .into_iter()
                                    .find(|candidate| candidate.code() == action_id.as_str())
                                else {
                                    return;
                                };
                                let result = config_runtime::set_interface_language(candidate, cx);
                                let error_message = result.as_ref().err().map(|error| {
                                    tracing::error!("保存界面语言失败：{error}");
                                    save_error.clone()
                                });
                                view.update(cx, |view, cx| {
                                    view.language_error = notify_settings_error(
                                        SettingsPage::General,
                                        error_message,
                                        cx,
                                    );
                                    cx.notify();
                                });
                            },
                            cx,
                        );
                    })
                    .child(div().flex_1().truncate().child(language_label))
                    .child(hover_icon(
                        "settings-language-select",
                        "chevron-down",
                        12.0,
                        theme.muted_foreground,
                    )),
            );

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
                translator.translate("settings.page.general"),
                translator.translate("settings.general.description"),
                reset_button,
            ))
            .child(self.setting_row(
                theme,
                translator.translate("settings.general.interface_language"),
                None,
                selector,
            ))
            .child(
                self.setting_row(
                    theme,
                    translator.translate("settings.general.global_notifications"),
                    Some(
                        translator
                            .translate("settings.general.global_notifications_description")
                            .into(),
                    ),
                    notification_switch,
                ),
            )
            .child(self.highlight_element(
                theme,
                translator.translate("settings.notifications.title"),
                clickable_setting_row(
                    "settings-general-notification-preferences",
                    theme,
                    translator.translate("settings.notifications.title"),
                    Some(notification_summary.into()),
                    {
                        let view = cx.entity();
                        move |_, window, cx| open_notification_preferences(view.clone(), window, cx)
                    },
                ),
            ))
            .child(
                self.setting_row(
                    theme,
                    translator.translate("settings.general.launch_at_login"),
                    Some(
                        translator
                            .translate("settings.general.launch_at_login_description")
                            .into(),
                    ),
                    launch_switch,
                ),
            )
            .child(
                self.setting_row(
                    theme,
                    translator.translate("settings.general.run_as_administrator"),
                    Some(
                        translator
                            .translate("settings.general.run_as_administrator_description")
                            .into(),
                    ),
                    administrator_switch,
                ),
            );
        if let Some(error) = &self.language_error {
            section = section.child(
                div()
                    .border_t_1()
                    .border_color(theme.border)
                    .px_4()
                    .py_2()
                    .text_sm()
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }
        section.into_any_element()
    }
}

fn notification_duration_slider(duration_ms: u32) -> SliderState {
    SliderState::new()
        // 范围设置会立即计算滑块位置，先增大上限，避免下限超过默认上限。
        .max(NotificationConfig::MAX_DURATION_MS as f32)
        .min(NotificationConfig::MIN_DURATION_MS as f32)
        .step(100.0)
        .default_value(duration_ms as f32)
}

fn open_notification_preferences(
    view: Entity<SettingsWindowView>,
    window: &mut Window,
    cx: &mut App,
) {
    let duration_ms = config_runtime::notification_config(cx).duration_ms;
    let (duration_slider, duration_subscription) = view.update(cx, |_, cx| {
        let slider = cx.new(|_| notification_duration_slider(duration_ms));
        let subscription =
            cx.subscribe_in(
                &slider,
                window,
                |view, slider, event, window, cx| match event {
                    SliderEvent::Change(_) => cx.notify(),
                    SliderEvent::Release(value) => {
                        if !config_runtime::global_notification_enabled(cx) {
                            return;
                        }
                        let config = config_runtime::notification_config(cx);
                        let error = config_runtime::set_notification_presentation(
                            config.position,
                            value.end().round() as u32,
                            cx,
                        )
                        .err()
                        .map(|error| {
                            tracing::error!("保存通知显示时长失败：{error}");
                            slider.update(cx, |slider, cx| {
                                slider.set_value(config.duration_ms as f32, window, cx);
                            });
                            i18n::translator(cx)
                                .translate("settings.general.notification_save_error")
                        });
                        view.language_error =
                            notify_settings_error(SettingsPage::General, error, cx);
                        cx.notify();
                    }
                },
            );
        (slider, Rc::new(subscription))
    });
    window.open_sheet(cx, move |sheet, _, cx| {
        // 订阅随偏好面板释放，避免重复打开时累积监听器。
        let _ = &duration_subscription;
        let theme = cx.theme().colors;
        let translator = i18n::translator(cx);
        let enabled = config_runtime::global_notification_enabled(cx);
        let preferences = config_runtime::notification_preferences(cx);
        let notifications = config_runtime::notification_config(cx);
        let appearance = config_runtime::appearance_config(cx);
        let background = gpui::ColorExt::opacity(
            &crate::window_material::excluded_window_theme(cx).background,
            0.9,
        );
        let mut content = v_flex().size_full().min_h_0().overflow_y_scrollbar();
        if !enabled {
            content = content.child(
                div()
                    .px_4()
                    .py_2()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(translator.translate("settings.notifications.paused")),
            );
        }
        let mut position_grid = v_flex().gap_2().w_full();
        let mut positions = NotificationPosition::ALL.into_iter();
        for row_index in 0..3 {
            let mut row = h_flex().w_full().gap_2();
            for column_index in 0..3 {
                if row_index == 1 && column_index == 1 {
                    row = row.child(div().flex_1());
                    continue;
                }
                let position = positions.next().expect("通知方向布局应包含八个位置");
                let label = translator.translate(position.label_key());
                row = row.child(
                    Button::new(format!(
                        "settings-notification-position-{row_index}-{column_index}"
                    ))
                    .small()
                    .flex_1()
                    .label(label)
                    .when(position == notifications.position, |button| {
                        button.primary()
                    })
                    .disabled(!enabled)
                    .on_click({
                        let view = view.clone();
                        move |_, _, cx| {
                            let duration_ms = config_runtime::notification_config(cx).duration_ms;
                            let error = config_runtime::set_notification_presentation(
                                position,
                                duration_ms,
                                cx,
                            )
                            .err()
                            .map(|error| {
                                tracing::error!("保存通知显示位置失败：{error}");
                                i18n::translator(cx)
                                    .translate("settings.general.notification_save_error")
                            });
                            view.update(cx, |view, cx| {
                                view.language_error =
                                    notify_settings_error(SettingsPage::General, error, cx);
                                cx.notify();
                            });
                        }
                    }),
                );
            }
            position_grid = position_grid.child(row);
        }
        content = content
            .child(
                v_flex()
                    .flex_shrink_0()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_sm()
                            .child(translator.translate("settings.notifications.position")),
                    )
                    .child(position_grid),
            )
            .child(
                v_flex()
                    .flex_shrink_0()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .justify_between()
                            .text_sm()
                            .child(translator.translate("settings.notifications.duration"))
                            .child(translator.translate_with_args(
                                "settings.notifications.duration_seconds",
                                &[(
                                    "seconds",
                                    format!(
                                        "{:.1}",
                                        duration_slider.read(cx).value().end() / 1000.0
                                    ),
                                )],
                            )),
                    )
                    .child(Slider::new(&duration_slider).disabled(!enabled).w_full()),
            );
        for (kind, checked) in preferences {
            let group = match kind {
                NotificationKind::Startup => Some("settings.notifications.group.daily"),
                NotificationKind::OperationResult => Some("settings.notifications.group.feedback"),
                NotificationKind::VersionUpdate => Some("settings.notifications.group.updates"),
                _ => None,
            };
            if let Some(group) = group {
                content = content.child(
                    div()
                        .px_4()
                        .pt_4()
                        .pb_2()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.muted_foreground)
                        .child(translator.translate(group)),
                );
            }
            let label = translator.translate(kind.label_key());
            let switch = Switch::new(format!("settings-notification-{}", kind.label_key()))
                .large()
                .checked(checked)
                .disabled(!enabled)
                .accessibility_label(label.clone())
                .on_click({
                    let view = view.clone();
                    move |checked, _, cx| {
                        let error = config_runtime::set_notification_preference(kind, *checked, cx)
                            .err()
                            .map(|error| {
                                tracing::error!("保存通知偏好失败：{error}");
                                i18n::translator(cx)
                                    .translate("settings.general.notification_save_error")
                            });
                        view.update(cx, |view, cx| {
                            view.language_error =
                                notify_settings_error(SettingsPage::General, error, cx);
                            cx.notify();
                        });
                    }
                });
            content = content.child(setting_row(theme, label, None, switch));
        }
        if let Some(error) = view.read(cx).language_error.clone() {
            content = content.child(
                div()
                    .px_4()
                    .py_2()
                    .text_sm()
                    .text_color(theme.danger)
                    .child(error),
            );
        }
        sheet
            .title(translator.translate("settings.notifications.title"))
            .size(ui_rem(420.0))
            .resizable(false)
            .shadow_none()
            .bg(background)
            .when(
                appearance.window_material != WindowMaterial::Disabled,
                |this| this.backdrop_blur(px(18.0)),
            )
            .child(content)
    });
}

#[cfg(test)]
mod tests {
    use super::{NotificationConfig, notification_duration_slider};

    #[test]
    fn notification_duration_slider_initializes_without_panicking() {
        for duration_ms in [
            NotificationConfig::MIN_DURATION_MS,
            NotificationConfig::DEFAULT_DURATION_MS,
            NotificationConfig::MAX_DURATION_MS,
        ] {
            let slider = notification_duration_slider(duration_ms);
            assert_eq!(
                slider.min_value(),
                NotificationConfig::MIN_DURATION_MS as f32
            );
            assert_eq!(
                slider.max_value(),
                NotificationConfig::MAX_DURATION_MS as f32
            );
            assert_eq!(slider.step_value(), 100.0);
            assert_eq!(slider.value().end(), duration_ms as f32);
        }
    }
}

fn handle_run_as_administrator_switch(
    checked: bool,
    view: Entity<SettingsWindowView>,
    save_error: String,
    window: &mut Window,
    cx: &mut App,
) {
    if !checked {
        let error = config_runtime::set_run_as_administrator(false, cx)
            .err()
            .map(|error| {
                tracing::error!("保存管理员启动设置失败：{error}");
                save_error.clone()
            });
        view.update(cx, |view, cx| {
            view.language_error = notify_settings_error(SettingsPage::General, error, cx);
            cx.notify();
        });
        return;
    }

    match elevation::is_elevated() {
        Ok(true) => {
            let error = config_runtime::set_run_as_administrator(true, cx)
                .err()
                .map(|error| {
                    tracing::error!("保存管理员启动设置失败：{error}");
                    save_error.clone()
                });
            view.update(cx, |view, cx| {
                view.language_error = notify_settings_error(SettingsPage::General, error, cx);
                cx.notify();
            });
        }
        Ok(false) => {
            let translator = i18n::translator(cx);
            let title = translator.translate("settings.general.administrator_confirm_title");
            let detail = translator.translate("settings.general.administrator_confirm_detail");
            let cancel = PromptButton::cancel(
                translator.translate("settings.general.administrator_confirm_cancel"),
            );
            let restart_later = PromptButton::new(
                translator.translate("settings.general.administrator_confirm_restart_later"),
            );
            let restart_now = PromptButton::ok(
                translator.translate("settings.general.administrator_confirm_restart_now"),
            );
            let answer = window.prompt(
                PromptLevel::Warning,
                &title,
                Some(&detail),
                &[cancel, restart_later, restart_now],
                cx,
            );
            cx.spawn(async move |cx| {
                let choice = answer.await.unwrap_or(0);
                cx.update(|cx| {
                    let (error, should_quit) = match choice {
                        1 => (
                            config_runtime::set_run_as_administrator(true, cx)
                                .err()
                                .map(|error| {
                                    tracing::error!("保存管理员启动设置失败：{error}");
                                    save_error.clone()
                                }),
                            false,
                        ),
                        2 => match config_runtime::set_run_as_administrator(true, cx) {
                            Err(error) => (
                                {
                                    tracing::error!("保存管理员启动设置失败：{error}");
                                    Some(save_error.clone())
                                },
                                false,
                            ),
                            Ok(_) => match elevation::relaunch_elevated() {
                                Ok(()) => (None, true),
                                Err(error) => {
                                    tracing::error!("请求管理员权限重启失败：{error}");
                                    if let Err(restore_error) =
                                        config_runtime::set_run_as_administrator(false, cx)
                                    {
                                        tracing::error!("恢复管理员启动设置失败：{restore_error}");
                                    }
                                    (Some(save_error.clone()), false)
                                }
                            },
                        },
                        _ => (None, false),
                    };
                    if should_quit {
                        cx.quit();
                        return;
                    }
                    let _ = view.update(cx, |view, cx| {
                        view.language_error =
                            notify_settings_error(SettingsPage::General, error, cx);
                        cx.notify();
                    });
                });
            })
            .detach();
        }
        Err(error) => {
            tracing::error!("读取管理员权限状态失败：{error}");
            view.update(cx, |view, cx| {
                view.language_error =
                    notify_settings_error(SettingsPage::General, Some(save_error), cx);
                cx.notify();
            });
        }
    }
}

const fn language_label_key(language: InterfaceLanguage) -> &'static str {
    match language {
        InterfaceLanguage::ZhCn => "language.zh_cn",
        InterfaceLanguage::EnUs => "language.en_us",
    }
}
