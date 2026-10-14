mod client;

use std::{fs, io::Write, path::PathBuf, time::{Duration, SystemTime, UNIX_EPOCH}};

use atomic_write_file::AtomicWriteFile;
use gpui::{App, AppContext, BorrowAppContext, Global};
use serde::{Deserialize, Serialize};

use crate::{config_runtime, i18n};
use crate::windows::notification::{NotificationLevel, NotificationRequest, NotificationService};
pub(crate) use client::{Release, valid_proxy_url};
use client::{fetch_latest, newer_than_current};
#[cfg(debug_assertions)]
use client::{RELEASES_API, api_url};

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct CheckRecord {
    checked_at: u64,
    latest: Option<Release>,
    notified_tag: Option<String>,
}

pub(crate) struct UpdateService {
    pub(crate) checking: bool,
    pub(crate) status_key: &'static str,
    pub(crate) error: Option<String>,
    record: CheckRecord,
    path: PathBuf,
    retry_after: u64,
}

impl Global for UpdateService {}

impl UpdateService {
    pub(crate) fn install(path: PathBuf, cx: &mut App) {
        #[cfg(debug_assertions)]
        let path = if api_url() != RELEASES_API {
            path.with_file_name("config.test-updates.json")
        } else { path };
        let record = fs::read(&path).ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        let status_key = check_status(&record);
        cx.set_global(Self {
            checking: false, status_key, error: None,
            record, path, retry_after: 0,
        });
        cx.refresh_windows();
        cx.spawn(async move |cx| {
            loop {
                cx.update(|cx| {
                    let service = cx.global::<Self>();
                    if service.checking { return; }
                    // “永不更新”只停止自动检查
                    let frequency = config_runtime::update_config(cx).frequency;
                    let Some(interval) = frequency.seconds() else { return; };
                    let now = unix_time();
                    if now >= service.retry_after
                        && (service.record.checked_at > now || service.record.checked_at == 0
                            || now.saturating_sub(service.record.checked_at) >= interval)
                    {
                        Self::check(false, cx);
                    }
                });
                cx.background_executor().timer(Duration::from_secs(60)).await;
            }
        }).detach();
    }

    pub(crate) fn available(cx: &App) -> Option<Release> {
        cx.try_global::<Self>()?.record.latest.clone().filter(|release| {
            newer_than_current(release)
        })
    }

    pub(crate) fn open_release(cx: &mut App) {
        if let Some(release) = Self::available(cx) {
            cx.open_url(&release.url());
        }
    }

    pub(crate) fn check(manual: bool, cx: &mut App) {
        if cx.global::<Self>().checking {
            return;
        }
        cx.update_global::<Self, _>(|service, _| {
            service.checking = true;
            service.error = None;
            service.status_key = "updates.status.checking";
        });
        cx.refresh_windows();
        let config = config_runtime::update_config(cx);
        let task = cx.background_spawn(async move { fetch_latest(&config) });
        cx.spawn(async move |cx| {
            let result = task.await;
            cx.update(|cx| Self::finish(result, manual, cx));
        }).detach();
    }

    fn finish(result: Result<Option<Release>, String>, manual: bool, cx: &mut App) {
        let config = config_runtime::update_config(cx);
        let notifications = (manual || config.notification_enabled) && config_runtime::global_notification_enabled(cx);
        let mut notify_release = None;
        cx.update_global::<Self, _>(|service, _| {
            service.checking = false;
            match result {
                Ok(latest) => {
                    service.record.checked_at = unix_time();
                    service.record.latest = latest;
                    service.retry_after = 0;
                    service.status_key = check_status(&service.record);
                    if let Some(release) = service.record.latest.as_ref().filter(|r| newer_than_current(r)) {
                        if notifications && (manual || service.record.notified_tag.as_deref() != Some(&release.tag_name)) {
                            notify_release = Some(release.clone());
                            service.record.notified_tag = Some(release.tag_name.clone());
                        }
                    }
                    if let Err(error) = service.persist() {
                        tracing::warn!("保存更新检查记录失败：{error}");
                    }
                }
                Err(error) => {
                    tracing::warn!("检查版本更新失败：{error}");
                    service.error = Some(error);
                    service.status_key = "updates.status.failed";
                    service.retry_after = unix_time().saturating_add(3600);
                }
            }
        });
        cx.refresh_windows();
        let translator = i18n::translator(cx);
        if let Some(release) = notify_release {
            let title = translator.translate_with_args("updates.found", &[("version", release.version_label())]);
            let detail = translator.translate_with_args("updates.notification.detail", &[
                ("current", format!("v{}", env!("CARGO_PKG_VERSION"))),
                ("latest", release.version_label()),
            ]);
            let mut request = NotificationRequest::text(title, detail);
            request.duration = Duration::from_secs(12);
            request.action_url = Some(release.url());
            request.action_label = Some(translator.translate("updates.open_release"));
            NotificationService::show(request, cx);
        } else if manual && cx.global::<Self>().status_key != "updates.status.available" {
            let service = cx.global::<Self>();
            let message = service.error.clone().unwrap_or_else(|| translator.translate(service.status_key));
            NotificationService::show_message(
                translator.translate("updates.check"), message,
                if service.error.is_some() { NotificationLevel::Error } else { NotificationLevel::Info }, cx,
            );
        }
    }

    fn persist(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&self.record).map_err(|e| e.to_string())?;
        let mut file = AtomicWriteFile::open(&self.path).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.commit().map_err(|e| e.to_string())
    }
}

fn unix_time() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn check_status(record: &CheckRecord) -> &'static str {
    match record.latest.as_ref() {
        Some(release) if newer_than_current(release) => "updates.status.available",
        Some(_) => "updates.status.current",
        None if record.checked_at == 0 => "updates.status.idle",
        None => "updates.status.empty",
    }
}
