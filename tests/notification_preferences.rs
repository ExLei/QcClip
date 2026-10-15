#![allow(dead_code, unused_imports)]

#[path = "../src/config/mod.rs"]
mod config;

use config::{AppConfig, NotificationKind};

#[test]
fn notification_defaults_and_released_update_preference_are_preserved() {
    let loaded = config::load::from_str(
        r#"{"schemaVersion":1,"general":{"globalNotificationEnabled":false},"updates":{"notificationEnabled":false}}"#,
    ).expect("无法加载已发布版本的通知配置");
    assert!(!loaded.config.general.global_notification_enabled);
    assert!(loaded.config.notification_preference(NotificationKind::Startup));
    assert!(!loaded.config.notification_preference(NotificationKind::VersionUpdate));
    for kind in [NotificationKind::ClipboardPreview, NotificationKind::FeatureStatus,
        NotificationKind::OperationResult, NotificationKind::Attention] {
        assert!(loaded.config.notification_preference(kind));
    }
    assert!(loaded.warnings.is_empty());
}

#[test]
fn explicitly_disabled_startup_notification_is_preserved() {
    let loaded = config::load::from_str(
        r#"{"general":{"notifications":{"startup":false}}}"#,
    ).expect("无法加载已关闭启动通知的配置");
    assert!(!loaded.config.notification_preference(NotificationKind::Startup));
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
                assert_eq!(config.notification_preference(other), AppConfig::default().notification_preference(other));
            }
        }
        if kind == NotificationKind::VersionUpdate {
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(value["updates"]["notificationEnabled"], false);
            assert!(value["general"]["notifications"].get("versionUpdate").is_none());
        }
    }
}

#[test]
fn invalid_notification_field_does_not_reset_valid_siblings() {
    let loaded = config::load::from_str(
        r#"{"general":{"notifications":{"startup":true,"clipboardPreview":false,"featureStatus":"invalid","operationResult":false,"attention":false}},"updates":{"notificationEnabled":false}}"#,
    ).expect("无法加载字段级容错通知配置");
    assert!(loaded.config.notification_preference(NotificationKind::Startup));
    assert!(!loaded.config.notification_preference(NotificationKind::ClipboardPreview));
    assert!(loaded.config.notification_preference(NotificationKind::FeatureStatus));
    assert!(!loaded.config.notification_preference(NotificationKind::OperationResult));
    assert!(!loaded.config.notification_preference(NotificationKind::Attention));
    assert!(!loaded.config.notification_preference(NotificationKind::VersionUpdate));
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
