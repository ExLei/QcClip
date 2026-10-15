#![allow(dead_code, unused_imports)]

#[path = "../src/config/mod.rs"]
mod config;
#[path = "../src/windows/main/tabs.rs"]
mod tabs;

use config::{TabBarButton, TabBarLayout};
use tabs::{FilterId, SymbolCategoryId, TabId};

#[test]
fn main_window_uses_the_configured_order_and_visibility_for_each_group() {
    let mut layout = TabBarLayout::default();
    layout.move_button(TabBarButton::Emoji, TabBarButton::Clipboard);
    layout.move_button(TabBarButton::Pasted, TabBarButton::Text);
    layout.move_button(TabBarButton::Gallery, TabBarButton::SymbolEmoji);
    layout.toggle(TabBarButton::Favorites);
    layout.toggle(TabBarButton::Image);
    layout.toggle(TabBarButton::Symbols);
    assert_eq!(
        tabs::ordered_tabs(&layout)
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>(),
        vec![TabId::Emoji, TabId::Clipboard]
    );
    assert_eq!(
        tabs::ordered_filters(&layout)
            .iter()
            .map(|filter| filter.id)
            .collect::<Vec<_>>(),
        vec![
            FilterId::Pasted,
            FilterId::Text,
            FilterId::File,
            FilterId::Link,
            FilterId::Unpasted
        ]
    );
    assert_eq!(
        tabs::ordered_symbol_categories(&layout)
            .iter()
            .map(|category| category.id)
            .collect::<Vec<_>>(),
        vec![SymbolCategoryId::Gallery, SymbolCategoryId::Emoji]
    );
}

#[test]
fn fully_hidden_groups_have_no_main_window_buttons() {
    let mut layout = TabBarLayout::default();
    for button in TabBarButton::ALL {
        layout.toggle(button);
    }
    assert!(tabs::ordered_tabs(&layout).is_empty());
    assert!(tabs::ordered_filters(&layout).is_empty());
    assert!(tabs::ordered_symbol_categories(&layout).is_empty());
}

#[test]
fn disabling_all_symbol_categories_also_disables_the_page_entry() {
    let mut layout = TabBarLayout::default();
    for button in [
        TabBarButton::SymbolEmoji,
        TabBarButton::Symbols,
        TabBarButton::Gallery,
    ] {
        layout.toggle(button);
    }
    assert!(!layout.is_visible(TabBarButton::Emoji));
    assert!(
        !tabs::ordered_tabs(&layout)
            .iter()
            .any(|tab| tab.id == TabId::Emoji)
    );

    layout.toggle(TabBarButton::Gallery);
    assert!(layout.is_visible(TabBarButton::Emoji));
    assert!(
        tabs::ordered_tabs(&layout)
            .iter()
            .any(|tab| tab.id == TabId::Emoji)
    );
}

#[test]
fn manually_hidden_page_stays_hidden_when_a_category_is_enabled() {
    let mut layout = TabBarLayout::default();
    layout.toggle(TabBarButton::Emoji);
    for button in [
        TabBarButton::SymbolEmoji,
        TabBarButton::Symbols,
        TabBarButton::Gallery,
    ] {
        layout.toggle(button);
    }
    layout.toggle(TabBarButton::Gallery);
    assert!(!layout.is_visible(TabBarButton::Emoji));
    layout.toggle(TabBarButton::Emoji);
    assert!(layout.is_visible(TabBarButton::Emoji));
}

#[test]
fn enabling_an_empty_symbol_page_restores_a_category_in_the_custom_order() {
    let mut layout = TabBarLayout::default();
    layout.move_button(TabBarButton::Gallery, TabBarButton::SymbolEmoji);
    for button in [
        TabBarButton::SymbolEmoji,
        TabBarButton::Symbols,
        TabBarButton::Gallery,
    ] {
        layout.toggle(button);
    }
    layout.toggle(TabBarButton::Emoji);
    assert!(layout.is_visible(TabBarButton::Emoji));
    assert_eq!(
        tabs::ordered_symbol_categories(&layout)
            .iter()
            .map(|category| category.id)
            .collect::<Vec<_>>(),
        vec![SymbolCategoryId::Gallery]
    );
}

#[test]
fn editor_and_main_window_use_the_same_icons_and_labels() {
    for tab in tabs::MAIN_TABS {
        assert_eq!(tab.id.layout_button().icon(), tab.icon);
        assert_eq!(tab.id.layout_button().label_key(), tab.label_key);
    }
    for filter in tabs::CONTENT_FILTERS.iter().chain(tabs::PASTE_FILTERS) {
        assert_eq!(filter.id.layout_button().icon(), filter.icon);
        assert_eq!(filter.id.layout_button().label_key(), filter.label_key);
    }
    for category in tabs::SYMBOL_CATEGORIES {
        assert_eq!(category.id.layout_button().label_key(), category.label_key);
        if category.id != SymbolCategoryId::Emoji {
            assert_eq!(category.id.layout_button().icon(), category.icon);
        }
    }
}
