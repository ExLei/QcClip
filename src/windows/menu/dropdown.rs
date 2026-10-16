use std::cell::Cell;

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyWindowHandle, App, AvailableSpace, Context, IntoElement, MouseButton, Pixels,
    Render, ScrollHandle, Window, div, px, size,
};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{ActiveTheme as _, ThemeColor};

use crate::platform::window as platform_window;

use super::{
    animation::{MenuEntranceDirection, MenuPopupEntrance, animated_content_bounds},
    layout::{MENU_SCREEN_MARGIN, MenuRect, fit_rect_to_screen, max_menu_height},
    model::{DropdownAnchor, DropdownItem, DropdownModel},
    service::{DropdownSelectHandler, MenuService, open_popup_window},
    window::menu_surface,
};

const DROPDOWN_ITEM_HEIGHT: f32 = 30.0;
const DROPDOWN_SEPARATOR_HEIGHT: f32 = 9.0;
const DROPDOWN_ITEM_PADDING_X: f32 = 5.0;
const DROPDOWN_ITEM_GAP_Y: f32 = DROPDOWN_ITEM_PADDING_X;
const DROPDOWN_ITEM_PADDING_Y: f32 = DROPDOWN_ITEM_GAP_Y / 2.0;
const DROPDOWN_SHELL_PADDING_Y: f32 = DROPDOWN_ITEM_PADDING_Y;
const DROPDOWN_GAP: f32 = 2.0;
const DROPDOWN_MAX_HEIGHT: f32 = 240.0;

#[derive(Clone, Copy)]
struct DropdownPlacementResult {
    content_bounds: MenuRect,
    initial_bounds: MenuRect,
}

pub(super) struct DropdownMenuView {
    generation: u64,
    menu: DropdownModel,
    anchor: DropdownAnchor,
    entrance: MenuPopupEntrance,
    rendered_content_width: Cell<Option<f32>>,
    scroll_handle: ScrollHandle,
    initial_scroll_index: Option<usize>,
    on_select: DropdownSelectHandler,
    popup: platform_window::BorderlessPopup,
    target_display: Option<platform_window::DisplayPlacement>,
}

impl DropdownMenuView {
    pub(super) fn new(
        generation: u64,
        menu: DropdownModel,
        anchor: DropdownAnchor,
        on_select: DropdownSelectHandler,
        popup: platform_window::BorderlessPopup,
        target_display: Option<platform_window::DisplayPlacement>,
        _cx: &mut Context<Self>,
    ) -> Self {
        let initial_scroll_index = menu.selected_index();
        Self {
            generation,
            menu,
            anchor,
            entrance: MenuPopupEntrance::default(),
            rendered_content_width: Cell::new(None),
            scroll_handle: ScrollHandle::new(),
            initial_scroll_index,
            on_select,
            popup,
            target_display,
        }
    }

    fn display_scale_factor(&self, window: &Window) -> f32 {
        self.target_display
            .map(|display| display.scale_factor)
            .unwrap_or_else(|| self.anchor.scale_factor.max(window.scale_factor()).max(1.0))
    }

    fn click_item(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.menu.items().get(index) else {
            return;
        };
        if !item.enabled {
            return;
        }

        let id = item.id.clone();
        let generation = self.generation;
        let on_select = self.on_select.clone();
        window.prevent_default();
        window.remove_window();
        cx.defer(move |cx| {
            MenuService::close_generation(cx, generation);
            on_select(id, cx);
        });
    }

