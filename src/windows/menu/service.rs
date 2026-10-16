use std::rc::Rc;

use crate::platform::{input, window};
use gpui::{
    AnyWindowHandle, App, BorrowAppContext as _, Bounds, Entity, Global, Point, Render, Window,
    point, px, size,
};

use super::{
    dropdown::open_dropdown_window,
    layout::{MenuDirection, MenuRect},
    model::{DropdownAnchor, DropdownItemId, DropdownModel, MenuActionId, MenuModel, MenuPosition},
    window::MenuPlacement,
};

pub(super) type MenuActionHandler = Rc<dyn Fn(MenuActionId, &mut App)>;
pub(super) type DropdownSelectHandler = Rc<dyn Fn(DropdownItemId, &mut App)>;

const MENU_BOUNDS_HIT_SLOP: f32 = 2.0;

#[derive(Default)]
pub struct MenuService {
    generation: u64,
    windows: Vec<MenuWindowEntry>,
    opening_mouse_down_sequence: Option<u64>,
    hover_suppressed_since_mouse_move: Option<u64>,
}

impl Global for MenuService {}

#[derive(Clone, Copy)]
struct MenuWindowEntry {
    generation: u64,
    level: usize,
    handle: AnyWindowHandle,
    bounds: Option<MenuRect>,
    anchor_bounds: Option<MenuRect>,
}

impl MenuService {
    pub fn install(cx: &mut App) {
        cx.set_global(Self::default());
        cx.on_window_closed(|cx, _| {
            cx.defer(move |cx| {
                let menu_closed =
                    cx.update_global::<Self, _>(|service, cx| service.handle_window_closed(cx));
                sync_mouse_leave_state_after_close(menu_closed, cx);
            });
        })
        .detach();
    }

    pub fn open_context_menu(
        menu: MenuModel,
        anchor: MenuPosition,
        on_action: impl Fn(MenuActionId, &mut App) + 'static,
        cx: &mut App,
    ) {
        crate::windows::main::MainWindowService::cancel_hover_preview(cx);
        let anchor = window::physical_cursor_position()
            .map(|point| anchor.with_point(point))
            .unwrap_or(anchor);
        let opening_mouse_down_sequence = input::latest_mouse_down_sequence();
        let menu_closed = cx.update_global::<Self, _>(|service, cx| {
            service.generation = service.generation.wrapping_add(1);
            let generation = service.generation;
            let had_menu = service.close_all_windows(cx);
            if menu.is_empty() {
                return had_menu;
            }

            service.open_menu_level(
                generation,
                0,
                menu,
                MenuPlacement::Root { anchor },
                Rc::new(on_action),
                cx,
            );
            service.opening_mouse_down_sequence = opening_mouse_down_sequence;
            had_menu && service.windows.is_empty()
        });
        sync_mouse_leave_state_after_close(menu_closed, cx);
        crate::windows::main::MainWindowService::notify_main_view(cx);
    }

    pub fn open_dropdown_menu(
        menu: DropdownModel,
        anchor: DropdownAnchor,
        on_select: impl Fn(DropdownItemId, &mut App) + 'static,
        cx: &mut App,
    ) {
        crate::windows::preview::PreviewWindowService::hide(cx);
        crate::windows::main::MainWindowService::cancel_hover_preview(cx);
        let opening_mouse_down_sequence = input::latest_mouse_down_sequence();
        let menu_closed = cx.update_global::<Self, _>(|service, cx| {
            if service.should_toggle_dropdown_closed(anchor) {
                service.generation = service.generation.wrapping_add(1);
                return service.close_all_windows(cx);
            }

            service.generation = service.generation.wrapping_add(1);
            let generation = service.generation;
            let had_menu = service.close_all_windows(cx);
            if menu.is_empty() {
                return had_menu;
            }

            let Some(handle) =
                open_dropdown_window(generation, menu, anchor, Rc::new(on_select), cx)
            else {
                return had_menu;
            };

            service.windows.push(MenuWindowEntry {
                generation,
                level: 0,
                handle,
                bounds: None,
                anchor_bounds: Some(MenuRect::new(
                    anchor.x,
                    anchor.y,
                    anchor.width,
                    anchor.height,
                )),
            });
            service.opening_mouse_down_sequence = opening_mouse_down_sequence;
            false
        });
        sync_mouse_leave_state_after_close(menu_closed, cx);
        crate::windows::main::MainWindowService::notify_main_view(cx);
    }

