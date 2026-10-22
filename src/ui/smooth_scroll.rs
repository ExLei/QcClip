use std::time::Instant;

use gpui::{Along as _, Axis, IsZero, Pixels, Point, ScrollHandle, ScrollWheelEvent, Window, px};
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
    target: Pixels,
    last_offset: Pixels,
    last_frame_at: Instant,
}

pub(crate) struct SmoothScroll {
    enabled: bool,
    animation: Option<ScrollAnimation>,
    frame_scheduled: bool,
    axis: Axis,
}

impl SmoothScroll {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            animation: None,
            frame_scheduled: false,
            axis: Axis::Vertical,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.animation = None;
        self.frame_scheduled = false;
    }

    pub(crate) fn set_axis(&mut self, axis: Axis) {
        if self.axis != axis {
            self.reset();
            self.axis = axis;
        }
    }

    pub(crate) fn handle_wheel<T: SmoothScrollTarget>(
        &mut self,
        scroll_handle: &T,
        event: &ScrollWheelEvent,
        window: &Window,
        animations_enabled: bool,
    ) -> bool {
        let delta = wheel_delta(event.delta.pixel_delta(window.line_height()), self.axis);
        if delta.is_zero() {
            return false;
        }

        let current_offset = scroll_handle.offset();
        let current = current_offset.along(self.axis);
        let max_offset = scroll_handle.max_offset().along(self.axis);
        let should_smooth = self.enabled && animations_enabled && !event.delta.precise();
        let target = if should_smooth {
            let target = self
                .animation
                .filter(|animation| animation.last_offset == current)
                .map(|animation| animation.target)
                .unwrap_or(current);
            (target + delta).clamp(-max_offset, Pixels::ZERO)
        } else {
            (current + delta).clamp(-max_offset, Pixels::ZERO)
        };

        if should_smooth {
            let last_frame_at = self
                .animation
                .filter(|animation| animation.last_offset == current)
                .map(|animation| animation.last_frame_at)
                .unwrap_or_else(Instant::now);
            self.animation = Some(ScrollAnimation {
                target,
                last_offset: current,
                last_frame_at,
            });
            self.schedule_next_frame(window);
        } else {
            scroll_handle.set_offset(current_offset.apply_along(self.axis, |_| target));
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
        let current = current_offset.along(self.axis);
        if current != animation.last_offset {
            return;
        }

        let max_offset = scroll_handle.max_offset().along(self.axis);
        animation.target = animation.target.clamp(-max_offset, Pixels::ZERO);
        let now = Instant::now();
        let elapsed = now
            .saturating_duration_since(animation.last_frame_at)
            .as_secs_f32()
            .min(MAX_FRAME_SECONDS);
        animation.last_frame_at = now;
        let distance = animation.target - current;
        if distance.abs() <= px(SETTLE_DISTANCE) {
            scroll_handle.set_offset(current_offset.apply_along(self.axis, |_| animation.target));
            return;
        }

        let progress = 1.0 - (-elapsed / SMOOTHING_SECONDS).exp();
        let offset = current_offset.apply_along(self.axis, |value| value + distance * progress);
        animation.last_offset = offset.along(self.axis);
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

fn wheel_delta(delta: Point<Pixels>, axis: Axis) -> Pixels {
    match axis {
        Axis::Vertical => delta.y,
        // 横向触控板使用横轴，普通滚轮和以纵轴为主的手势映射到横轴。
        Axis::Horizontal if delta.x.abs() > delta.y.abs() => delta.x,
        Axis::Horizontal => delta.y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_scrolling_handles_mouse_wheel_and_trackpad_without_double_counting() {
        for (x, y, expected) in [(0.0, -40.0, -40.0), (-30.0, -2.0, -30.0), (2.0, 30.0, 30.0)] {
            let delta = Point { x: px(x), y: px(y) };
            assert_eq!(wheel_delta(delta, Axis::Horizontal), px(expected));
            assert_eq!(wheel_delta(delta, Axis::Vertical), px(y));
        }
    }

    #[test]
    fn switching_scroll_axis_cancels_the_previous_animation() {
        let mut scroll = SmoothScroll::default();
        scroll.animation = Some(ScrollAnimation {
            target: px(-100.0),
            last_offset: px(-20.0),
            last_frame_at: Instant::now(),
        });
        scroll.set_axis(Axis::Horizontal);
        assert!(scroll.animation.is_none());
    }
}

impl Default for SmoothScroll {
    fn default() -> Self {
        Self::new(true)
    }
}