    fn measure_content_size(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Size<Pixels> {
        let scale_factor = self.display_scale_factor(window);
        let max_height = logical_dropdown_max_height(window, cx, scale_factor, self.target_display);
        let measured_size = dropdown_shell(
            self.render_measurement_items(),
            cx.theme().colors,
            max_height,
            &self.scroll_handle,
            1.0,
        )
        .when_some(self.menu.font_family.clone(), |this, family| {
            this.font_family(family)
        })
        .into_any_element()
        .layout_as_root(
            size(AvailableSpace::MinContent, AvailableSpace::MinContent),
            window,
            cx,
        );
        let width = menu_width_for_trigger(
            f32::from(measured_size.width),
            self.anchor.width / scale_factor,
        );
        size(px(width), measured_size.height)
    }

    fn place_window(
        &self,
        measured_size: gpui::Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> DropdownPlacementResult {
        let scale_factor = self.display_scale_factor(window);
        let screen = screen_bounds(window, cx, scale_factor, self.target_display);
        let size = (
            (f32::from(measured_size.width) * scale_factor).ceil(),
            (f32::from(measured_size.height) * scale_factor)
                .ceil()
                .min(max_menu_height(screen)),
        );
        let content_bounds = fit_dropdown_bounds(self.anchor, size, screen, scale_factor);
        let entrance_direction = if dropdown_expands_below(self.anchor, size, screen, scale_factor)
        {
            MenuEntranceDirection::Down
        } else {
            MenuEntranceDirection::Up
        };
        let placement = DropdownPlacementResult {
            content_bounds,
            initial_bounds: animated_dropdown_bounds(
                content_bounds,
                self.anchor,
                entrance_direction,
                0.0,
            ),
        };

        // content_bounds 使用物理像素，渲染树使用逻辑像素。使用最终取整后的物理宽度
        // 反推渲染宽度，避免高 DPI 下窗口宽度和下拉面板宽度出现像素级分叉。
        self.rendered_content_width
            .set(Some(content_bounds.width / scale_factor));
        self.entrance.update_window(
            self.popup,
            self.generation,
            0,
            content_bounds,
            placement.initial_bounds,
            window,
            cx,
        );
        placement
    }

    fn render_items(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.menu
            .items()
            .iter()
            .enumerate()
            .map(|(index, item)| self.render_item(index, item, cx))
            .collect()
    }

    fn render_measurement_items(&self) -> Vec<AnyElement> {
        self.menu
            .items()
            .iter()
            .map(dropdown_measurement_item)
            .collect()
    }

    fn render_item(&self, index: usize, item: &DropdownItem, cx: &mut Context<Self>) -> AnyElement {
        if item.separator {
            return div()
                .h(gpui::rems(DROPDOWN_SEPARATOR_HEIGHT / 13.0))
                .px(gpui::rems(3.0 / 13.0))
                .flex()
                .items_center()
                .child(
                    div()
                        .h(gpui::rems(1.0 / 13.0))
                        .w_full()
                        .bg(cx.theme().border),
                )
                .into_any_element();
        }

        let enabled = item.enabled;
        let selected = item.selected;
        let label = item.label.clone();
        let colors = cx.theme().colors;
        let foreground = if enabled {
            colors.popover_foreground
        } else {
            colors.muted_foreground
        };

        div()
            .id(("qcclip-dropdown-item", index as u64))
            .w_full()
            .h(gpui::rems(DROPDOWN_ITEM_HEIGHT / 13.0))
            .flex()
            .min_w_0()
            .overflow_hidden()
            .px(gpui::rems(DROPDOWN_ITEM_PADDING_X / 13.0))
            .py(gpui::rems(DROPDOWN_ITEM_PADDING_Y / 13.0))
            .child(
                div()
                    .id(("qcclip-dropdown-item-row", index as u64))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .rounded(px(4.0))
                    .px(gpui::rems(9.0 / 13.0))
                    .text_color(foreground)
                    .when(selected, |this| this.bg(colors.accent))
                    .when(enabled, |this| this.hover(|style| style.bg(colors.accent)))
                    .when(!enabled, |this| this.opacity(0.5))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.click_item(index, window, cx);
                        }),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(label)),
            )
            .into_any_element()
    }
}

