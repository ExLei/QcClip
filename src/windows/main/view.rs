use gpui::{Context, InteractiveElement, IntoElement, ParentElement, Render, Styled, Window, div};
use gpui_component::ActiveTheme as _;

use crate::{
    config::{ListItemHeight, TitlebarPosition},
    config_runtime,
};

use super::{
    components::{
        clipboard_list, emoji_picker, gallery_picker, multi_selection_bar, symbol_picker,
        tab_bar, tags_panel, title_bar,
    },
    state::{AdaptiveHeightCacheKey, MainWindow},
    tabs::TabId,
    uses_sidebar_layout,
};

fn is_sidebar_layout(window: &Window) -> bool {
    uses_sidebar_layout(f32::from(window.viewport_size().width) / crate::ui::scale::window_scale(window))
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !(self.active_tab == TabId::Emoji
            && self.symbol_category == super::tabs::SymbolCategoryId::Gallery)
        {
            self.clear_gallery_resources();
        }
        self.cleanup_gallery_image_atlas(window);
        self.advance_scroll_animation(
            window,
            config_runtime::animation_enabled(cx, crate::config::AnimationKind::SmoothScroll),
        );
        self.cleanup_image_cache(window);
        self.cleanup_file_list_cache(window, cx);
        self.ensure_tags_search_input(window, cx);
        self.sync_tags_search_placeholder(window, cx);
        self.ensure_search_input(window, cx);
        self.sync_search_placeholder(window, cx);
        let theme = cx.theme().colors;
        let appearance = config_runtime::appearance_config(cx);
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
                display_priority: clipboard.display_priority,
                query_generation: self.query_generation,
            },
            appearance.list_item_height == ListItemHeight::Adaptive,
        );
        let titlebar_position = appearance.titlebar_position;
        let sidebar_layout = is_sidebar_layout(window);
        if !sidebar_layout {
            self.sidebar_collapsed = false;
        }
        let content = if self.active_tab == TabId::Emoji {
            self.ensure_symbol_search_input(window, cx);
            self.sync_symbol_search_placeholder(window, cx);
            match self.symbol_category {
                super::tabs::SymbolCategoryId::Symbols => symbol_picker::render(self, window, cx),
                super::tabs::SymbolCategoryId::Gallery => {
                    gallery_picker::render(self, window, cx)
                }
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
        root = root.child(super::window_drag::event_listener(clipboard.edge_hide, cx));

        root = match titlebar_position {
            TitlebarPosition::Top => root.flex_col().child(titlebar).child(workspace),
            TitlebarPosition::Bottom => root.flex_col().child(workspace).child(titlebar),
            TitlebarPosition::Left => root.flex_row().child(titlebar).child(workspace),
            TitlebarPosition::Right => root.flex_row().child(workspace).child(titlebar),
        };

        if !sidebar_layout {
            root = root.child(tags_panel::render(self, window, cx, false));
        }

        if self.gallery_import_progress.is_some() {
            root = root.child(gallery_picker::render_gallery_import_overlay(self, cx));
        }

        root
    }
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
