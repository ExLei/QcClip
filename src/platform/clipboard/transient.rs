use std::{
    collections::VecDeque,
    sync::mpsc::{Receiver, RecvTimeoutError, Sender},
    time::{Duration, Instant},
};

use crate::{
    domain::capture::{
        CapturedClipboard, RestoreState, classify_format_role, normalize_text_bytes,
    },
    services::capture::hash_format_set,
};

const RESTORE_WINDOW: Duration = Duration::from_millis(300);

pub(super) struct ClipboardObservation {
    pub captured: CapturedClipboard,
    pub recordable: bool,
    pub observed_at: Instant,
}

struct PendingCapture {
    observation: ClipboardObservation,
    fingerprint: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClipboardFingerprint {
    Empty,
    Content([u8; 32]),
}

struct TransientClipboardFilter {
    baseline: Option<ClipboardFingerprint>,
    clipboard_is_empty: bool,
    pending: VecDeque<PendingCapture>,
}

impl TransientClipboardFilter {
    fn new(initial: Option<&CapturedClipboard>) -> Self {
        let baseline = initial.and_then(fingerprint);
        Self {
            baseline,
            clipboard_is_empty: baseline == Some(ClipboardFingerprint::Empty),
            pending: VecDeque::new(),
        }
    }

    fn observe(&mut self, observation: ClipboardObservation) -> Vec<CapturedClipboard> {
        let ready = self.take_ready(observation.observed_at);
        let Some(fingerprint) = fingerprint(&observation.captured) else {
            tracing::debug!("剪贴板快照没有可比对的有效内容，保留稳定基准和待确认内容");
            return ready;
        };

        // 真实清空与读取失败分开处理，同时保留最后的有效基准用于识别旧内容还原。
        self.clipboard_is_empty = fingerprint == ClipboardFingerprint::Empty;
        let ClipboardFingerprint::Content(fingerprint) = fingerprint else {
            return ready;
        };

        // 临时内容可能未被捕获，还原也可能产生多次事件；稳定内容未变化时统一跳过。
        if self.baseline == Some(ClipboardFingerprint::Content(fingerprint)) {
            self.pending.clear();
            tracing::debug!("剪贴板内容与稳定内容相同，跳过重复捕获或还原事件");
            return ready;
        }

        if let Some(index) = self
            .pending
            .iter()
            .rposition(|pending| pending.fingerprint == fingerprint)
        {
            // 只撤销还原点之后的内容，保留此前尚未入库的正常复制。
            self.pending.truncate(index + 1);
            return ready;
        }

        self.pending.push_back(PendingCapture {
            observation,
            fingerprint,
        });
        ready
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.pending
            .front()
            .map(|pending| pending.observation.observed_at + RESTORE_WINDOW)
    }