impl Render for DropdownMenuView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(crate::ui::scale::standalone_rem_size(13.0, cx));
        let measured_size = self.measure_content_size(window, cx);
        let placement = self.place_window(measured_size, window, cx);
        if let Some(index) = self.initial_scroll_index.take() {
            let scroll_handle = self.scroll_handle.clone();
            // 首帧预绘制后滚动容器才有有效边界；每次打开只定位一次。
            window.on_next_frame(move |window, _| {
                scroll_handle.scroll_to_item(index);
                window.refresh();
            });
        }
        let scale_factor = self.display_scale_factor(window);
        let max_height = logical_dropdown_max_height(window, cx, scale_factor, self.target_display);
        let content_bounds = placement.content_bounds;
        let width = self
            .rendered_content_width
            .get()
            .unwrap_or_else(|| f32::from(measured_size.width));
        let height = content_bounds.height / scale_factor;
        let window_bounds = self.entrance.current_bounds().unwrap_or(content_bounds);
        let left = (content_bounds.x - window_bounds.x) / scale_factor;
        let top = (content_bounds.y - window_bounds.y) / scale_factor;
        let content = div()
            .id("qcclip-dropdown-window")
            .size_full()
            .relative()
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(width))
                    .h(px(height))
                    .child(
                        dropdown_shell(
                            self.render_items(cx),
                            cx.theme().colors,
                            max_height,
                            &self.scroll_handle,
                            width,
                        )
                        .when_some(self.menu.font_family.clone(), |this, family| {
                            this.font_family(family)
                        })
                        .w(px(width))
                        .h(px(height)),
                    ),
            );
        crate::window_material::with_background(content, window, cx)
    }
}

fn animated_dropdown_bounds(
    target_bounds: MenuRect,
    anchor: DropdownAnchor,
    direction: MenuEntranceDirection,
    progress: f32,
) -> MenuRect {
    let progress = progress.clamp(0.0, 1.0);
    let animated = animated_content_bounds(target_bounds, direction, progress);
    let trigger_width = anchor.width.min(target_bounds.width);
    let width = trigger_width + (target_bounds.width - trigger_width) * progress;
    MenuRect::new(target_bounds.x, animated.y, width, animated.height)
}

fn dropdown_shell(
    items: Vec<AnyElement>,
    colors: ThemeColor,
    max_height: f32,
    scroll_handle: &ScrollHandle,
    min_width: f32,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id("qcclip-dropdown-shell")
        .relative()
        .child(
            menu_surface(colors)
                .id("qcclip-dropdown-surface")
                .min_w(px(min_width))
                .max_h(px(max_height))
                .py(gpui::rems(DROPDOWN_SHELL_PADDING_Y / 13.0))
                .text_size(gpui::rems(13.0 / 13.0))
                .overflow_y_scroll()
                .track_scroll(scroll_handle)
                .children(items),
        )
        .vertical_scrollbar(scroll_handle)
}

fn dropdown_measurement_item(item: &DropdownItem) -> AnyElement {
    if item.separator {
        return div()
            .h(gpui::rems(DROPDOWN_SEPARATOR_HEIGHT / 13.0))
            .px(gpui::rems(3.0 / 13.0))
            .flex()
            .items_center()
            .child(div().h(gpui::rems(1.0 / 13.0)).w_full())
            .into_any_element();
    }

    div()
        .h(gpui::rems(DROPDOWN_ITEM_HEIGHT / 13.0))
        .px(gpui::rems(DROPDOWN_ITEM_PADDING_X / 13.0))
        .py(gpui::rems(DROPDOWN_ITEM_PADDING_Y / 13.0))
        .child(
            div()
                .h_full()
                .flex()
                .items_center()
                .px(gpui::rems(9.0 / 13.0))
                .child(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .child(item.label.clone()),
                ),
        )
        .into_any_element()
}

fn menu_width_for_trigger(content_width: f32, trigger_width: f32) -> f32 {
    content_width.max(trigger_width).max(1.0)
}

fn fit_dropdown_bounds(
    anchor: DropdownAnchor,
    size: (f32, f32),
    screen: MenuRect,
    scale_factor: f32,
) -> MenuRect {
    let gap = DROPDOWN_GAP * scale_factor;
    let below_y = anchor.y + anchor.height + gap;
    let above_y = anchor.y - gap - size.1;
    let y = if dropdown_expands_below(anchor, size, screen, scale_factor) {
        below_y
    } else {
        above_y
    };

    fit_rect_to_screen(
        MenuRect::new(anchor.x, y, size.0.max(anchor.width), size.1),
        screen,
    )
}