    pub fn close_all(cx: &mut App) {
        let menu_closed = cx.update_global::<Self, _>(|service, cx| {
            service.generation = service.generation.wrapping_add(1);
            service.close_all_windows(cx)
        });
        sync_mouse_leave_state_after_close(menu_closed, cx);
    }

    pub(crate) fn has_open_menu(cx: &App) -> bool {
        cx.try_global::<Self>()
            .is_some_and(|service| !service.windows.is_empty())
    }

    pub(crate) fn hover_animation_allowed(cx: &mut App) -> bool {
        let Some((menu_open, suppressed_sequence)) = cx.try_global::<Self>().map(|service| {
            (
                !service.windows.is_empty(),
                service.hover_suppressed_since_mouse_move,
            )
        }) else {
            return true;
        };
        if menu_open {
            return false;
        }
        let Some(suppressed_sequence) = suppressed_sequence else {
            return true;
        };
        let Some(mouse_move_sequence) =
            input::pointer_snapshot().map(|snapshot| snapshot.mouse_move_sequence)
        else {
            return true;
        };

        if mouse_move_sequence <= suppressed_sequence {
            return false;
        }
        cx.update_global::<Self, _>(|service, _| {
            if service.hover_suppressed_since_mouse_move == Some(suppressed_sequence) {
                service.hover_suppressed_since_mouse_move = None;
            }
        });
        true
    }

    pub(crate) fn handle_global_mouse_down(
        sequence: u64,
        position: Point<f32>,
        cx: &mut App,
    ) -> bool {
        let (consumed, menu_closed) = cx.update_global::<Self, _>(|service, cx| {
            if service.windows.is_empty() {
                return (false, false);
            }

            // 菜单可能在打开它的鼠标按下事件处理期间创建，忽略这一次事件，避免菜单立即关闭。
            if service.opening_mouse_down_sequence == Some(sequence) {
                service.opening_mouse_down_sequence = None;
                return (true, false);
            }

            let menu_closed = if service.should_close_for_pointer(position) {
                service.generation = service.generation.wrapping_add(1);
                service.close_all_windows(cx)
            } else {
                false
            };

            // 菜单存在时优先消费鼠标按下事件，主窗口不处理同一次点击。
            (true, menu_closed)
        });
        sync_mouse_leave_state_after_close(menu_closed, cx);
        consumed
    }

    pub(super) fn open_submenu(request: SubmenuRequest, cx: &mut App) {
        let SubmenuRequest {
            generation,
            level,
            menu,
            parent_menu,
            parent_bounds,
            item_index,
            preferred_direction,
            scale_factor,
            scroll_offset_y,
            on_action,
        } = request;
        cx.update_global::<Self, _>(|service, cx| {
            if generation != service.generation {
                return;
            }
            service.close_from_level(level, cx);
            if menu.is_empty() {
                return;
            }

            service.open_menu_level(
                generation,
                level,
                menu,
                MenuPlacement::Submenu {
                    parent_menu,
                    parent_bounds,
                    item_index,
                    preferred_direction,
                    scale_factor,
                    scroll_offset_y,
                },
                on_action,
                cx,
            );
        });
    }

    pub(super) fn close_generation(cx: &mut App, generation: u64) {
        let menu_closed = cx.update_global::<Self, _>(|service, cx| {
            if generation != service.generation {
                return false;
            }
            service.generation = service.generation.wrapping_add(1);
            service.close_all_windows(cx)
        });
        sync_mouse_leave_state_after_close(menu_closed, cx);
    }

