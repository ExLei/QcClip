pub(crate) mod components;
mod file_list;
pub(crate) mod screen_boundary;
mod state;
pub(crate) mod tabs;
mod view;
mod window_drag;
mod window_state;
mod window_visibility;

const SIDEBAR_LAYOUT_MIN_WIDTH: f32 = 500.0;

#[cfg(target_os = "windows")]
use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, AppContext as _, BorrowAppContext as _, Entity, Global, Pixels, Size,
    Window,
};

use crate::{
    config::SessionShortcutId,
    config_runtime,
    platform::{input, window as platform_window},
};

pub use state::MainWindow;
pub(crate) use window_visibility::sync_mouse_leave_state;

pub(crate) fn uses_sidebar_layout(logical_width: f32) -> bool {
    logical_width >= SIDEBAR_LAYOUT_MIN_WIDTH
}

#[derive(Default)]
pub(crate) struct MainWindowService {
    window: Option<AnyWindowHandle>,
    main_view: Option<Entity<MainWindow>>,
    mouse_event_listener_id: Option<u64>,
    window_state: window_state::State,
    pub(crate) memory_trim_state: crate::services::memory_trim::State,
}

impl Global for MainWindowService {}

impl MainWindowService {
    pub(crate) fn install(cx: &mut App) {
        cx.set_global(Self::default());
        cx.on_window_closed(|cx, window_id| {
            let closes_main_window = cx.read_global::<Self, _>(|service, _| {
                service
                    .window
                    .is_some_and(|window| window.window_id() == window_id)
            });
            if closes_main_window {
                cx.update_global::<Self, _>(|service, _| {
                    service.window = None;
                    service.main_view = None;
                    service.window_state.reset();
                    service.memory_trim_state = Default::default();
                    if let Some(subscription_id) = service.mouse_event_listener_id.take() {
                        input::unsubscribe_mouse_events(subscription_id);
                    }
                });
                crate::hotkey::sync_session_shortcuts(false, false, cx);
            }
        })
        .detach();

        let (subscription_id, receiver) = input::subscribe_mouse_events();
        cx.update_global::<Self, _>(|service, _| {
            service.mouse_event_listener_id = Some(subscription_id);
        });
        cx.spawn(async move |cx| {
            while let Ok(event) = receiver.recv().await {
                cx.update(|cx| match event {
                    input::GlobalMouseEvent::MouseDown {
                        sequence,
                        position,
                        foreground_window,
                    } => {
                        if !crate::windows::menu::MenuService::handle_global_mouse_down(
                            sequence, position, cx,
                        ) {
                            window_visibility::handle_global_mouse_down(
                                position,
                                foreground_window,
                                cx,
                            );
                        }
                    }
                    input::GlobalMouseEvent::MouseUp {
                        position,
                        left_pressed,
                    } if !left_pressed => window_drag::handle_mouse_up(position, cx),
                    input::GlobalMouseEvent::MouseUp { .. } => {}
                });
            }
            tracing::debug!("全局鼠标事件监听任务已退出");
        })
        .detach();
    }

    pub(crate) fn startup_edge_bounds(
        cx: &App,
    ) -> Option<(gpui::Bounds<gpui::Pixels>, gpui::DisplayId)> {
        window_drag::startup_edge_bounds(cx)
    }

    pub(crate) fn set_main_view(main_view: Entity<MainWindow>, cx: &mut App) {
        cx.update_global::<Self, _>(|service, _| service.main_view = Some(main_view));
    }

    pub(crate) fn handle_window_shown(window: &mut Window, cx: &mut App) {
        let main_view = cx
            .try_global::<Self>()
            .and_then(|service| service.main_view.clone());
        if let Some(main_view) = main_view {
            let _ = main_view.update(cx, |view, cx| view.handle_window_shown(window, cx));
        }
    }

    pub(crate) fn clipboard_write_suppressor(
        cx: &App,
    ) -> Option<crate::platform::clipboard::ClipboardWriteSuppressor> {
        cx.read_global::<Self, _>(|service, cx| {
            service
                .main_view
                .as_ref()
                .map(|main_view| main_view.read(cx).clipboard_write_suppressor.clone())
        })
    }

    pub(crate) fn refresh_history(cx: &mut App) {
        let main_view = cx
            .try_global::<Self>()
            .and_then(|service| service.main_view.clone());
        if let Some(main_view) = main_view {
            main_view.update(cx, |view, cx| view.refresh_history(cx));
        }
    }

