use serde::{Deserialize, Serialize};

use super::AppConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotificationConfig {
    pub startup: bool,
    pub clipboard_preview: bool,
    pub feature_status: bool,
    pub operation_result: bool,
    pub attention: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            startup: true,
            clipboard_preview: true,
            feature_status: true,
            operation_result: true,
            attention: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    Startup,
    ClipboardPreview,
    FeatureStatus,
    OperationResult,
    Attention,
    VersionUpdate,
}

impl NotificationKind {
    pub const ALL: [Self; 6] = [
        Self::Startup, Self::ClipboardPreview, Self::FeatureStatus,
        Self::OperationResult, Self::Attention, Self::VersionUpdate,
    ];

    pub const fn label_key(self) -> &'static str {
        match self {
            Self::Startup => "settings.notifications.startup",
            Self::ClipboardPreview => "settings.notifications.clipboard_preview",
            Self::FeatureStatus => "settings.notifications.feature_status",
            Self::OperationResult => "settings.notifications.operation_result",
            Self::Attention => "settings.notifications.attention",
            Self::VersionUpdate => "updates.notification",
        }
    }
}

impl AppConfig {
    pub fn notification_preference(&self, kind: NotificationKind) -> bool {
        let notifications = &self.general.notifications;
        match kind {
            NotificationKind::Startup => notifications.startup,
            NotificationKind::ClipboardPreview => notifications.clipboard_preview,
            NotificationKind::FeatureStatus => notifications.feature_status,
            NotificationKind::OperationResult => notifications.operation_result,
            NotificationKind::Attention => notifications.attention,
            NotificationKind::VersionUpdate => self.updates.notification_enabled,
        }
    }

    pub fn set_notification_preference(&mut self, kind: NotificationKind, enabled: bool) {
        let notifications = &mut self.general.notifications;
        match kind {
            NotificationKind::Startup => notifications.startup = enabled,
            NotificationKind::ClipboardPreview => notifications.clipboard_preview = enabled,
            NotificationKind::FeatureStatus => notifications.feature_status = enabled,
            NotificationKind::OperationResult => notifications.operation_result = enabled,
            NotificationKind::Attention => notifications.attention = enabled,
            NotificationKind::VersionUpdate => self.updates.notification_enabled = enabled,
        }
    }

    pub fn allows_notification(&self, kind: NotificationKind) -> bool {
        self.general.global_notification_enabled && self.notification_preference(kind)
    }
}
