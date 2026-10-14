use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{App, BorrowAppContext as _, Context, Window};

use crate::platform::window::BorderlessPopup;

use super::{layout::MenuRect, service::MenuService};

const MENU_ENTRANCE_DURATION: Duration = Duration::from_millis(140);
const MENU_ENTRANCE_FRAME: Duration = Duration::from_millis(16);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MenuEntranceDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum PopupPhase {
    #[default]
    Preparing,
    BoundsPending,
    Animating,
    Complete,
}

/// 先按最终尺寸布局菜单，再只调整原生窗口的屏幕边界。
#[derive(Default)]
pub(super) struct MenuPopupEntrance {
    phase: Rc<Cell<PopupPhase>>,
    target_bounds: Cell<Option<MenuRect>>,
    current_bounds: Rc<Cell<Option<MenuRect>>>,
}

impl MenuPopupEntrance {
    pub(super) fn target_bounds(&self) -> Option<MenuRect> {
        self.target_bounds.get()
    }

    pub(super) fn current_bounds(&self) -> Option<MenuRect> {
        self.current_bounds.get()
    }

    pub(super) fn update_window<V: 'static>(
        &self,
        popup: BorderlessPopup,
        generation: u64,
        level: usize,
        target_bounds: MenuRect,
        initial_bounds: MenuRect,
        window: &mut Window,
        cx: &mut Context<V>,
    ) {
        self.target_bounds.set(Some(target_bounds));

        match self.phase.get() {
            PopupPhase::Preparing => {
                let animation_enabled = !cx.reduce_motion()
                    && crate::config_runtime::animation_enabled(
                        cx,
                        crate::config::AnimationKind::MenuPopup,
                    );
                self.phase.set(PopupPhase::BoundsPending);
                let phase = self.phase.clone();
                let current_bounds = self.current_bounds.clone();
                window.defer(cx, move |window, cx| {
                    if popup.prepare_to_physical_bounds(
                        window,
                        cx,
                        target_bounds.x,
                        target_bounds.y,
                        target_bounds.width,
                        target_bounds.height,
                    ) {
                        current_bounds.set(Some(native_bounds(target_bounds)));
                        set_service_bounds(cx, generation, level, target_bounds);
                        if !animation_enabled || !cfg!(target_os = "windows") {
                            show_complete(popup, window, cx, target_bounds);
                            phase.set(PopupPhase::Complete);
                        } else if popup.set_animation_bounds(
                            window,
                            cx,
                            initial_bounds.x,
                            initial_bounds.y,
                            initial_bounds.width,
                            initial_bounds.height,
                        ) {
                            current_bounds.set(Some(native_bounds(initial_bounds)));
                            popup.set_visible(window, true);
                            phase.set(PopupPhase::Animating);
                            animate_bounds(
                                window,
                                cx,
                                popup,
                                initial_bounds,
                                target_bounds,
                                phase.clone(),
                                current_bounds.clone(),
                            );
                        } else {
                            current_bounds.set(Some(native_bounds(target_bounds)));
                            show_complete(popup, window, cx, target_bounds);
                            phase.set(PopupPhase::Complete);
                        }
                        window.refresh();
                    } else {
                        if !popup.resize_to_physical_bounds(
                            window,
                            cx,
                            target_bounds.x,
                            target_bounds.y,
                            target_bounds.width,
                            target_bounds.height,
                        ) {
                            show_complete(popup, window, cx, target_bounds);
                        }
                        current_bounds.set(Some(native_bounds(target_bounds)));
                        set_service_bounds(cx, generation, level, target_bounds);
                        phase.set(PopupPhase::Complete);
                        window.refresh();
                    }
                });
            }
            PopupPhase::BoundsPending | PopupPhase::Animating | PopupPhase::Complete => {}
        }
    }
}

fn animate_bounds(
    window: &mut Window,
    cx: &mut App,
    popup: BorderlessPopup,
    initial_bounds: MenuRect,
    target_bounds: MenuRect,
    phase: Rc<Cell<PopupPhase>>,
    current_bounds: Rc<Cell<Option<MenuRect>>>,
) {
    window
        .spawn(cx, async move |cx| {
            let started_at = Instant::now();
            loop {
                cx.background_executor().timer(MENU_ENTRANCE_FRAME).await;
                let progress = (started_at.elapsed().as_secs_f32()
                    / MENU_ENTRANCE_DURATION.as_secs_f32())
                    .clamp(0.0, 1.0);
                let progress = attached_window_easing(progress);
                let bounds = interpolate_bounds(initial_bounds, target_bounds, progress);
                let updated = cx
                    .update(|window, cx| {
                        let updated = popup.set_animation_bounds(
                            window,
                            cx,
                            bounds.x,
                            bounds.y,
                            bounds.width,
                            bounds.height,
                        );
                        if updated {
                            current_bounds.set(Some(native_bounds(bounds)));
                            window.refresh();
                        }
                        updated
                    })
                    .unwrap_or(false);
                if !updated {
                    let _ = cx.update(|window, cx| {
                        show_complete(popup, window, cx, target_bounds)
                    });
                    current_bounds.set(Some(native_bounds(target_bounds)));
                    phase.set(PopupPhase::Complete);
                    break;
                }
                if progress >= 1.0 {
                    phase.set(PopupPhase::Complete);
                    break;
                }
            }
        })
        .detach();
}

