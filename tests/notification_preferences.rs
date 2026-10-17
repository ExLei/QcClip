#![allow(dead_code, unused_imports)]

#[path = "../src/config/mod.rs"]
mod config;

use config::{AppConfig, NotificationConfig, NotificationKind, NotificationPosition};

#[test]
fn released_notification_config_keeps_presentation_defaults() {
    let loaded = config::load::from_str(
        r#"{"general":{"notifications":{"startup":false,"attention":false}}}"#,
    )
    .expect("无法读取已发布的通知配置");
    assert_eq!(
        loaded.config.general.notifications.position,
        NotificationPosition::BottomRight
    );
    assert_eq!(loaded.config.general.notifications.duration_ms, 4200);
    assert!(!loaded.config.general.notifications.startup);
    assert!(!loaded.config.general.notifications.attention);
    assert!(loaded.warnings.is_empty());
}

#[test]
fn all_eight_positions_and_custom_duration_round_trip() {
    assert_eq!(NotificationPosition::ALL.len(), 8);
    for position in NotificationPosition::ALL {
        let mut config = AppConfig::default();
        config.general.notifications.position = position;
        config.general.notifications.duration_ms = 7300;
        config.general.notifications.clipboard_preview = false;
        let json = serde_json::to_string(&config).expect("无法保存通知显示配置");
        let loaded = config::load::from_str(&json).expect("无法读取通知显示配置");
        assert_eq!(loaded.config, config);
        assert!(loaded.warnings.is_empty());
    }
}

#[test]
fn invalid_presentation_fields_preserve_other_preferences() {
    let loaded = config::load::from_str(
        r#"{"general":{"notifications":{"position":"unknown","durationMs":"invalid","clipboardPreview":false}}}"#,
    )
    .expect("无法读取有无效显示字段的通知配置");
    assert_eq!(
        loaded.config.general.notifications.position,
        NotificationPosition::BottomRight
    );
    assert_eq!(loaded.config.general.notifications.duration_ms, 4200);
    assert!(!loaded.config.general.notifications.clipboard_preview);
    assert_eq!(loaded.warnings.len(), 2);
    assert!(
        loaded
            .warnings
            .iter()
            .any(|warning| warning.contains("general.notifications.position"))
    );
    assert!(
        loaded
            .warnings
            .iter()
            .any(|warning| warning.contains("general.notifications.durationMs"))
    );
}

#[test]
fn notification_duration_is_clamped_when_loading_or_normalizing() {
    for (input, expected) in [
        (0, 1000),
        (999, 1000),
        (4200, 4200),
        (15000, 15000),
        (u32::MAX, 15000),
    ] {
        let loaded = config::load::from_str(&format!(
            r#"{{"general":{{"notifications":{{"durationMs":{input}}}}}}}"#,
        ))
        .expect("无法读取通知显示时长");
        assert_eq!(loaded.config.general.notifications.duration_ms, expected);
        let mut config = AppConfig::default();
        config.general.notifications.duration_ms = input;
        assert_eq!(
            config.normalized().general.notifications.duration_ms,
            expected
        );
    }
    assert_eq!(NotificationConfig::default().duration_ms, 4200);
}

#[test]
fn notification_defaults_and_released_update_preference_are_preserved() {
    let loaded = config::load::from_str(
        r#"{"schemaVersion":1,"general":{"globalNotificationEnabled":false},"updates":{"notificationEnabled":false}}"#,
    ).expect("无法加载已发布版本的通知配置");
    assert!(!loaded.config.general.global_notification_enabled);
    assert!(
        loaded
            .config
            .notification_preference(NotificationKind::Startup)
    );
    assert!(
        !loaded
            .config
            .notification_preference(NotificationKind::VersionUpdate)
    );
    for kind in [
        NotificationKind::ClipboardPreview,
        NotificationKind::FeatureStatus,
        NotificationKind::OperationResult,
        NotificationKind::Attention,
    ] {
        assert!(loaded.config.notification_preference(kind));
    }
    assert!(loaded.warnings.is_empty());
}

#[test]
fn explicitly_disabled_startup_notification_is_preserved() {
    let loaded = config::load::from_str(r#"{"general":{"notifications":{"startup":false}}}"#)
        .expect("无法加载已关闭启动通知的配置");
    assert!(
        !loaded
            .config
            .notification_preference(NotificationKind::Startup)
    );
    assert!(loaded.warnings.is_empty());
}

#[test]
fn each_notification_preference_round_trips_independently() {
    for kind in NotificationKind::ALL {
        let mut config = AppConfig::default();
        config.set_notification_preference(kind, !config.notification_preference(kind));
        let json = serde_json::to_string(&config).expect("无法序列化通知配置");
        let loaded = config::load::from_str(&json).expect("无法重新加载通知配置");
        assert_eq!(loaded.config, config);
        assert!(loaded.warnings.is_empty());
        for other in NotificationKind::ALL {
            if other != kind {
                assert_eq!(
                    config.notification_preference(other),
                    AppConfig::default().notification_preference(other)
                );
            }
        }
        if kind == NotificationKind::VersionUpdate {
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(value["updates"]["notificationEnabled"], false);
            assert!(
                value["general"]["notifications"]
                    .get("versionUpdate")
                    .is_none()
            );
        }
    }
}

#[test]
fn invalid_notification_field_does_not_reset_valid_siblings() {
    let loaded = config::load::from_str(
        r#"{"general":{"notifications":{"startup":true,"clipboardPreview":false,"featureStatus":"invalid","operationResult":false,"attention":false}},"updates":{"notificationEnabled":false}}"#,
    ).expect("无法加载字段级容错通知配置");
    assert!(
        loaded
            .config
            .notification_preference(NotificationKind::Startup)
    );
    assert!(
        !loaded
            .config
            .notification_preference(NotificationKind::ClipboardPreview)
    );
    assert!(
        loaded
            .config
            .notification_preference(NotificationKind::FeatureStatus)
    );
    assert!(
        !loaded
            .config
            .notification_preference(NotificationKind::OperationResult)
    );
    assert!(
        !loaded
            .config
            .notification_preference(NotificationKind::Attention)
    );
    assert!(
        !loaded
            .config
            .notification_preference(NotificationKind::VersionUpdate)
    );
    assert_eq!(loaded.warnings.len(), 1);
    assert!(loaded.warnings[0].contains("general.notifications.featureStatus"));
}

#[test]
fn master_switch_suppresses_all_notifications_without_changing_preferences() {
    let mut config = AppConfig::default();
    config.set_notification_preference(NotificationKind::Startup, true);
    config.set_notification_preference(NotificationKind::ClipboardPreview, false);
    let preferences = config.general.notifications.clone();
    config.general.global_notification_enabled = false;
    for kind in NotificationKind::ALL {
        assert!(!config.allows_notification(kind));
    }
    config.general.global_notification_enabled = true;
    assert!(config.allows_notification(NotificationKind::Startup));
    assert!(!config.allows_notification(NotificationKind::ClipboardPreview));
    assert!(config.allows_notification(NotificationKind::Attention));
    assert_eq!(config.general.notifications, preferences);
}
