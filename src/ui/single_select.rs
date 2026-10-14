use crate::ui::scale::ui_rem;

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gpui::prelude::*;
use gpui::{
    AnimationExt as _, AnyElement, App, ElementId, IntoElement, Pixels, SharedString, div, px,
    relative,
};
use gpui_component::ThemeColor;

thread_local! {
    static SINGLE_SELECT_MEASUREMENTS: RefCell<HashMap<String, Vec<(f32, f32)>>> =
        RefCell::new(HashMap::new());
}

#[derive(Clone)]
pub struct SingleSelectOption<T> {
    pub value: T,
    pub label: SharedString,
}

impl<T> SingleSelectOption<T> {
    pub fn new(value: T, label: impl Into<SharedString>) -> Self {
        Self {
            value,
            label: label.into(),
        }
    }
}

pub fn single_select<T, I, F>(
    id: impl Into<ElementId>,
    options: I,
    selected: T,
    on_select: F,
    theme: ThemeColor,
    animations_enabled: bool,
) -> AnyElement
where
    T: Clone + PartialEq + 'static,
    I: IntoIterator<Item = SingleSelectOption<T>>,
    F: Fn(T, &mut App) + 'static,
{
    single_select_with_layout(
        id,
        options,
        selected,
        on_select,
        SingleSelectLayout::Row,
        theme,
        animations_enabled,
    )
}

pub fn single_select_grid<T, I, F>(
    id: impl Into<ElementId>,
    options: I,
    selected: T,
    columns: usize,
    on_select: F,
    theme: ThemeColor,
    animations_enabled: bool,
) -> AnyElement
where
    T: Clone + PartialEq + 'static,
    I: IntoIterator<Item = SingleSelectOption<T>>,
    F: Fn(T, &mut App) + 'static,
{
    single_select_with_layout(
        id,
        options,
        selected,
        on_select,
        SingleSelectLayout::Grid { columns },
        theme,
        animations_enabled,
    )
}

#[derive(Clone, Copy)]
enum SingleSelectLayout {
    Row,
    Grid { columns: usize },
}

fn single_select_with_layout<T, I, F>(
    id: impl Into<ElementId>,
    options: I,
    selected: T,
    on_select: F,
    layout: SingleSelectLayout,
    theme: ThemeColor,
    animations_enabled: bool,
) -> AnyElement
where
    T: Clone + PartialEq + 'static,
    I: IntoIterator<Item = SingleSelectOption<T>>,
    F: Fn(T, &mut App) + 'static,
{
    let on_select: Rc<dyn Fn(T, &mut App)> = Rc::new(on_select);
    let id = id.into();
    let options = options.into_iter().collect::<Vec<_>>();
    let option_count = options.len();
    let selected_index = options
        .iter()
        .position(|option| option.value == selected)
        .unwrap_or(0);
    let columns = match layout {
        SingleSelectLayout::Row => option_count.max(1),
        SingleSelectLayout::Grid { columns } => columns.max(1),
    };
    let row_count = match layout {
        SingleSelectLayout::Row => 1,
        SingleSelectLayout::Grid { .. } => option_count
            .saturating_add(columns - 1)
            .checked_div(columns)
            .unwrap_or(0)
            .max(1),
    };
    let selector_key = id.to_string();
    let measured_indicator = SINGLE_SELECT_MEASUREMENTS.with(|measurements| {
        measurements
            .borrow()
            .get(&selector_key)
            .and_then(|bounds| bounds_for_indicator(bounds, selected_index))
    });
    let mut selector = div()
        .h(ui_rem(32.0 * row_count as f32))
        .flex()
        .flex_shrink_0()
        .overflow_hidden()
        .rounded(px(7.0))
        .border_1()
        .border_color(theme.input)
        .bg(theme.background)
        .relative()
        .child(single_select_indicator(
            (id.clone(), "indicator"),
            selected_index,
            option_count,
            row_count,
            layout,
            theme,
            animations_enabled,
            measured_indicator,
        ));

    match layout {
        SingleSelectLayout::Row => {
            selector = selector.w_auto().min_w_auto().flex_none();
            let selector_key = selector_key.clone();
            selector = selector.on_children_prepainted(move |bounds, _, cx| {
                let options = if bounds.len() == option_count + 1 {
                    bounds.into_iter().skip(1).collect::<Vec<_>>()
                } else {
                    bounds
                };
                if options.is_empty() {
                    return;
                }
                let first = options[0].origin.x;
                let measurements = options
                    .into_iter()
                    .map(|bounds| {
                        (
                            f32::from(bounds.origin.x - first),
                            f32::from(bounds.size.width),
                        )
                    })
                    .collect::<Vec<_>>();
                let changed = SINGLE_SELECT_MEASUREMENTS.with(|measurements_cache| {
                    let mut measurements_cache = measurements_cache.borrow_mut();
                    if measurements_cache.get(&selector_key) == Some(&measurements) {
                        false
                    } else {
                        measurements_cache.insert(selector_key.clone(), measurements);
                        true
                    }
                });
                if changed {
                    cx.refresh_windows();
                }
            });
            for (index, option) in options.into_iter().enumerate() {
                selector = selector.child(single_select_option(
                    option,
                    index,
                    0,
                    index,
                    option_count,
                    1,
                    layout,
                    &selected,
                    on_select.clone(),
                    theme,
                ));
            }
        }
        SingleSelectLayout::Grid { .. } => {
            selector = selector.w_auto().min_w_auto().flex_none().flex_col();
            let mut options = options.into_iter().enumerate();
            for row_index in 0..row_count {
                let row_start = row_index * columns;
                let row_len = (option_count.saturating_sub(row_start)).min(columns);
                let mut row = div().h(ui_rem(32.0)).flex();
                for column_index in 0..row_len {
                    let Some((index, option)) = options.next() else {
                        break;
                    };
                    row = row.child(single_select_option(
                        option,
                        index,
                        row_index,
                        column_index,
                        row_len,
                        row_count,
                        layout,
                        &selected,
                        on_select.clone(),
                        theme,
                    ));
                }
                selector = selector.child(row);
            }
        }
    }

    selector.id(id).into_any_element()
}

