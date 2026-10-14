use std::time::Instant;

use gpui::{IsZero, Pixels, Point, ScrollHandle, ScrollWheelEvent, Window, px};
use gpui_component::VirtualListScrollHandle;

const SMOOTHING_SECONDS: f32 = 0.11;
const SETTLE_DISTANCE: f32 = 0.5;
const MAX_FRAME_SECONDS: f32 = 0.1;

pub(crate) trait SmoothScrollTarget {
    fn offset(&self) -> Point<Pixels>;
    fn max_offset(&self) -> Point<Pixels>;
    fn set_offset(&self, offset: Point<Pixels>);
}

impl SmoothScrollTarget for ScrollHandle {
    fn offset(&self) -> Point<Pixels> {
        self.offset()
    }

    fn max_offset(&self) -> Point<Pixels> {
        self.max_offset()
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.set_offset(offset);
    }
}

impl SmoothScrollTarget for VirtualListScrollHandle {
    fn offset(&self) -> Point<Pixels> {
        self.as_ref().offset()
    }

    fn max_offset(&self) -> Point<Pixels> {
        self.as_ref().max_offset()
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.as_ref().set_offset(offset);
    }
}

#[derive(Clone, Copy)]
struct ScrollAnimation {
    target_y: Pixels,
    last_offset_y: Pixels,
    last_frame_at: Instant,
}

pub(crate) struct SmoothScroll {
    enabled: bool,
    animation: Option<ScrollAnimation>,
    frame_scheduled: bool,
}

impl SmoothScroll {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            animation: None,
            frame_scheduled: false,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.animation = None;
        self.frame_scheduled = false;
    }

    pub(crate) fn handle_wheel<T: SmoothScrollTarget>(
        &mut self,
        scroll_handle: &T,
        event: &ScrollWheelEvent,
        window: &Window,
        animations_enabled: bool,
    ) -> bool {
        let delta = event.delta.pixel_delta(window.line_height());
        if delta.y.is_zero() {
            return false;
        }

        let current_offset = scroll_handle.offset();
        let max_offset = scroll_handle.max_offset();
        let should_smooth = self.enabled && animations_enabled && !event.delta.precise();
        let target_y = if should_smooth {
            let target_y = self
                .animation
                .filter(|animation| animation.last_offset_y == current_offset.y)
                .map(|animation| animation.target_y)
                .unwrap_or(current_offset.y);
            (target_y + delta.y).clamp(-max_offset.y, Pixels::ZERO)
        } else {
            (current_offset.y + delta.y).clamp(-max_offset.y, Pixels::ZERO)
        };

        if should_smooth {
            let last_frame_at = self
                .animation
                .filter(|animation| animation.last_offset_y == current_offset.y)
                .map(|animation| animation.last_frame_at)
                .unwrap_or_else(Instant::now);
            self.animation = Some(ScrollAnimation {
                target_y,
                last_offset_y: current_offset.y,
                last_frame_at,
            });
            self.schedule_next_frame(window);
        } else {
            let mut offset = current_offset;
            offset.y = target_y;
            scroll_handle.set_offset(offset);
            self.animation = None;
        }

        true
    }

    pub(crate) fn advance<T: SmoothScrollTarget>(&mut self, scroll_handle: &T, window: &Window) {
        self.frame_scheduled = false;
        let Some(mut animation) = self.animation.take() else {
            return;
        };

        let current_offset = scroll_handle.offset();
        if current_offset.y != animation.last_offset_y {
            return;
        }

        let max_offset = scroll_handle.max_offset();
        animation.target_y = animation.target_y.clamp(-max_offset.y, Pixels::ZERO);
        let now = Instant::now();
        let elapsed = now
            .saturating_duration_since(animation.last_frame_at)
            .as_secs_f32()
            .min(MAX_FRAME_SECONDS);
        animation.last_frame_at = now;
        let distance = animation.target_y - current_offset.y;
        if distance.abs() <= px(SETTLE_DISTANCE) {
            let mut offset = current_offset;
            offset.y = animation.target_y;
            scroll_handle.set_offset(offset);
            return;
        }

        let mut offset = current_offset;
        let progress = 1.0 - (-elapsed / SMOOTHING_SECONDS).exp();
        offset.y += distance * progress;
        animation.last_offset_y = offset.y;
        scroll_handle.set_offset(offset);
        self.animation = Some(animation);
        self.schedule_next_frame(window);
    }

    fn schedule_next_frame(&mut self, window: &Window) {
        if self.frame_scheduled {
            return;
        }

        self.frame_scheduled = true;
        // 滚轮回调不一定处在视图渲染栈中，不能调用依赖 current_view 的动画帧 API。
        window.on_next_frame(|window, _| window.refresh());
    }
}

impl Default for SmoothScroll {
    fn default() -> Self {
        Self::new(true)
    }
}