fn dropdown_expands_below(
    anchor: DropdownAnchor,
    size: (f32, f32),
    screen: MenuRect,
    scale_factor: f32,
) -> bool {
    let gap = DROPDOWN_GAP * scale_factor;
    let below_y = anchor.y + anchor.height + gap;
    let above_y = anchor.y - gap - size.1;
    let margin = MENU_SCREEN_MARGIN * scale_factor;
    let available_below = screen.bottom() - margin - below_y;
    let available_above = above_y + size.1 - (screen.y + margin);
    size.1 <= available_below || available_below >= available_above
}

fn screen_bounds(
    window: &Window,
    cx: &Context<DropdownMenuView>,
    scale_factor: f32,
    target_display: Option<platform_window::DisplayPlacement>,
) -> MenuRect {
    target_display
        .map(|display| {
            let bounds = display.bounds;
            MenuRect::new(bounds.x, bounds.y, bounds.width, bounds.height)
        })
        .or_else(|| {
            window
                .display(cx)
                .or_else(|| cx.primary_display())
                .map(|display| {
                    let bounds = display.bounds();
                    MenuRect::new(
                        f32::from(bounds.origin.x) * scale_factor,
                        f32::from(bounds.origin.y) * scale_factor,
                        f32::from(bounds.size.width) * scale_factor,
                        f32::from(bounds.size.height) * scale_factor,
                    )
                })
        })
        .unwrap_or_else(|| MenuRect::new(0.0, 0.0, 1920.0, 1080.0))
}

fn logical_dropdown_max_height(
    window: &Window,
    cx: &Context<DropdownMenuView>,
    scale_factor: f32,
    target_display: Option<platform_window::DisplayPlacement>,
) -> f32 {
    let screen = screen_bounds(window, cx, scale_factor, target_display);
    (max_menu_height(screen) / scale_factor)
        .min(DROPDOWN_MAX_HEIGHT)
        .max(DROPDOWN_ITEM_HEIGHT)
}

pub(super) fn open_dropdown_window(
    generation: u64,
    menu: DropdownModel,
    anchor: DropdownAnchor,
    on_select: DropdownSelectHandler,
    cx: &mut App,
) -> Option<AnyWindowHandle> {
    open_popup_window(
        anchor.x,
        anchor.y,
        anchor.scale_factor,
        cx,
        move |_, cx, popup, target_display| {
            cx.new(|cx| {
                DropdownMenuView::new(
                    generation,
                    menu,
                    anchor,
                    on_select,
                    popup,
                    target_display,
                    cx,
                )
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropdown_menu_width_uses_the_larger_content_or_trigger_width() {
        assert_eq!(menu_width_for_trigger(120.0, 156.0), 156.0);
        assert_eq!(menu_width_for_trigger(220.0, 156.0), 220.0);
    }

    #[test]
    fn dropdown_menu_is_aligned_to_trigger_and_opens_below_it() {
        let bounds = fit_dropdown_bounds(
            DropdownAnchor::new(40.0, 40.0, 156.0, 32.0, 1.0),
            (156.0, 80.0),
            MenuRect::new(0.0, 0.0, 600.0, 400.0),
            1.0,
        );

        assert_eq!(bounds.x, 40.0);
        assert_eq!(bounds.y, 74.0);
        assert_eq!(bounds.width, 156.0);
    }

    #[test]
    fn dropdown_menu_opens_above_when_below_space_is_insufficient() {
        let bounds = fit_dropdown_bounds(
            DropdownAnchor::new(40.0, 340.0, 156.0, 32.0, 1.0),
            (156.0, 100.0),
            MenuRect::new(0.0, 0.0, 600.0, 400.0),
            1.0,
        );

        assert_eq!(bounds.y, 238.0);
    }
}