    pub(crate) fn reload_after_data_import(cx: &mut App) {
        window_visibility::apply_imported_window_state(cx);
        let main_view = cx
            .try_global::<Self>()
            .and_then(|service| service.main_view.clone());
        if let Some(main_view) = main_view {
            main_view.update(cx, |view, cx| view.reload_after_data_import(cx));
        }
    }

    pub(crate) fn replace_data_sources(
        database: crate::services::database::Database,
        blob_store: crate::services::storage::RawBlobStore,
        data_root: &std::path::Path,
        cx: &mut App,
    ) {
        window_visibility::apply_imported_window_state(cx);
        let main_view = cx
            .try_global::<Self>()
            .and_then(|service| service.main_view.clone());
        if let Some(main_view) = main_view {
            main_view.update(cx, |view, cx| {
                view.replace_data_sources(database, blob_store, data_root, cx);
            });
        }
    }

    pub(crate) fn stop_clipboard_listener(cx: &mut App) {
        let main_view = cx
            .try_global::<Self>()
            .and_then(|service| service.main_view.clone());
        if let Some(main_view) = main_view {
            main_view.update(cx, |view, _| view.stop_clipboard_listener());
        }
    }

    pub(crate) fn restart_clipboard_listener(cx: &mut App) {
        let main_view = cx
            .try_global::<Self>()
            .and_then(|service| service.main_view.clone());
        if let Some(main_view) = main_view {
            main_view.update(cx, |view, cx| view.restart_clipboard_listener(cx));
        }
    }

    pub(crate) fn toggle_clipboard_listener(cx: &mut App) {
        let main_view = cx
            .try_global::<Self>()
            .and_then(|service| service.main_view.clone());
        let Some(main_view) = main_view else {
            tracing::error!("主窗口尚未就绪，无法切换剪贴板监听");
            return;
        };
        let result = main_view.update(cx, |view, cx| {
            view.toggle_clipboard_listener(cx)
        });
        let (title_key, detail_key, level) = match result {
            state::ClipboardListenerToggleResult::Enabled => (
                "notification.clipboard_monitor.enabled.title",
                "notification.clipboard_monitor.enabled.detail",
                crate::windows::notification::NotificationLevel::Success,
            ),
            state::ClipboardListenerToggleResult::Disabled => (
                "notification.clipboard_monitor.disabled.title",
                "notification.clipboard_monitor.disabled.detail",
                crate::windows::notification::NotificationLevel::Info,
            ),
            state::ClipboardListenerToggleResult::Failed => (
                "notification.clipboard_monitor.failed.title",
                "notification.clipboard_monitor.failed.detail",
                crate::windows::notification::NotificationLevel::Error,
            ),
        };
        let translator = crate::i18n::translator(cx);
        crate::windows::notification::NotificationService::show_message_for(
            crate::config::NotificationKind::FeatureStatus,
            translator.translate(title_key),
            translator.translate(detail_key),
            level,
            cx,
        );
        main_view.update(cx, |_, cx| cx.notify());
    }

    pub(crate) fn clipboard_listener_active(cx: &App) -> bool {
        cx.try_global::<Self>()
            .and_then(|service| service.main_view.clone())
            .map(|main_view| main_view.read(cx)._clipboard_monitor.is_some())
            .unwrap_or(true)
    }

    pub(crate) fn paste_plain_text_shortcut(cx: &mut App) {
        let visible = Self::is_logically_visible(cx);
        let main_view = cx
            .try_global::<Self>()
            .and_then(|service| service.main_view.clone());
        let Some(main_view) = main_view else {
            Self::show_plain_text_paste_unavailable(cx);
            return;
        };
        if visible {
            let started = main_view.update(cx, |view, cx| {
                view.paste_selected_item_as_plain_text(cx)
            });
            if !started {
                Self::show_plain_text_paste_unavailable(cx);
            }
        } else {
            main_view.update(cx, |view, cx| view.paste_latest_item_as_plain_text(cx));
        }
    }

    pub(crate) fn paste_history_index(index: usize, cx: &mut App) {
        let main_view = cx
            .try_global::<Self>()
            .and_then(|service| service.main_view.clone());
        if let Some(main_view) = main_view {
            main_view.update(cx, |view, cx| view.paste_history_index(index, cx));
        }
    }

    pub(crate) fn show_plain_text_paste_unavailable(cx: &mut App) {
        let translator = crate::i18n::translator(cx);
        crate::windows::notification::NotificationService::show_message(
            translator.translate("notification.plain_text_paste.unavailable.title"),
            translator.translate("notification.plain_text_paste.unavailable.detail"),
            crate::windows::notification::NotificationLevel::Warning,
            cx,
        );
    }