fn single_select_option<T>(
    option: SingleSelectOption<T>,
    index: usize,
    row_index: usize,
    column_index: usize,
    row_len: usize,
    row_count: usize,
    layout: SingleSelectLayout,
    selected: &T,
    on_select: Rc<dyn Fn(T, &mut App)>,
    theme: ThemeColor,
) -> AnyElement
where
    T: Clone + PartialEq + 'static,
{
    let is_selected = option.value == *selected;
    let value = option.value;
    let mut button = div()
        .id(("single-select-option", index as u64))
        .h_full()
        .min_w_0()
        .flex()
        .items_center()
        .justify_center()
        .px_3()
        .text_sm()
        .whitespace_nowrap()
        .cursor_pointer()
        .text_color(theme.foreground)
        .child(option.label)
        .on_click(move |_, _, cx| on_select(value.clone(), cx));

    match layout {
        SingleSelectLayout::Row => {
            button = button
                .min_w_0()
                .truncate()
                .w_auto()
                .max_w_full()
                .flex_none();
            if index == 0 {
                button = button.rounded_l(px(6.0));
            }
            if index + 1 == row_len {
                button = button.rounded_r(px(6.0));
            }
        }
        SingleSelectLayout::Grid { .. } => {
            button = button.min_w_auto().flex_1();
            let is_first_row = row_index == 0;
            let is_last_row = row_index + 1 == row_count;
            let is_first_column = column_index == 0;
            let is_last_column = column_index + 1 == row_len;
            if is_first_row && is_first_column {
                button = button.rounded_tl(px(6.0));
            }
            if is_first_row && is_last_column {
                button = button.rounded_tr(px(6.0));
            }
            if is_last_row && is_first_column {
                button = button.rounded_bl(px(6.0));
            }
            if is_last_row && is_last_column {
                button = button.rounded_br(px(6.0));
            }
            if !is_last_column {
                button = button.border_r_1().border_color(theme.input);
            }
            if !is_last_row {
                button = button.border_b_1().border_color(theme.input);
            }
        }
    }

    if is_selected {
        button = button
            .text_color(theme.primary_foreground)
            .active(|style| style.bg(theme.primary_active));
    } else {
        button = button
            .hover(|style| style.bg(theme.accent))
            .active(|style| style.bg(theme.secondary_active));
    }
    button.into_any_element()
}

fn single_select_indicator(
    id: impl Into<ElementId>,
    selected_index: usize,
    option_count: usize,
    row_count: usize,
    layout: SingleSelectLayout,
    theme: ThemeColor,
    animations_enabled: bool,
    measured: Option<(Pixels, Pixels)>,
) -> AnyElement {
    if let Some((left, width)) = measured {
        let id = id.into();
        let bar = div()
            .id((id.clone(), "bar"))
            .absolute()
            .left(left)
            .top_0()
            .w(width)
            .h_full()
            .rounded(px(6.0))
            .bg(theme.primary);
        return if animations_enabled {
            bar.with_spring((id, "left"), spring(left), |this, value| this.left(value))
                .into_any_element()
        } else {
            bar.into_any_element()
        };
    }
    let id = id.into();
    let (left, top, width, height) = match layout {
        SingleSelectLayout::Row => {
            let count = option_count.max(1) as f32;
            (selected_index as f32 / count, 0.0, 1.0 / count, 1.0)
        }
        SingleSelectLayout::Grid { columns } => {
            let columns = columns.max(1);
            let row = selected_index / columns;
            let column = selected_index % columns;
            (
                column as f32 / columns as f32,
                row as f32 / row_count.max(1) as f32,
                1.0 / columns as f32,
                1.0 / row_count.max(1) as f32,
            )
        }
    };
    let bar = div()
        .id((id.clone(), "bar"))
        .absolute()
        .left(relative(left))
        .top_0()
        .w(relative(width))
        .h_full()
        .rounded(px(6.0))
        .bg(theme.primary);
    let bar = if animations_enabled {
        bar.with_spring((id.clone(), "left"), spring(left), |this, value| {
            this.left(relative(value))
        })
        .into_any_element()
    } else {
        bar.into_any_element()
    };
    let indicator = div()
        .id(id.clone())
        .absolute()
        .left_0()
        .top(relative(top))
        .w_full()
        .h(relative(height))
        .child(bar);
    if animations_enabled {
        indicator
            .with_spring((id, "top"), spring(top), |this, value| {
                this.top(relative(value))
            })
            .into_any_element()
    } else {
        indicator.into_any_element()
    }
}

fn bounds_for_indicator(
    bounds: &[(f32, f32)],
    selected_index: usize,
) -> Option<(Pixels, Pixels)> {
    let (left, width) = *bounds.get(selected_index)?;
    Some((px(left), px(width)))
}

fn spring<T: gpui::SpringTarget>(target: T) -> gpui::SpringAnimation<T> {
    gpui::SpringAnimation::new(gpui::SpringConfig::new(420.0, 36.0, 1.0)).to(target)
}