    fn take_ready(&mut self, now: Instant) -> Vec<CapturedClipboard> {
        let mut ready = Vec::new();
        while self.next_deadline().is_some_and(|deadline| deadline <= now) {
            let pending = self.pending.pop_front().expect("到期的剪贴板事件应存在");
            // 到期前重新出现有效内容时仍保留正常快速复制；持续为空则丢弃临时内容。
            if self.clipboard_is_empty {
                tracing::debug!("剪贴板内容在确认前已清空，丢弃临时捕获");
                continue;
            }
            self.baseline = Some(ClipboardFingerprint::Content(pending.fingerprint));
            if pending.observation.recordable {
                ready.push(pending.observation.captured);
            }
        }
        ready
    }
}

fn fingerprint(captured: &CapturedClipboard) -> Option<ClipboardFingerprint> {
    if captured.formats.is_empty() {
        return Some(ClipboardFingerprint::Empty);
    }
    let has_unicode = captured.formats.iter().any(|format| {
        format.format_namespace == "windows"
            && format.format_key == "CF_UNICODETEXT"
            && format.capture_state.has_data()
            && format.restore_state == RestoreState::Restorable
            && format.bytes.as_ref().is_some_and(|bytes| !bytes.is_empty())
    });
    let mut formats = captured
        .formats
        .iter()
        .filter_map(|format| {
            if !format.capture_state.has_data()
                || format.restore_state != RestoreState::Restorable
                || classify_format_role(format.format_kind, &format.format_key, &format.format_name)
                    .is_internal()
            {
                return None;
            }
            if format.format_namespace == "windows" {
                // ANSI 派生文本和控制标记不属于还原比对内容，避免编码或元数据变化干扰判断。
                if has_unicode && matches!(format.format_key.as_str(), "CF_TEXT" | "CF_OEMTEXT") {
                    return None;
                }
                if [
                    "ExcludeClipboardContentFromMonitorProcessing",
                    "CanIncludeInClipboardHistory",
                    "CanUploadToCloudClipboard",
                ]
                .iter()
                .any(|name| format.format_name.eq_ignore_ascii_case(name))
                {
                    return None;
                }
            }
            let bytes = format.bytes.as_deref().filter(|bytes| !bytes.is_empty())?;
            Some((
                format.format_namespace.as_str(),
                format.format_key.as_str(),
                normalize_text_bytes(&format.format_namespace, &format.format_key, bytes),
            ))
        })
        .peekable();
    formats.peek()?;
    Some(ClipboardFingerprint::Content(hash_format_set(formats)))
}

pub(super) fn filter_events(
    receiver: Receiver<Result<ClipboardObservation, String>>,
    sender: Sender<Result<CapturedClipboard, String>>,
    initial: Option<CapturedClipboard>,
) {
    let mut filter = TransientClipboardFilter::new(initial.as_ref());
    drop(initial);
    loop {
        let event = match filter.next_deadline() {
            Some(deadline) => {
                receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            }
            None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        let ready = match event {
            Ok(Ok(observation)) => filter.observe(observation),
            Ok(Err(error)) => {
                if sender.send(Err(error)).is_err() {
                    break;
                }
                continue;
            }
            Err(RecvTimeoutError::Timeout) => filter.take_ready(Instant::now()),
            // 停止监听时丢弃尚未确认的内容，避免关闭监听后继续入库和提示。
            Err(RecvTimeoutError::Disconnected) => break,
        };
        for captured in ready {
            if sender.send(Ok(captured)).is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::capture::{FormatKind, RawFormat, SourceApplication};

    fn text(value: &str) -> CapturedClipboard {
        CapturedClipboard {
            formats: vec![RawFormat::captured(
                "windows",
                "CF_UNICODETEXT",
                "CF_UNICODETEXT",
                FormatKind::Text,
                value
                    .encode_utf16()
                    .chain([0])
                    .flat_map(u16::to_le_bytes)
                    .collect(),
                0,
                true,
            )],
            primary_format_identity: None,
            preview_candidate: None,
            source_application: SourceApplication::default(),
        }
    }

    fn observation(
        captured: CapturedClipboard,
        observed_at: Instant,
        recordable: bool,
    ) -> ClipboardObservation {
        ClipboardObservation {
            captured,
            observed_at,
            recordable,
        }
    }

    fn invalid_snapshots() -> Vec<CapturedClipboard> {
        let mut snapshots = Vec::new();
        let mut empty = text("");
        empty.formats.clear();
        snapshots.push(empty);

        for format in [
            RawFormat::failed(
                "windows",
                "CF_UNICODETEXT",
                "CF_UNICODETEXT",
                FormatKind::Text,
                0,
                "clipboard_read_failed",
                "读取剪贴板失败",
            ),
            RawFormat::unsupported(
                "windows",
                "CF_BITMAP",
                "CF_BITMAP",
                FormatKind::Image,
                0,
                "native_handle_unsupported",
                "不支持该句柄格式",
            ),
            RawFormat::captured(
                "windows",
                "CF_UNICODETEXT",
                "CF_UNICODETEXT",
                FormatKind::Text,
                Vec::new(),
                0,
                true,
            ),
            RawFormat::captured(
                "windows",
                "CF_BITMAP",
                "CF_BITMAP",
                FormatKind::Image,
                vec![1; 4],
                0,
                false,
            ),
            RawFormat::captured(
                "windows",
                "CF_LOCALE",
                "CF_LOCALE",
                FormatKind::Custom,
                vec![1; 4],
                0,
                true,
            ),
            RawFormat::captured(
                "windows",
                "registered:CanUploadToCloudClipboard",
                "CanUploadToCloudClipboard",
                FormatKind::Custom,
                vec![1; 4],
                0,
                true,
            ),
        ] {
            let mut captured = text("");
            captured.formats = vec![format];
            snapshots.push(captured);
        }
        let mut missing_bytes = text("读取不完整");
        missing_bytes.formats[0].bytes = None;
        snapshots.push(missing_bytes);
        snapshots
    }

    #[test]
    fn invalid_snapshots_do_not_replace_the_baseline_even_after_the_restore_window() {
        let now = Instant::now();
        let original = text("原内容");
        for invalid in invalid_snapshots() {
            let mut filter = TransientClipboardFilter::new(Some(&original));
            assert!(filter.observe(observation(invalid, now, true)).is_empty());
            assert!(filter.take_ready(now + RESTORE_WINDOW).is_empty());
            assert!(filter.next_deadline().is_none());
            assert!(
                filter
                    .observe(observation(
                        original.clone(),
                        now + RESTORE_WINDOW * 2,
                        true,
                    ))
                    .is_empty()
            );
            assert!(filter.take_ready(now + RESTORE_WINDOW * 3).is_empty());

            let next = text("主动复制的新内容");
            filter.observe(observation(next.clone(), now + RESTORE_WINDOW * 3, true));
            assert_eq!(filter.take_ready(now + RESTORE_WINDOW * 4), vec![next]);
        }
    }

    #[test]
    fn unreadable_snapshots_preserve_pending_normal_copies() {
        let now = Instant::now();
        let next = text("正常快速复制");
        for invalid in invalid_snapshots()
            .into_iter()
            .filter(|snapshot| !snapshot.formats.is_empty())
        {
            let mut filter = TransientClipboardFilter::new(Some(&text("原内容")));
            filter.observe(observation(next.clone(), now, true));
            assert!(
                filter
                    .observe(observation(invalid, now + Duration::from_millis(50), true,))
                    .is_empty()
            );
            assert_eq!(filter.take_ready(now + RESTORE_WINDOW), vec![next.clone()]);
        }
    }

    #[test]
    fn invalid_initial_snapshots_do_not_discard_the_first_valid_copy() {
        let now = Instant::now();
        let first = text("第一份内容");
        for invalid in invalid_snapshots() {
            let mut filter = TransientClipboardFilter::new(Some(&invalid));
            filter.observe(observation(first.clone(), now, true));
            if !invalid.formats.is_empty() {
                filter.observe(observation(invalid, now + Duration::from_millis(50), true));
            }
            assert_eq!(filter.take_ready(now + RESTORE_WINDOW), vec![first.clone()]);
        }
    }

    #[test]
    fn temporary_copy_and_restore_do_not_reach_history_or_notifications() {
        let now = Instant::now();
        let original = text("原内容");
        let mut filter = TransientClipboardFilter::new(Some(&original));
        assert!(
            filter
                .observe(observation(text("划词内容"), now, true))
                .is_empty()
        );
        assert!(
            filter
                .observe(observation(original, now + Duration::from_millis(52), true))
                .is_empty()
        );
        assert!(filter.take_ready(now + RESTORE_WINDOW).is_empty());
        assert!(filter.next_deadline().is_none());
    }

    #[test]
    fn rapid_distinct_copies_are_all_retained_at_their_own_deadlines() {
        let now = Instant::now();
        let mut filter = TransientClipboardFilter::new(Some(&text("原内容")));
        let copies = [text("第一条"), text("第二条"), text("第三条")];
        for (index, captured) in copies.iter().enumerate() {
            assert!(
                filter
                    .observe(observation(
                        captured.clone(),
                        now + Duration::from_millis(index as u64 * 50),
                        true
                    ))
                    .is_empty()
            );
        }
        assert!(
            filter
                .take_ready(now + RESTORE_WINDOW - Duration::from_millis(1))
                .is_empty()
        );
        for (index, captured) in copies.into_iter().enumerate() {
            assert_eq!(
                filter.take_ready(now + RESTORE_WINDOW + Duration::from_millis(index as u64 * 50)),
                vec![captured]
            );
        }
    }

    #[test]
    fn restoring_a_pending_normal_copy_preserves_it_and_discards_only_temporary_content() {
        let now = Instant::now();
        let original = text("新复制的内容");
        let mut filter = TransientClipboardFilter::new(Some(&text("旧内容")));
        filter.observe(observation(original.clone(), now, true));
        filter.observe(observation(
            text("划词内容"),
            now + Duration::from_millis(50),
            true,
        ));
        filter.observe(observation(
            original.clone(),
            now + Duration::from_millis(100),
            true,
        ));
        assert_eq!(filter.take_ready(now + RESTORE_WINDOW), vec![original]);
    }

    #[test]
    fn copies_outside_the_restore_window_are_retained() {
        let now = Instant::now();
        let original = text("原内容");
        let next = text("主动复制的内容");
        let mut filter = TransientClipboardFilter::new(Some(&original));
        filter.observe(observation(next.clone(), now, true));
        assert_eq!(
            filter.observe(observation(original.clone(), now + RESTORE_WINDOW, true)),
            vec![next]
        );
        assert_eq!(filter.take_ready(now + RESTORE_WINDOW * 2), vec![original]);
    }

    #[test]
    fn self_writes_update_restore_baseline_without_becoming_history_events() {
        let now = Instant::now();
        let own_copy = text("从历史复制的内容");
        let mut filter = TransientClipboardFilter::new(Some(&text("旧内容")));
        filter.observe(observation(own_copy.clone(), now, false));
        assert!(filter.take_ready(now + RESTORE_WINDOW).is_empty());
        filter.observe(observation(text("划词内容"), now + RESTORE_WINDOW, true));
        filter.observe(observation(
            own_copy,
            now + RESTORE_WINDOW + Duration::from_millis(63),
            true,
        ));
        assert!(filter.take_ready(now + RESTORE_WINDOW * 2).is_empty());
    }

    #[test]
    fn restoring_an_empty_clipboard_also_discards_temporary_content() {
        let now = Instant::now();
        let mut empty = text("");
        empty.formats.clear();
        let mut filter = TransientClipboardFilter::new(Some(&empty));
        filter.observe(observation(text("划词内容"), now, true));
        filter.observe(observation(empty, now + Duration::from_millis(50), true));
        assert!(filter.take_ready(now + RESTORE_WINDOW).is_empty());
    }

    #[test]
    fn runtime_clear_keeps_filtering_repeated_selection_copies_after_a_normal_copy() {
        let now = Instant::now();
        let mut empty = text("");
        empty.formats.clear();
        let mut filter = TransientClipboardFilter::new(Some(&text("正常复制的内容")));
        filter.observe(observation(empty.clone(), now, true));
        assert!(filter.take_ready(now + RESTORE_WINDOW).is_empty());

        for index in 1..=5 {
            let start = now + Duration::from_secs(index);
            filter.observe(observation(text(&format!("划词内容{index}")), start, true));
            filter.observe(observation(
                empty.clone(),
                start + Duration::from_millis(50),
                true,
            ));
            assert!(filter.take_ready(start + RESTORE_WINDOW).is_empty());
            assert!(filter.next_deadline().is_none());
        }

        let normal = text("后续正常复制");
        filter.observe(observation(
            normal.clone(),
            now + Duration::from_secs(6),
            true,
        ));
        assert_eq!(
            filter.take_ready(now + Duration::from_secs(6) + RESTORE_WINDOW),
            vec![normal]
        );
    }

    #[test]
    fn clearing_a_selection_without_an_initial_baseline_discards_it() {
        let now = Instant::now();
        let mut empty = text("");
        empty.formats.clear();
        let mut filter = TransientClipboardFilter::new(None);
        filter.observe(observation(text("划词内容"), now, true));
        filter.observe(observation(empty, now + Duration::from_millis(50), true));
        assert!(filter.take_ready(now + RESTORE_WINDOW).is_empty());
    }

    #[test]
    fn brief_clear_between_normal_copies_preserves_both_copies() {
        let now = Instant::now();
        let mut empty = text("");
        empty.formats.clear();
        for initial in [text("原内容"), empty.clone()] {
            let mut filter = TransientClipboardFilter::new(Some(&initial));
            let first = text("第一条正常复制");
            let second = text("第二条正常复制");
            filter.observe(observation(first.clone(), now, true));
            filter.observe(observation(
                empty.clone(),
                now + Duration::from_millis(50),
                true,
            ));
            filter.observe(observation(
                second.clone(),
                now + Duration::from_millis(100),
                true,
            ));
            assert_eq!(filter.take_ready(now + RESTORE_WINDOW), vec![first]);
            assert_eq!(
                filter.take_ready(now + RESTORE_WINDOW + Duration::from_millis(100)),
                vec![second]
            );
        }
    }

    #[test]
    fn clearing_after_the_restore_window_preserves_the_confirmed_copy() {
        let now = Instant::now();
        let normal = text("正常复制");
        let mut empty = text("");
        empty.formats.clear();
        let mut filter = TransientClipboardFilter::new(Some(&text("原内容")));
        filter.observe(observation(normal.clone(), now, true));
        assert_eq!(
            filter.observe(observation(empty, now + RESTORE_WINDOW, true)),
            vec![normal]
        );
        assert!(filter.take_ready(now + RESTORE_WINDOW * 2).is_empty());
    }

    #[test]
    fn restore_without_a_captured_temporary_copy_does_not_produce_an_event() {
        let now = Instant::now();
        let original = text("原内容");
        let mut filter = TransientClipboardFilter::new(Some(&original));
        filter.observe(observation(original, now, true));
        assert!(filter.take_ready(now + RESTORE_WINDOW).is_empty());
        assert!(filter.next_deadline().is_none());
    }

    #[test]
    fn repeated_restore_events_after_a_temporary_copy_do_not_produce_notifications() {
        let now = Instant::now();
        let original = text("原内容");
        let mut filter = TransientClipboardFilter::new(Some(&original));
        filter.observe(observation(text("划词内容"), now, true));
        for delay in [52, 53, 100, 450] {
            assert!(
                filter
                    .observe(observation(
                        original.clone(),
                        now + Duration::from_millis(delay),
                        true,
                    ))
                    .is_empty()
            );
        }
        assert!(filter.take_ready(now + Duration::from_secs(1)).is_empty());
        assert!(filter.next_deadline().is_none());
    }

    #[test]
    fn restore_after_a_self_write_is_silent_even_if_temporary_content_was_not_captured() {
        let now = Instant::now();
        let own_copy = text("从历史复制的内容");
        let mut filter = TransientClipboardFilter::new(Some(&text("旧内容")));
        filter.observe(observation(own_copy.clone(), now, false));
        assert!(filter.take_ready(now + RESTORE_WINDOW).is_empty());
        filter.observe(observation(own_copy, now + RESTORE_WINDOW, true));
        assert!(filter.take_ready(now + RESTORE_WINDOW * 2).is_empty());
    }

    #[test]
    fn first_capture_without_an_initial_baseline_is_still_recorded() {
        let now = Instant::now();
        let first = text("第一份内容");
        let mut filter = TransientClipboardFilter::new(None);
        filter.observe(observation(first.clone(), now, true));
        assert_eq!(filter.take_ready(now + RESTORE_WINDOW), vec![first]);
    }

    #[test]
    fn restore_matching_ignores_source_format_order_ansi_and_internal_metadata() {
        let original = text("原内容");
        let mut restored = original.clone();
        restored.source_application.name = Some("取词工具".to_owned());
        restored.formats[0].format_order = 4;
        restored.formats[0].bytes.as_mut().unwrap().extend([0; 8]);
        for name in [
            "CF_TEXT",
            "CF_OEMTEXT",
            "GPUI internal metadata",
            "CanUploadToCloudClipboard",
        ] {
            restored.formats.push(RawFormat::captured(
                "windows",
                name,
                name,
                FormatKind::Custom,
                vec![1; 4],
                0,
                true,
            ));
        }
        assert_eq!(fingerprint(&original), fingerprint(&restored));
    }

    #[test]
    fn matching_text_does_not_hide_changes_to_images_or_rich_text() {
        let plain = text("相同文本");
        for (key, kind) in [
            ("CF_DIB", FormatKind::Image),
            ("registered:HTML Format", FormatKind::Html),
        ] {
            let mut rich = plain.clone();
            rich.formats.push(RawFormat::captured(
                "windows",
                key,
                key,
                kind,
                vec![1; 4],
                1,
                true,
            ));
            assert_ne!(fingerprint(&plain), fingerprint(&rich));
            let mut changed = rich.clone();
            changed.formats[1].bytes = Some(vec![2; 4]);
            assert_ne!(fingerprint(&rich), fingerprint(&changed));
        }
    }

    #[test]
    fn worker_discards_pending_events_when_monitor_stops() {
        let (input, receiver) = std::sync::mpsc::channel();
        let (sender, output) = std::sync::mpsc::channel();
        input
            .send(Ok(observation(text("未确认的内容"), Instant::now(), true)))
            .unwrap();
        drop(input);
        filter_events(receiver, sender, Some(text("原内容")));
        assert!(output.try_recv().is_err());
    }

    #[test]
    fn worker_skips_invalid_snapshots_and_late_restores_but_delivers_normal_copies() {
        let now = Instant::now() - Duration::from_secs(3);
        let original = text("原内容");
        let next = text("正常复制");
        for invalid in invalid_snapshots() {
            let (input, receiver) = std::sync::mpsc::channel();
            let (sender, output) = std::sync::mpsc::channel();
            for (captured, delay) in [
                (invalid, 0),
                (original.clone(), 400),
                (next.clone(), 800),
                (text("关闭时尚未确认的内容"), 1200),
            ] {
                input
                    .send(Ok(observation(
                        captured,
                        now + Duration::from_millis(delay),
                        true,
                    )))
                    .unwrap();
            }
            input.send(Err("处理完成".to_owned())).unwrap();
            drop(input);
            filter_events(receiver, sender, Some(original.clone()));
            assert_eq!(
                output.into_iter().collect::<Vec<_>>(),
                vec![Ok(next.clone()), Err("处理完成".to_owned()),]
            );
        }
    }

    #[test]
    fn worker_filters_repeated_selection_clears_after_a_nonempty_baseline() {
        let now = Instant::now() - Duration::from_secs(4);
        let mut empty = text("");
        empty.formats.clear();
        let normal = text("正常复制");
        let (input, receiver) = std::sync::mpsc::channel();
        let (sender, output) = std::sync::mpsc::channel();
        for (captured, delay) in [
            (text("第一次划词"), 0),
            (empty.clone(), 50),
            (text("第二次划词"), 500),
            (empty.clone(), 550),
            (text("第三次划词"), 1000),
            (empty, 1050),
            (normal.clone(), 1500),
            (text("关闭时尚未确认的内容"), 1900),
        ] {
            input
                .send(Ok(observation(
                    captured,
                    now + Duration::from_millis(delay),
                    true,
                )))
                .unwrap();
        }
        drop(input);
        filter_events(receiver, sender, Some(text("原内容")));
        assert_eq!(output.into_iter().collect::<Vec<_>>(), vec![Ok(normal)]);
    }

    #[test]
    fn worker_filters_a_queued_round_trip_using_capture_times() {
        let now = Instant::now() - Duration::from_secs(1);
        let original = text("原内容");
        let (input, receiver) = std::sync::mpsc::channel();
        let (sender, output) = std::sync::mpsc::channel();
        input
            .send(Ok(observation(text("划词内容"), now, true)))
            .unwrap();
        input
            .send(Ok(observation(
                original.clone(),
                now + Duration::from_millis(52),
                true,
            )))
            .unwrap();
        drop(input);
        filter_events(receiver, sender, Some(original));
        assert!(output.try_recv().is_err());
    }
}
