use gpui::{
    AnyElement, App, Context, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    Window, div,
};
use gpui_component::ActiveTheme as _;

use crate::{
    config::{ListItemHeight, TitlebarPosition},
    config_runtime,
};

use super::{
    MainWindowService,
    components::{
        clipboard_list, emoji_picker, gallery_picker, multi_selection_bar, symbol_picker, tab_bar,
        tags_panel, title_bar,
    },
    state::{AdaptiveHeightCacheKey, MainWindow},
    tabs::TabId,
    uses_sidebar_layout,
};

fn is_sidebar_layout(window: &Window) -> bool {
    uses_sidebar_layout(
        f32::from(window.viewport_size().width) / crate::ui::scale::window_scale(window),
    )
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let blocking_operation = MainWindowService::blocking_operation(cx);
        let is_blocked = blocking_operation.is_some();
        if is_blocked {
            self.cancel_hover_preview(cx);
            self.window_drag_start = None;
            self.floating_toolbar_drag = None;
            self.tags_panel_resize_start = None;
            self.heat_scrollbar.drag_origin = None;
            self.heat_scrollbar.dragging = false;
            self.focus_handle.focus(window, cx);
        }
        let appearance = config_runtime::appearance_config(cx);
        let layout = &appearance.tab_bar;
        let tabs = super::tabs::ordered_tabs(layout);
        if !layout.is_visible(self.active_tab.layout_button()) {
            let fallback = tabs.first().map(|tab| tab.id).unwrap_or(TabId::Clipboard);
            self.select_tab(fallback, cx);
        }
        if !layout.is_visible(self.symbol_category.layout_button()) {
            let categories = super::tabs::ordered_symbol_categories(layout);
            let fallback = categories
                .first()
                .map(|category| category.id)
                .unwrap_or(super::tabs::SymbolCategoryId::Emoji);
            self.select_symbol_category(fallback, cx);
        }
        let mut filters_changed = self.filters.retain_visible(layout);
        if is_blocked || !config_runtime::clipboard_config(cx).tab_bar_hover_switch {
            self.cancel_tab_bar_hover();
            filters_changed |= self.filters.clear_hovered();
        }
        if !window.is_window_hovered()
            || self
                .tab_bar_hover
                .pending_button()
                .is_some_and(|button| !layout.is_visible(button))
        {
            self.cancel_tab_bar_hover();
        }
        if !layout.is_visible(crate::config::TabBarButton::Tags) {
            self.tags_panel_open = false;
            self.tags_button_bounds = None;
            self.cancel_tags_panel_close();
            filters_changed |= !self.selected_tag_ids.is_empty();
            self.selected_tag_ids.clear();
        }
        if filters_changed {
            self.refresh_history(cx);
        }
        if !(self.active_tab == TabId::Emoji
            && self.symbol_category == super::tabs::SymbolCategoryId::Gallery)
        {
            self.clear_gallery_resources();
        }
        self.cleanup_gallery_image_atlas(window);
        self.advance_scroll_animation(
            window,
            !is_blocked
                && config_runtime::animation_enabled(
                    cx,
                    crate::config::AnimationKind::SmoothScroll,
                ),
        );
        self.cleanup_image_cache(window);
        self.cleanup_file_list_cache(window, cx);
        self.ensure_tags_search_input(window, cx);
        self.sync_tags_search_placeholder(window, cx);
        self.ensure_search_input(window, cx);
        self.sync_search_placeholder(window, cx);
        let theme = cx.theme().colors;
        let clipboard = config_runtime::clipboard_config(cx);
        self.prepare_adaptive_height_cache(
            AdaptiveHeightCacheKey {
                viewport_width: window.viewport_size().width,
                rem_size: window.rem_size(),
                font_metrics: crate::ui::fonts::AppFontMetrics::from_window(window, cx),
                list_item_style: appearance.list_item_style,
                card_item_gap: appearance.card_item_gap,
                adaptive_max_lines: appearance.adaptive_max_lines,
                only_show_notes: appearance.only_show_notes,
                show_info_row: appearance.item_display.shows_info_row(),
                display_priority: clipboard.display_priority,
                query_generation: self.query_generation,
            },
            appearance.list_item_height == ListItemHeight::Adaptive,
        );
        let titlebar_position = appearance.titlebar_position;
        let sidebar_layout = is_sidebar_layout(window);
        if !sidebar_layout || !self.tags_panel_open {
            self.finish_tags_panel_resize(cx);
        }
        if !sidebar_layout {
            self.sidebar_collapsed = false;
        }
        let content = if self.active_tab == TabId::Emoji {
            self.ensure_symbol_search_input(window, cx);
            self.sync_symbol_search_placeholder(window, cx);
            match self.symbol_category {
                super::tabs::SymbolCategoryId::Symbols => symbol_picker::render(self, window, cx),
                super::tabs::SymbolCategoryId::Gallery => gallery_picker::render(self, window, cx),
                super::tabs::SymbolCategoryId::Emoji => emoji_picker::render(self, window, cx),
            }
        } else {
            div()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .flex()
                .flex_col()
                .child(clipboard_list::render(
                    self.active_tab,
                    self.can_reorder_items(),
                    self.selected_item_id,
                    &self.multi_selected_item_ids,
                    &self.list_status,
                    &self.list_scroll_handle,
                    &mut self.list_heatmap,
                    &self.heat_scrollbar,
                    &mut self.adaptive_item_heights,
                    &mut self.list_layout_cache,
                    self.list_layout_generation,
                    self.query_generation,
                    self.total_item_count,
                    &self.list_order_ids,
                    &self.loaded_items,
                    &self.file_list_views,
                    self.floating_toolbar_bottom,
                    self.floating_toolbar_drag.is_some(),
                    window,
                    cx,
                ))
                .child(multi_selection_bar::render(
                    self.multi_selected_item_ids.len(),
                    self.active_tab,
                    cx,
                ))
                .into_any_element()
        };
        let titlebar = title_bar::render(self, window, cx, titlebar_position).into_any_element();
        let tab_bar = tab_bar::render(self, sidebar_layout, cx).into_any_element();
        let workspace = if sidebar_layout {
            div()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .flex()
                .flex_row()
                .child(tab_bar)
                .child(tags_panel::render(self, window, cx, true))
                .child(content)
        } else {
            div()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .flex()
                .flex_col()
                .child(tab_bar)
                .child(content)
        };

        let mut root = div()
            .size_full()
            .bg(theme.background)
            .flex()
            .relative()
            .track_focus(&self.focus_handle);
        if !is_blocked {
            root = root.child(super::window_drag::event_listener(clipboard.edge_hide, cx));
        }

        root = match titlebar_position {
            TitlebarPosition::Top => root.flex_col().child(titlebar).child(workspace),
            TitlebarPosition::Bottom => root.flex_col().child(workspace).child(titlebar),
            TitlebarPosition::Left => root.flex_row().child(titlebar).child(workspace),
            TitlebarPosition::Right => root.flex_row().child(workspace).child(titlebar),
        };

        if !sidebar_layout {
            root = root.child(tags_panel::render(self, window, cx, false));
        }

        if let Some(title_key) = blocking_operation {
            root = root
                .capture_any_mouse_down(|_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .capture_any_mouse_up(|_, _, cx| cx.stop_propagation())
                .capture_key_down(|_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .capture_key_up(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .id("main-blocking-operation-overlay")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                        .child(if self.gallery_import_progress.is_some() {
                            gallery_picker::render_gallery_import_overlay(self, cx)
                        } else {
                            render_blocking_operation_overlay(title_key, cx)
                        }),
                );
        }

        root
    }
}

fn render_blocking_operation_overlay(title_key: &str, cx: &App) -> AnyElement {
    use crate::ui::{icon, scale::ui_rem};

    let theme = cx.theme().colors;
    let translator = crate::i18n::translator(cx);
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::ColorExt::opacity(&theme.background, 0.58))
        .child(
            div()
                .w_full()
                .max_w(ui_rem(320.0))
                .mx(ui_rem(16.0))
                .p(ui_rem(20.0))
                .flex()
                .flex_col()
                .items_center()
                .gap(ui_rem(16.0))
                .rounded_xl()
                .bg(theme.background)
                .shadow_lg()
                .child(icon("refresh", 36.0, theme.primary))
                .child(
                    div()
                        .text_center()
                        .text_size(ui_rem(18.0))
                        .text_color(theme.foreground)
                        .child(translator.translate(title_key)),
                )
                .child(
                    div()
                        .text_center()
                        .text_size(ui_rem(13.0))
                        .text_color(theme.muted_foreground)
                        .child(translator.translate("settings.data_management.busy")),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::super::uses_sidebar_layout;

    #[test]
    fn sidebar_layout_uses_logical_width_boundary() {
        assert!(!uses_sidebar_layout(699.0));
        assert!(uses_sidebar_layout(700.0));
    }
}
