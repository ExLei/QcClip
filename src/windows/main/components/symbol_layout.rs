use gpui::prelude::*;
use gpui::{AnyElement, Context, IntoElement, Window, div, px};

use crate::windows::main::{state::MainWindow, uses_sidebar_layout};

use super::symbol_sidebar::TAB_BAR_HEIGHT;

pub(crate) fn render(
    sidebar: AnyElement,
    search_box: AnyElement,
    content: AnyElement,
    horizontal_categories: bool,
    _cx: &mut Context<MainWindow>,
) -> AnyElement {
    if horizontal_categories {
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .w_full()
                    .h(px(TAB_BAR_HEIGHT))
                    .flex_none()
                    .flex()
                    .items_center()
                    .child(div().flex_1().min_w_0().h_full().child(search_box))
                    .child(div().flex_1().min_w_0().h_full().child(sidebar)),
            )
            .child(content)
            .into_any_element()
    } else {
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .child(sidebar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(search_box)
                    .child(content),
            )
            .into_any_element()
    }
}

pub(crate) fn grid_columns(
    main: &MainWindow,
    window: &Window,
    cell_min_width: f32,
    horizontal_categories: bool,
) -> u16 {
    let viewport_width =
        f32::from(window.viewport_size().width) / crate::ui::scale::window_scale(window);
    let main_sidebar_width = if !horizontal_categories && uses_sidebar_layout(viewport_width) {
        if main.sidebar_collapsed { 56.0 } else { 132.0 }
    } else {
        0.0
    };
    let category_sidebar_width = if horizontal_categories {
        0.0
    } else {
        TAB_BAR_HEIGHT
    };
    let grid_padding = 4.0;
    let grid_gap = 2.0;
    let available_width =
        (viewport_width - main_sidebar_width - category_sidebar_width - grid_padding * 2.0)
            .max(cell_min_width);
    ((available_width + grid_gap) / (cell_min_width + grid_gap))
        .floor()
        .clamp(1.0, 12.0) as u16
}