    pub(super) fn close_from_level(&mut self, level: usize, cx: &mut App) {
        if level >= self.windows.len() {
            return;
        }
        let handles = self
            .windows
            .drain(level..)
            .map(|entry| entry.handle)
            .collect::<Vec<_>>();
        close_menu_windows(handles, cx);
    }

    pub(super) fn set_window_bounds(&mut self, generation: u64, level: usize, bounds: MenuRect) {
        if let Some(entry) = self
            .windows
            .iter_mut()
            .find(|entry| entry.generation == generation && entry.level == level)
        {
            entry.bounds = Some(bounds);
        }
    }

    pub(super) fn is_generation_current(&self, generation: u64) -> bool {
        self.generation == generation
    }

    fn open_menu_level(
        &mut self,
        generation: u64,
        level: usize,
        menu: MenuModel,
        placement: MenuPlacement,
        on_action: MenuActionHandler,
        cx: &mut App,
    ) {
        let Some(handle) =
            super::window::open_context_window(generation, level, menu, placement, on_action, cx)
        else {
            return;
        };

        if self.windows.len() <= level {
            self.windows.push(MenuWindowEntry {
                generation,
                level,
                handle,
                bounds: None,
                anchor_bounds: None,
            });
        } else {
            self.windows[level] = MenuWindowEntry {
                generation,
                level,
                handle,
                bounds: None,
                anchor_bounds: None,
            };
        }
    }

    fn close_all_windows(&mut self, cx: &mut App) -> bool {
        let had_menu = !self.windows.is_empty();
        self.opening_mouse_down_sequence = None;
        let handles = self
            .windows
            .drain(..)
            .map(|entry| entry.handle)
            .collect::<Vec<_>>();
        close_menu_windows(handles, cx);
        had_menu
    }

    fn handle_window_closed(&mut self, cx: &mut App) -> bool {
        let open_windows = cx.windows();
        let Some(closed_level) = first_closed_menu_level(&self.windows, &open_windows) else {
            return false;
        };

        if closed_level == 0 {
            self.generation = self.generation.wrapping_add(1);
            self.close_all_windows(cx);
            true
        } else {
            self.close_from_level(closed_level, cx);
            false
        }
    }

    fn should_close_for_pointer(&self, position: gpui::Point<f32>) -> bool {
        let mut has_bounds = false;
        for entry in &self.windows {
            if let Some(bounds) = entry.bounds {
                has_bounds = true;
                if point_in_menu_rect(position, bounds) {
                    return false;
                }
            }
            if let Some(anchor) = entry.anchor_bounds {
                has_bounds = true;
                if point_in_menu_rect(position, anchor) {
                    return false;
                }
            }
        }
        has_bounds
    }

    fn should_toggle_dropdown_closed(&self, anchor: DropdownAnchor) -> bool {
        let current_anchor = self
            .windows
            .iter()
            .find(|entry| entry.generation == self.generation && entry.level == 0)
            .and_then(|entry| entry.anchor_bounds);

        should_toggle_dropdown_closed_for_anchor(current_anchor, anchor)
    }
}

fn sync_mouse_leave_state_after_close(menu_closed: bool, cx: &mut App) {
    if menu_closed {
        cx.update_global::<MenuService, _>(|service, _| {
            service.hover_suppressed_since_mouse_move =
                input::pointer_snapshot().map(|snapshot| snapshot.mouse_move_sequence);
        });
        crate::windows::main::sync_mouse_leave_state(cx);
        crate::windows::main::MainWindowService::notify_main_view(cx);
    }
}

fn point_in_menu_rect(position: gpui::Point<f32>, bounds: MenuRect) -> bool {
    position.x >= bounds.x - MENU_BOUNDS_HIT_SLOP
        && position.x <= bounds.right() + MENU_BOUNDS_HIT_SLOP
        && position.y >= bounds.y - MENU_BOUNDS_HIT_SLOP
        && position.y <= bounds.bottom() + MENU_BOUNDS_HIT_SLOP
}