    pub(crate) fn window_handle(cx: &App) -> Option<AnyWindowHandle> {
        cx.try_global::<Self>()
            .and_then(|service| service.window)
    }

    pub(crate) fn set_window(
        window: AnyWindowHandle,
        initially_visible: bool,
        initial_logical_size: Size<Pixels>,
        cx: &mut App,
    ) {
        cx.update_global::<Self, _>(|service, _| {
            service.window = Some(window);
            service.window_state.set_visibility(initially_visible);
        });
        if config_runtime::clipboard_config(cx).edge_hide {
            window_visibility::initialize_edge_hide_enabled(cx);
        }
        cx.defer(move |cx| {
            let result = window.update(cx, |_, window, cx| {
                let position_result = window_visibility::position_initial_window(
                    window,
                    initial_logical_size,
                    cx,
                );
                let hide_result =
                    window_visibility::hide_initial_window(window, initially_visible, cx);
                let result = position_result.and(hide_result);
                if initially_visible && result.is_ok() {
                    Self::handle_window_shown(window, cx);
                }
                result
            });
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::error!("定位主窗口失败：{error}"),
                Err(error) => tracing::error!("定位主窗口失败：{error}"),
            }
        });
        crate::services::memory_trim::schedule_startup_cleanup(cx);

        #[cfg(target_os = "windows")]
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let Some(window_is_open) = cx
                    .try_read_global::<MainWindowService, _>(|service, _| service.window.is_some())
                else {
                    break;
                };
                if !window_is_open {
                    break;
                }
                let Some(position) = input::pointer_snapshot().map(|snapshot| snapshot.position)
                else {
                    continue;
                };
                cx.update(|cx| {
                    let (visible, pointer_inside) = Self::session_shortcut_scope(cx);
                    crate::hotkey::sync_session_shortcuts(visible, pointer_inside, cx);
                });
                // 每帧都按当前指针位置判定贴边显示/隐藏，鼠标停在触发区不动时也能恢复显示。
                let drag_request = cx.update(|cx| {
                    window_drag::sync_edge_bounds_after_native_move(cx);
                    let drag_request =
                        crate::platform::external_drag::ExternalDragService::prepare_poll(cx);
                    let position = input::pointer_snapshot()
                        .map(|snapshot| snapshot.position)
                        .unwrap_or(position);
                    window_visibility::handle_mouse_move(position, cx);
                    drag_request
                });
                if let Some(request) = drag_request {
                    // DoDragDrop 会同步泵送 Windows 消息，因此不能持有 GPUI App 借用。
                    let completion =
                        crate::platform::external_drag::ExternalDragService::execute(request);
                    cx.update(|cx| {
                        crate::platform::external_drag::ExternalDragService::finish(
                            completion,
                            cx,
                        );
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn cancel_hover_preview(cx: &mut App) {
        let Some(main_view) = cx
            .try_global::<Self>()
            .and_then(|service| service.main_view.clone())
        else {
            return;
        };
        main_view.update(cx, |main_window, cx| {
            main_window.cancel_hover_preview(cx);
        });
    }

    pub(crate) fn notify_main_view(cx: &mut App) {
        let Some(main_view) = cx
            .try_global::<Self>()
            .and_then(|service| service.main_view.clone())
        else {
            return;
        };
        let _ = main_view.update(cx, |_, cx| cx.notify());
    }

    pub(crate) fn open_new_text_editor(cx: &mut App) {
        let Some(main_view) = cx
            .try_global::<Self>()
            .and_then(|service| service.main_view.clone())
        else {
            tracing::error!("主窗口尚未就绪，无法打开新增文本窗口");
            return;
        };
        main_view.update(cx, |main_window, cx| {
            main_window.new_text_item(cx);
        });
    }

    pub(crate) fn is_logically_visible(cx: &App) -> bool {
        cx.global::<Self>().window_state.is_logically_visible()
    }

    pub(crate) fn session_shortcut_scope(cx: &mut App) -> (bool, bool) {
        let (window, visible) = cx.read_global::<Self, _>(|service, _| {
            (
                service.window,
                service.window_state.is_logically_visible(),
            )
        });
        let Some(window) = window.filter(|_| visible) else {
            return (visible, false);
        };
        let pointer = input::pointer_snapshot().map(|snapshot| snapshot.position);
        let inside = pointer.is_some_and(|position| {
            window
                .update(cx, |_, window, _| {
                    platform_window::physical_client_bounds(window)
                        .is_ok_and(|bounds| window_visibility::point_in_client_bounds(position, bounds))
                })
                .unwrap_or(false)
        });
        (visible, inside)
    }

    pub(crate) fn set_window_fixed(fixed: bool, cx: &mut App) {
        cx.update_global::<Self, _>(|service, _| service.window_state.set_fixed(fixed));
    }

    pub(crate) fn set_edge_hide_enabled(enabled: bool, cx: &mut App) {
        // 当前窗口事件结束前，GPUI 会暂时移出当前窗口；等待窗口归还后再按句柄更新。
        cx.defer(move |cx| window_visibility::set_edge_hide_enabled(enabled, cx));
    }

    pub(crate) fn refresh_edge_hide_position(cx: &mut App) {
        window_visibility::refresh_edge_hide_position(cx);
    }

    pub(crate) fn set_edge_hide_temporarily_blocked(blocked: bool, cx: &mut App) {
        window_visibility::set_edge_hide_temporarily_blocked(blocked, cx);
    }
}

pub fn show_main_window(cx: &mut App) {
    if let Err(error) =
        window_visibility::set_visibility(true, window_visibility::VisibilitySource::Implicit, cx)
    {
        tracing::error!("显示主窗口失败：{error}");
    }
}

/// 由交互顺带触发的隐藏（例如粘贴后隐藏），固定窗口状态下不会隐藏。
pub fn hide_main_window(cx: &mut App) {
    if let Err(error) =
        window_visibility::set_visibility(false, window_visibility::VisibilitySource::Implicit, cx)
    {
        tracing::error!("隐藏主窗口失败：{error}");
    }
}

pub fn toggle_main_window(cx: &mut App) {
    if let Err(error) = window_visibility::toggle_visibility(cx) {
        tracing::error!("切换主窗口失败：{error}");
    }
}

pub fn hide_main_window_explicitly(cx: &mut App) {
    if let Err(error) = window_visibility::set_visibility(
        false,
        window_visibility::VisibilitySource::ExplicitToggle,
        cx,
    ) {
        tracing::error!("主动隐藏主窗口失败：{error}");
    }
}

pub(crate) fn dispatch_session_shortcut(id: SessionShortcutId, cx: &mut App) {
    if id == SessionShortcutId::HideWindow {
        hide_main_window_explicitly(cx);
        return;
    }
    let Some((window, main_view)) = cx
        .try_global::<MainWindowService>()
        .and_then(|service| Some((service.window?, service.main_view.clone()?)))
    else {
        return;
    };
    let favorite_removal = main_view.read_with(cx, |main, _| {
        main.favorite_removal_target_for_shortcut(id)
    });
    if let Some((favorite_id, history_id)) = favorite_removal {
        crate::windows::main::components::clipboard_list::confirm_remove_from_favorites(
            main_view,
            window,
            favorite_id,
            history_id,
            cx,
        );
        return;
    }
    if let Err(error) = window.update(cx, |_, window, cx| {
        main_view.update(cx, |main, cx| match id {
            SessionShortcutId::SelectPreviousItem => main.move_selection(-1, window, cx),
            SessionShortcutId::SelectNextItem => main.move_selection(1, window, cx),
            SessionShortcutId::SelectFirstItem => main.select_list_boundary(false, window, cx),
            SessionShortcutId::SelectLastItem => main.select_list_boundary(true, window, cx),
            SessionShortcutId::PreviousTab => main.move_tab(-1, cx),
            SessionShortcutId::NextTab => main.move_tab(1, cx),
            SessionShortcutId::ToggleWindowFixed => {
                main.titlebar_window_fixed = !main.titlebar_window_fixed;
                MainWindowService::set_window_fixed(main.titlebar_window_fixed, cx);
                cx.notify();
            }
            SessionShortcutId::FocusSearch => {
                main.toggle_search_input(window, cx);
            }
            SessionShortcutId::ExecutePaste => main.paste_selected_item(cx),
            SessionShortcutId::ToggleFavorite => main.toggle_selected_favorite(cx),
            SessionShortcutId::TogglePinned => main.toggle_selected_pinned(cx),
            SessionShortcutId::DeleteItem => main.delete_selected_item(cx),
            SessionShortcutId::HideWindow => {}
        });
    }) {
        tracing::debug!(%error, "执行窗口快捷键失败");
    }
}
