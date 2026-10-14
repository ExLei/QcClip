use serde::{Deserialize, Serialize};

#[repr(u32)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum TabId {
    Clipboard,
    Favorites,
    Emoji,
}

#[derive(Clone, Copy, Debug)]
pub struct TabDefinition {
    pub id: TabId,
    pub label_key: &'static str,
    pub icon: &'static str,
}

pub const MAIN_TABS: &[TabDefinition] = &[
    TabDefinition {
        id: TabId::Clipboard,
        label_key: "main.tab.clipboard",
        icon: "clipboard-text",
    },
    TabDefinition {
        id: TabId::Favorites,
        label_key: "main.tab.favorites",
        icon: "star",
    },
    TabDefinition {
        id: TabId::Emoji,
        label_key: "main.tab.emoji",
        icon: "mood-smile",
    },
];

#[repr(u32)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum SymbolCategoryId {
    Emoji,
    Symbols,
    Gallery,
}

#[derive(Clone, Copy, Debug)]
pub struct SymbolCategoryDefinition {
    pub id: SymbolCategoryId,
    pub label_key: &'static str,
    pub icon: &'static str,
}

pub const SYMBOL_CATEGORIES: &[SymbolCategoryDefinition] = &[
    SymbolCategoryDefinition {
        id: SymbolCategoryId::Emoji,
        label_key: "main.symbol.emoji",
        icon: "😀",
    },
    SymbolCategoryDefinition {
        id: SymbolCategoryId::Symbols,
        label_key: "main.symbol.symbols",
        icon: "math-symbols",
    },
    SymbolCategoryDefinition {
        id: SymbolCategoryId::Gallery,
        label_key: "main.symbol.gallery",
        icon: "photo-star",
    },
];

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FilterId {
    Text,
    Image,
    File,
    Link,
    Unpasted,
    Pasted,
}

#[derive(Clone, Copy, Debug)]
pub struct FilterDefinition {
    pub id: FilterId,
    pub label_key: &'static str,
    pub icon: &'static str,
}

pub const CONTENT_FILTERS: &[FilterDefinition] = &[
    FilterDefinition {
        id: FilterId::Text,
        label_key: "main.filter.text",
        icon: "file-text",
    },
    FilterDefinition {
        id: FilterId::Image,
        label_key: "main.filter.image",
        icon: "photo",
    },
    FilterDefinition {
        id: FilterId::File,
        label_key: "main.filter.file",
        icon: "folder",
    },
    FilterDefinition {
        id: FilterId::Link,
        label_key: "main.filter.link",
        icon: "link",
    },
];

pub const PASTE_FILTERS: &[FilterDefinition] = &[
    FilterDefinition {
        id: FilterId::Unpasted,
        label_key: "main.filter.unpasted",
        icon: "clipboard-x",
    },
    FilterDefinition {
        id: FilterId::Pasted,
        label_key: "main.filter.pasted",
        icon: "clipboard-check",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_tabs_and_filters_have_stable_unique_ids() {
        let tab_ids: Vec<_> = MAIN_TABS.iter().map(|tab| tab.id).collect();
        let filter_ids: Vec<_> = CONTENT_FILTERS
            .iter()
            .chain(PASTE_FILTERS.iter())
            .map(|filter| filter.id)
            .collect();

        assert_eq!(
            tab_ids,
            vec![TabId::Clipboard, TabId::Favorites, TabId::Emoji]
        );
        assert_eq!(filter_ids.len(), 6);
        for (index, id) in filter_ids.iter().enumerate() {
            assert!(!filter_ids[..index].contains(id));
        }

        let symbol_category_ids: Vec<_> = SYMBOL_CATEGORIES
            .iter()
            .map(|category| category.id)
            .collect();
        assert_eq!(
            symbol_category_ids,
            vec![
                SymbolCategoryId::Emoji,
                SymbolCategoryId::Symbols,
                SymbolCategoryId::Gallery,
            ]
        );
    }
}