fn show_complete(popup: BorderlessPopup, window: &mut Window, cx: &mut App, bounds: MenuRect) {
    let _ = popup.set_animation_bounds(
        window,
        cx,
        bounds.x,
        bounds.y,
        bounds.width,
        bounds.height,
    );
    popup.set_visible(window, true);
    window.refresh();
}

fn set_service_bounds(cx: &mut App, generation: u64, level: usize, bounds: MenuRect) {
    cx.update_global::<MenuService, _>(|service, _| {
        service.set_window_bounds(generation, level, bounds);
    });
}

fn native_bounds(bounds: MenuRect) -> MenuRect {
    MenuRect::new(
        bounds.x.round(),
        bounds.y.round(),
        bounds.width.round().max(1.0),
        bounds.height.round().max(1.0),
    )
}

fn interpolate_bounds(start: MenuRect, target: MenuRect, progress: f32) -> MenuRect {
    let progress = progress.clamp(0.0, 1.0);
    let left = interpolate(start.x, target.x, progress);
    let top = interpolate(start.y, target.y, progress);
    let right = interpolate(start.right(), target.right(), progress);
    let bottom = interpolate(start.bottom(), target.bottom(), progress);
    MenuRect::new(left, top, right - left, bottom - top)
}

fn attached_window_easing(progress: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    if progress <= 0.0 || progress >= 1.0 {
        return progress;
    }

    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..20 {
        let middle = (low + high) * 0.5;
        if cubic_bezier_axis(middle, 0.4, 0.2) < progress {
            low = middle;
        } else {
            high = middle;
        }
    }
    cubic_bezier_axis((low + high) * 0.5, 0.0, 1.0)
}

fn cubic_bezier_axis(t: f32, first: f32, second: f32) -> f32 {
    let inverse = 1.0 - t;
    3.0 * inverse * inverse * t * first + 3.0 * inverse * t * t * second + t * t * t
}

pub(super) fn vertical_entrance_direction(
    anchor_y: f32,
    target_bounds: MenuRect,
) -> MenuEntranceDirection {
    if target_bounds.y >= anchor_y {
        MenuEntranceDirection::Down
    } else {
        MenuEntranceDirection::Up
    }
}

pub(super) fn animated_content_bounds(
    target_bounds: MenuRect,
    direction: MenuEntranceDirection,
    progress: f32,
) -> MenuRect {
    let progress = progress.clamp(0.0, 1.0);
    match direction {
        MenuEntranceDirection::Down => MenuRect::new(
            target_bounds.x,
            target_bounds.y,
            target_bounds.width,
            animated_size(target_bounds.height, progress),
        ),
        MenuEntranceDirection::Up => {
            let height = animated_size(target_bounds.height, progress);
            MenuRect::new(
                target_bounds.x,
                target_bounds.bottom() - height,
                target_bounds.width,
                height,
            )
        }
        MenuEntranceDirection::Right => MenuRect::new(
            target_bounds.x,
            target_bounds.y,
            animated_size(target_bounds.width, progress),
            target_bounds.height,
        ),
        MenuEntranceDirection::Left => {
            let width = animated_size(target_bounds.width, progress);
            MenuRect::new(
                target_bounds.right() - width,
                target_bounds.y,
                width,
                target_bounds.height,
            )
        }
    }
}

pub(super) fn submenu_content_bounds(
    target_bounds: MenuRect,
    item_bounds: MenuRect,
    direction: MenuEntranceDirection,
    progress: f32,
) -> MenuRect {
    let progress = progress.clamp(0.0, 1.0);
    let (start_left, start_right) = match direction {
        MenuEntranceDirection::Right => (item_bounds.right() - 1.0, item_bounds.right()),
        MenuEntranceDirection::Left => (item_bounds.x, item_bounds.x + 1.0),
        MenuEntranceDirection::Up | MenuEntranceDirection::Down => {
            (item_bounds.x, item_bounds.right())
        }
    };
    let left = interpolate(start_left, target_bounds.x, progress);
    let right = interpolate(start_right, target_bounds.right(), progress);
    let top = interpolate(item_bounds.y, target_bounds.y, progress);
    let bottom = interpolate(item_bounds.bottom(), target_bounds.bottom(), progress);
    MenuRect::new(left, top, right - left, bottom - top)
}

pub(super) fn point_anchored_content_bounds(
    target_bounds: MenuRect,
    anchor_x: f32,
    anchor_y: f32,
    progress: f32,
) -> MenuRect {
    let progress = progress.clamp(0.0, 1.0);
    let (left, right) = point_anchored_axis(
        target_bounds.x,
        target_bounds.right(),
        anchor_x,
        progress,
    );
    let (top, bottom) = point_anchored_axis(
        target_bounds.y,
        target_bounds.bottom(),
        anchor_y,
        progress,
    );
    MenuRect::new(left, top, right - left, bottom - top)
}

fn animated_size(target_size: f32, progress: f32) -> f32 {
    (target_size * progress).max(1.0)
}

fn interpolate(start: f32, end: f32, progress: f32) -> f32 {
    start + (end - start) * progress
}

fn point_anchored_axis(
    target_start: f32,
    target_end: f32,
    anchor: f32,
    progress: f32,
) -> (f32, f32) {
    let anchor = anchor.clamp(target_start, target_end);
    let start = interpolate(anchor, target_start, progress);
    let end = interpolate(anchor, target_end, progress);
    if end - start >= 1.0 || target_end - target_start <= 1.0 {
        return (start, end);
    }
    let start = (anchor - 0.5).clamp(target_start, target_end - 1.0);
    (start, start + 1.0)
}
