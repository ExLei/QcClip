use std::time::Duration;

pub(super) fn next_check_delay(
    interval: Option<u64>,
    checked_at: u64,
    retry_after: u64,
    now: u64,
) -> Option<Duration> {
    let interval = interval?;
    let due_at = if checked_at == 0 || checked_at > now {
        now
    } else {
        checked_at.saturating_add(interval)
    };
    Some(Duration::from_secs(
        due_at.max(retry_after).saturating_sub(now),
    ))
}
