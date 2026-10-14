use crate::ui::scale::ui_rem;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Focusable, IntoElement, MouseButton, MouseDownEvent, Window, div, px,
};
use gpui_component::{ActiveTheme as _, Sizable, input::Input};

use crate::{ui::hover_icon, windows::main::state::MainWindow};

use super::symbol_sidebar::{TAB_BAR_HEIGHT, TAB_BUTTON_HEIGHT};

pub(crate) fn render(
    main: &MainWindow,
    window: &Window,
    cx: &mut Context<MainWindow>,
    trailing: Option<AnyElement>,
) -> AnyElement {
    let theme = cx.theme().colors;
    let search_input = main
        .symbol_search_input
        .as_ref()
        .expect("符号搜索输入框状态尚未初始化");
    let input_focused = search_input.focus_handle(cx).is_focused(window);
    let input_border = if input_focused {
        theme.ring
    } else {
        theme.input
    };
    let search_input_for_mouse = search_input.clone();
    let has_search_text = !search_input.read(cx).value().is_empty();
    let mut input = Input::new(search_input)
        .small()
        .appearance(false)
        .bordered(false)
        .focus_bordered(false)
        .cleanable(false)
        .w_full()
        .pl_0()
        .pr_0();
    if has_search_text {
        let input_for_clear = search_input.clone();
        input = input.suffix(
            div()
                .id("symbol-search-clear")
                .group("symbol-search-clear")
                .size(ui_rem(20.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .cursor_pointer()
                .hover(|this| this.bg(theme.secondary_hover))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.symbol_search_query.clear();
                    input_for_clear.update(cx, |input, cx| {
                        input.set_value("", window, cx);
                        input.focus(window, cx);
                    });
                    cx.notify();
                }))
                .child(hover_icon(
                    "symbol-search-clear",
                    "x",
                    14.0,
                    theme.muted_foreground,
                )),
        );
    }
    let mut root = div()
        .h(px(TAB_BAR_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .child(
            div()
                .h(px(TAB_BUTTON_HEIGHT))
                .group("symbol-search-input")
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap_1()
                .rounded_lg()
                .border_1()
                .border_color(input_border)
                .bg(theme.background)
                .capture_any_mouse_down(cx.listener(
                    move |this, event: &MouseDownEvent, window, cx| {
                        this.window_drag_start = None;
                        if event.button != MouseButton::Left || window.is_window_active() {
                            return;
                        }
                        if let Err(error) = crate::services::focus::focus_input_window(window) {
                            tracing::error!("点击符号搜索框时激活窗口失败：{error}");
                            return;
                        }
                        search_input_for_mouse.update(cx, |input, cx| input.focus(window, cx));
                    },
                ))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .size(ui_rem(20.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(hover_icon(
                            "symbol-search-input",
                            "search",
                            16.0,
                            theme.foreground,
                        )),
                )
                .child(input),
        );
    if let Some(trailing) = trailing {
        root = root.child(trailing);
    }
    root.into_any_element()
}
