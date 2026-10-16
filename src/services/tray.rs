use std::time::Duration;

use gpui::{App, Global, Point};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::{
    i18n,
    windows::menu::{MenuActionId, MenuIcon, MenuItem, MenuModel, MenuPosition, MenuService},
};

const TRAY_EVENT_POLL_INTERVAL: Duration = Duration::from_millis(16);
const TOGGLE_MAIN_WINDOW_ACTION: &str = "tray.toggle-main-window";
const TOGGLE_CLIPBOARD_MONITOR_ACTION: &str = "tray.toggle-clipboard-monitor";
const TOGGLE_GLOBAL_HOTKEYS_ACTION: &str = "tray.toggle-global-hotkeys";
const SETTINGS_ACTION: &str = "tray.settings";
const RESTART_ACTION: &str = "tray.restart";
const QUIT_ACTION: &str = "tray.quit";
const CHECK_UPDATES_ACTION: &str = "tray.check-updates";
const OPEN_RELEASE_ACTION: &str = "tray.open-release";

pub struct TrayService {
    _icon: TrayIcon,
}

impl Global for TrayService {}

impl TrayService {
    pub fn install(cx: &mut App) {
        let icon = match create_tray_icon() {
            Ok(icon) => icon,
            Err(error) => {
                tracing::error!("{error}");
                return;
            }
        };

        cx.set_global(Self { _icon: icon });
        cx.spawn(async move |cx| {
            let receiver = TrayIconEvent::receiver();
            loop {
                cx.background_executor()
                    .timer(TRAY_EVENT_POLL_INTERVAL)
                    .await;

                let Some(action) = receiver.try_iter().find_map(tray_event_action) else {
                    continue;
                };
                cx.update(|cx| match action {
                    TrayEventAction::ToggleMainWindow => {
                        crate::windows::main::toggle_main_window(cx)
                    }
                    TrayEventAction::OpenMenu(position) => open_menu(position, cx),
                });
            }
        })
        .detach();
    }

    pub(crate) fn contains_point(cx: &App, position: Point<f32>) -> bool {
        let Some(service) = cx.try_global::<Self>() else {
            return false;
        };
        let Some(rect) = service._icon.rect() else {
            return false;
        };

        let x = f64::from(position.x);
        let y = f64::from(position.y);
        x >= rect.position.x
            && x < rect.position.x + f64::from(rect.size.width)
            && y >= rect.position.y
            && y < rect.position.y + f64::from(rect.size.height)
    }
}

fn create_tray_icon() -> Result<TrayIcon, String> {
    let icon = load_tray_icon()?;
    TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip("QcClip")
        .with_menu_on_left_click(false)
        .with_menu_on_right_click(false)
        .build()
        .map_err(|error| format!("创建托盘图标失败：{error}"))
}

fn load_tray_icon() -> Result<Icon, String> {
    let image = image::load_from_memory(include_bytes!("../../assets/icon.ico"))
        .map_err(|error| format!("读取托盘图标失败：{error}"))?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height)
        .map_err(|error| format!("转换托盘图标失败：{error}"))
}

enum TrayEventAction {
    ToggleMainWindow,
    OpenMenu(MenuPosition),
}

fn tray_event_action(event: TrayIconEvent) -> Option<TrayEventAction> {
    let TrayIconEvent::Click {
        position,
        button,
        button_state: MouseButtonState::Up,
        ..
    } = event
    else {
        return None;
    };

    if button == MouseButton::Left {
        let x = position.x as f32;
        let y = position.y as f32;
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        return Some(TrayEventAction::ToggleMainWindow);
    }
    if button != MouseButton::Right {
        return None;
    }

    let x = position.x as f32;
    let y = position.y as f32;
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some(TrayEventAction::OpenMenu(MenuPosition::tray(x, y, 1.0)))
}

