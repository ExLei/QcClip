use super::super::{SettingsPage, notify_settings_error};
use crate::ui::scale::ui_rem;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, FontWeight, Window, div};
use std::rc::Rc;

use super::super::{
    SettingsWindowView,
    components::{clickable_setting_row, setting_row},
};
use crate::{
    config::{NotificationConfig, NotificationKind, NotificationPosition},
    config_runtime, i18n,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::slider::{Slider, SliderEvent, SliderState};
use gpui_component::switch::Switch;
use gpui_component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};

impl SettingsWindowView {
    /// “通知与音效”页中的通知分组：总开关与通知偏好入口。
    pub(super) fn render_notification_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme().colors;
        let translator = i18n::translator(cx);
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
        let view = cx.entity().clone();
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
                    view.notification_error =
                        notify_settings_error(SettingsPage::Notifications, error, cx);
                    cx.notify();
                });
            });
        vec![
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
            self.highlight_element(
                theme,
                translator.translate("settings.notifications.title"),
                clickable_setting_row(
                    "settings-notification-preferences",
                    theme,
                    translator.translate("settings.notifications.title"),
                    Some(notification_summary.into()),
                    move |_, window, cx| open_notification_preferences(view.clone(), window, cx),
                ),
            ),
        ]
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

pub(super) fn open_notification_preferences(
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
                        view.notification_error =
                            notify_settings_error(SettingsPage::Notifications, error, cx);
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
                                view.notification_error =
                                    notify_settings_error(SettingsPage::Notifications, error, cx);
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
                            view.notification_error =
                                notify_settings_error(SettingsPage::Notifications, error, cx);
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
            .map(|this| crate::window_material::sheet_surface(this, cx))
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
