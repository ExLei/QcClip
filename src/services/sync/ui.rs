use super::{SyncService, protocol::Mode};
use crate::{
    i18n,
    windows::{
        main::MainWindowService,
        menu::{MenuActionId, MenuIcon, MenuItem},
        notification::{NotificationLevel, NotificationService},
    },
};
use gpui::{App, PromptButton, PromptLevel};

pub fn menu(cx: &App) -> MenuItem {
    let t = i18n::translator(cx);
    let snapshot = SyncService::snapshot(cx);
    let mut groups = Vec::new();
    if let Some(config) = snapshot.config.filter(|c| c.enabled) {
        for (kind, label) in [
            ("content", "sync.send_content"),
            ("upload", "sync.upload_content"),
        ] {
            let mut targets = Vec::new();
            let available = |peer: &super::config::Peer| {
                if kind == "upload" {
                    config.dav_allowed(peer)
                } else {
                    snapshot
                        .statuses
                        .get(&peer.profile.id)
                        .cloned()
                        .unwrap_or_default()
                        .channel(&config, peer)
                        .is_some()
                }
            };
            let online = config.peers.iter().any(available);
            let all = MenuItem::action(
                MenuActionId::new(format!("sync:{kind}:all")),
                t.translate("sync.all_online"),
            );
            targets.push(if online { all } else { all.disabled() });
            for peer in &config.peers {
                let channel = if kind == "upload" {
                    config.dav_allowed(peer).then_some("WebDAV")
                } else {
                    snapshot
                        .statuses
                        .get(&peer.profile.id)
                        .cloned()
                        .unwrap_or_default()
                        .channel(&config, peer)
                };
                let label = format!(
                    "{} · {}",
                    peer.profile.label(),
                    channel.map(str::to_owned).unwrap_or_else(|| t.translate(
                        if peer.policy.enabled {
                            "sync.offline"
                        } else {
                            "sync.blocked"
                        }
                    ))
                );
                let item = MenuItem::action(
                    MenuActionId::new(format!("sync:{kind}:{}", peer.profile.id)),
                    label,
                );
                targets.push(if available(peer) {
                    item
                } else {
                    item.disabled()
                });
            }
            if config.peers.is_empty() {
                targets.push(MenuItem::action(
                    MenuActionId::new("sync:settings"),
                    t.translate("sync.no_devices_help"),
                ));
            }
            groups.push(MenuItem::submenu(t.translate(label), targets));
        }
    }
    if groups.is_empty() {
        groups.push(MenuItem::action(
            MenuActionId::new("sync:settings"),
            t.translate("sync.open_settings"),
        ));
    }
    MenuItem::submenu(t.translate("settings.page.sync"), groups)
        .with_icon(MenuIcon::svg("devices.svg"))
}

pub fn perform(action: &str, item_id: i64, favorite: bool, cx: &mut App) -> bool {
    let Some(action) = action.strip_prefix("sync:") else {
        return false;
    };
    if action == "settings" {
        crate::windows::settings::open_sync_window(cx);
        return true;
    }
    let Some((kind, target)) = action.split_once(':') else {
        return true;
    };
    let mode = match kind {
        "content" | "upload" if favorite => Mode::ManualFavorite,
        "content" | "upload" => Mode::ManualContent,
        _ => return true,
    };
    let upload_only = kind == "upload";
    let target = if target == "all" {
        None
    } else {
        Some(target.to_owned())
    };
    let t = i18n::translator(cx);
    let Some(window) = MainWindowService::window_handle(cx) else {
        return true;
    };
    let title = if upload_only {
        t.translate("sync.upload_content")
    } else {
        t.translate("sync.send_content")
    };
    let detail = t.translate(if favorite {
        "sync.favorite_confirm"
    } else {
        "sync.content_confirm"
    });
    let answer = window.update(cx, |_, window, cx| {
        window.prompt(
            PromptLevel::Warning,
            &title,
            Some(&detail),
            &[
                PromptButton::cancel(t.translate("sync.cancel")),
                PromptButton::ok(t.translate("sync.send")),
            ],
            cx,
        )
    });
    if let Ok(answer) = answer {
        cx.spawn(async move |cx| {
            if answer.await.unwrap_or(0) != 1 {
                return;
            }
            cx.update(|cx| {
                let result = SyncService::get(cx).and_then(|r| {
                    if upload_only {
                        r.upload(target.as_deref(), item_id, mode)
                    } else {
                        r.manual(target.as_deref(), item_id, mode)
                    }
                });
                let (message, level) = match result {
                    Ok(count) => (
                        i18n::translator(cx).translate_with_args(
                            "sync.sending_count",
                            &[("count", count.to_string())],
                        ),
                        NotificationLevel::Info,
                    ),
                    Err(error) => (error.to_string(), NotificationLevel::Error),
                };
                NotificationService::show_message(title, message, level, cx);
            });
        })
        .detach();
    }
    true
}