fn open_menu(position: MenuPosition, cx: &mut App) {
    let translator = i18n::translator(cx);
    let version_item = version_menu_item(cx);
    let checking = cx
        .try_global::<crate::services::updates::UpdateService>()
        .is_some_and(|service| service.checking);
    let menu = MenuModel::new(vec![
        MenuItem::action(
            MenuActionId::new(TOGGLE_MAIN_WINDOW_ACTION),
            translator.translate("tray.menu.toggle"),
        )
        .with_icon(MenuIcon::svg("app-window.svg")),
        MenuItem::action(
            MenuActionId::new(TOGGLE_CLIPBOARD_MONITOR_ACTION),
            translator.translate(
                if crate::windows::main::MainWindowService::clipboard_listener_active(cx) {
                    "tray.menu.clipboard_monitor.enabled"
                } else {
                    "tray.menu.clipboard_monitor.disabled"
                },
            ),
        )
        .with_icon(MenuIcon::svg("clipboard.svg")),
        MenuItem::action(
            MenuActionId::new(TOGGLE_GLOBAL_HOTKEYS_ACTION),
            translator.translate(if crate::hotkey::global_hotkeys_enabled(cx) {
                "tray.menu.global_hotkeys.enabled"
            } else {
                "tray.menu.global_hotkeys.disabled"
            }),
        )
        .with_icon(MenuIcon::svg("keyboard.svg")),
        MenuItem::separator(),
        MenuItem::action(
            MenuActionId::new(SETTINGS_ACTION),
            translator.translate("tray.menu.settings"),
        )
        .with_icon(MenuIcon::svg("settings.svg")),
        MenuItem::separator(),
        MenuItem::action(
            MenuActionId::new(CHECK_UPDATES_ACTION),
            translator.translate(if checking {
                "updates.status.checking"
            } else {
                "updates.check"
            }),
        )
        .with_icon(MenuIcon::svg("refresh.svg")),
        MenuItem::action(
            MenuActionId::new(RESTART_ACTION),
            translator.translate("tray.menu.restart"),
        )
        .with_icon(MenuIcon::svg("refresh.svg")),
        MenuItem::action(
            MenuActionId::new(QUIT_ACTION),
            translator.translate("tray.menu.quit"),
        )
        .with_icon(MenuIcon::svg("power.svg")),
        MenuItem::separator(),
        version_item,
    ]);
    MenuService::open_context_menu(menu, position, handle_menu_action, cx);
}

fn version_menu_item(cx: &App) -> MenuItem {
    if let Some(release) = crate::services::updates::UpdateService::available(cx) {
        MenuItem::action(
            MenuActionId::new(OPEN_RELEASE_ACTION),
            i18n::translator(cx)
                .translate_with_args("updates.found", &[("version", release.version_label())]),
        )
        .centered()
        .success()
    } else {
        MenuItem::action(
            MenuActionId::new(OPEN_RELEASE_ACTION),
            format!("QcClip v{}", env!("CARGO_PKG_VERSION")),
        )
        .centered()
        .disabled()
    }
}

pub(crate) fn refresh_update_menu(menu: &mut MenuModel, cx: &App) -> bool {
    let Some(index) = menu.items().iter().position(|item| {
        item.id
            .as_ref()
            .is_some_and(|id| id.as_str() == OPEN_RELEASE_ACTION)
    }) else {
        return false;
    };
    if let Some(item) = menu.item_mut(index) {
        *item = version_menu_item(cx);
    }
    let checking = cx
        .global::<crate::services::updates::UpdateService>()
        .checking;
    let check_index = menu.items().iter().position(|item| {
        item.id
            .as_ref()
            .is_some_and(|id| id.as_str() == CHECK_UPDATES_ACTION)
    });
    if let Some(item) = check_index.and_then(|index| menu.item_mut(index)) {
        item.enabled = !checking;
        item.label = i18n::translator(cx)
            .translate(if checking {
                "updates.status.checking"
            } else {
                "updates.check"
            })
            .into();
    }
    true
}

fn handle_menu_action(action: MenuActionId, cx: &mut App) {
    match action.as_str() {
        TOGGLE_MAIN_WINDOW_ACTION => crate::windows::main::toggle_main_window(cx),
        TOGGLE_CLIPBOARD_MONITOR_ACTION => {
            crate::windows::main::MainWindowService::toggle_clipboard_listener(cx)
        }
        TOGGLE_GLOBAL_HOTKEYS_ACTION => {
            use crate::windows::notification::{NotificationLevel, NotificationService};
            let enabled = !crate::hotkey::global_hotkeys_enabled(cx);
            let result = crate::hotkey::toggle_global_hotkeys(cx);
            let translator = i18n::translator(cx);
            let (title_key, detail, level) = match result {
                Ok(()) if enabled => (
                    "notification.global_hotkeys.enabled.title",
                    translator.translate("notification.global_hotkeys.enabled.detail"),
                    NotificationLevel::Success,
                ),
                Ok(()) => (
                    "notification.global_hotkeys.disabled.title",
                    translator.translate("notification.global_hotkeys.disabled.detail"),
                    NotificationLevel::Info,
                ),
                Err(error) => {
                    tracing::error!("切换全局快捷键失败：{error}");
                    (
                        "notification.global_hotkeys.failed.title",
                        error,
                        NotificationLevel::Error,
                    )
                }
            };
            NotificationService::show_message_for(
                crate::config::NotificationKind::FeatureStatus,
                translator.translate(title_key),
                detail,
                level,
                cx,
            );
        }
        SETTINGS_ACTION => crate::windows::settings::open_settings_window(cx),
        CHECK_UPDATES_ACTION => crate::services::updates::UpdateService::check(true, cx),
        OPEN_RELEASE_ACTION => crate::services::updates::UpdateService::open_release(cx),
        RESTART_ACTION => cx.restart(),
        QUIT_ACTION => cx.quit(),
        _ => {}
    }
}