fn dropdown_anchor_bounds(anchor: DropdownAnchor) -> MenuRect {
    MenuRect::new(anchor.x, anchor.y, anchor.width, anchor.height)
}

fn should_toggle_dropdown_closed_for_anchor(
    current_anchor: Option<MenuRect>,
    anchor: DropdownAnchor,
) -> bool {
    let Some(current_anchor) = current_anchor else {
        return false;
    };
    menu_rects_nearly_equal(current_anchor, dropdown_anchor_bounds(anchor))
}

fn menu_rects_nearly_equal(left: MenuRect, right: MenuRect) -> bool {
    const EPSILON: f32 = 1.0;

    (left.x - right.x).abs() <= EPSILON
        && (left.y - right.y).abs() <= EPSILON
        && (left.width - right.width).abs() <= EPSILON
        && (left.height - right.height).abs() <= EPSILON
}

fn first_closed_menu_level(
    entries: &[MenuWindowEntry],
    open_windows: &[AnyWindowHandle],
) -> Option<usize> {
    entries.iter().position(|entry| {
        !open_windows
            .iter()
            .any(|handle| handle.window_id() == entry.handle.window_id())
    })
}

pub(super) struct SubmenuRequest {
    pub(super) generation: u64,
    pub(super) level: usize,
    pub(super) menu: MenuModel,
    pub(super) parent_menu: MenuModel,
    pub(super) parent_bounds: MenuRect,
    pub(super) item_index: usize,
    pub(super) preferred_direction: MenuDirection,
    pub(super) scale_factor: f32,
    pub(super) scroll_offset_y: f32,
    pub(super) on_action: MenuActionHandler,
}

fn close_menu_windows(handles: Vec<AnyWindowHandle>, cx: &mut App) {
    for handle in handles {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}

pub(super) fn open_popup_window<V>(
    x: f32,
    y: f32,
    scale_factor: f32,
    cx: &mut App,
    build_root_view: impl FnOnce(
        &mut Window,
        &mut App,
        window::BorderlessPopup,
        Option<window::DisplayPlacement>,
    ) -> Entity<V>
    + 'static,
) -> Option<AnyWindowHandle>
where
    V: 'static + Render,
{
    let scale_factor = scale_factor.max(1.0);
    let display = window::display_for_physical_point(cx, point(x, y));
    let display_scale_factor = display
        .map(|display| display.scale_factor)
        .unwrap_or(scale_factor);
    let display_id = display.map(|display| display.display_id);
    let initial_bounds = Bounds::new(
        point(px(x / display_scale_factor), px(y / display_scale_factor)),
        size(px(1.0), px(1.0)),
    );
    let handle = crate::platform::window::open_borderless_popup(
        cx,
        initial_bounds,
        display_id,
        move |window, cx, popup| build_root_view(window, cx, popup, display),
    )
    .map_err(|error| tracing::error!("打开菜单窗口失败：{error}"))
    .ok()?;
    Some(handle.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_dropdown_trigger_should_toggle_closed() {
        let anchor = DropdownAnchor::new(40.0, 80.0, 156.0, 32.0, 1.25);
        let current_anchor = Some(dropdown_anchor_bounds(anchor));

        assert!(should_toggle_dropdown_closed_for_anchor(
            current_anchor,
            anchor,
        ));
    }

    #[test]
    fn different_dropdown_trigger_should_open_new_menu() {
        let current = DropdownAnchor::new(40.0, 80.0, 156.0, 32.0, 1.25);
        let other = DropdownAnchor::new(240.0, 80.0, 156.0, 32.0, 1.25);

        assert!(!should_toggle_dropdown_closed_for_anchor(
            Some(dropdown_anchor_bounds(current)),
            other,
        ));
    }

    #[test]
    fn closed_menu_should_not_toggle_dropdown() {
        let anchor = DropdownAnchor::new(40.0, 80.0, 156.0, 32.0, 1.25);

        assert!(!should_toggle_dropdown_closed_for_anchor(None, anchor,));
    }
}
