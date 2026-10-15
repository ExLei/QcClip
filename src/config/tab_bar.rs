use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TabBarSection {
    Pages,
    Filters,
    Tags,
}

impl TabBarSection {
    pub const ALL: [Self; 3] = [Self::Pages, Self::Filters, Self::Tags];

    pub const fn label_key(self) -> &'static str {
        match self {
            Self::Pages => "settings.personalization.tab_bar.pages",
            Self::Filters => "settings.personalization.tab_bar.filters",
            Self::Tags => "main.tags.title",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabBarButtonGroup {
    Pages,
    Filters,
    Symbols,
    Tags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TabBarButton {
    Clipboard,
    Favorites,
    Emoji,
    Text,
    Image,
    File,
    Link,
    Unpasted,
    Pasted,
    SymbolEmoji,
    Symbols,
    Gallery,
    Tags,
}

impl TabBarButton {
    pub const ALL: [Self; 13] = [
        Self::Clipboard,
        Self::Favorites,
        Self::Emoji,
        Self::Text,
        Self::Image,
        Self::File,
        Self::Link,
        Self::Unpasted,
        Self::Pasted,
        Self::SymbolEmoji,
        Self::Symbols,
        Self::Gallery,
        Self::Tags,
    ];

    pub const fn group(self) -> TabBarButtonGroup {
        match self {
            Self::Clipboard | Self::Favorites | Self::Emoji => TabBarButtonGroup::Pages,
            Self::Text | Self::Image | Self::File | Self::Link | Self::Unpasted | Self::Pasted => {
                TabBarButtonGroup::Filters
            }
            Self::SymbolEmoji | Self::Symbols | Self::Gallery => TabBarButtonGroup::Symbols,
            Self::Tags => TabBarButtonGroup::Tags,
        }
    }

    pub const fn label_key(self) -> &'static str {
        match self {
            Self::Clipboard => "main.tab.clipboard",
            Self::Favorites => "main.tab.favorites",
            Self::Emoji => "main.tab.emoji",
            Self::Text => "main.filter.text",
            Self::Image => "main.filter.image",
            Self::File => "main.filter.file",
            Self::Link => "main.filter.link",
            Self::Unpasted => "main.filter.unpasted",
            Self::Pasted => "main.filter.pasted",
            Self::SymbolEmoji => "main.symbol.emoji",
            Self::Symbols => "main.symbol.symbols",
            Self::Gallery => "main.symbol.gallery",
            Self::Tags => "main.tags.title",
        }
    }

    pub const fn icon(self) -> &'static str {
        match self {
            Self::Clipboard => "clipboard-text",
            Self::Favorites => "star",
            Self::Emoji | Self::SymbolEmoji => "mood-smile",
            Self::Text => "file-text",
            Self::Image => "photo",
            Self::File => "folder",
            Self::Link => "link",
            Self::Unpasted => "clipboard-x",
            Self::Pasted => "clipboard-check",
            Self::Symbols => "math-symbols",
            Self::Gallery => "photo-star",
            Self::Tags => "tags",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TabBarLayout {
    pub sections: Vec<TabBarSection>,
    pub buttons: Vec<TabBarButton>,
    pub hidden: Vec<TabBarButton>,
}

impl Default for TabBarLayout {
    fn default() -> Self {
        Self {
            sections: TabBarSection::ALL.to_vec(),
            buttons: TabBarButton::ALL.to_vec(),
            hidden: Vec::new(),
        }
    }
}

impl TabBarLayout {
    pub fn is_visible(&self, button: TabBarButton) -> bool {
        !self.hidden.contains(&button)
            && (button != TabBarButton::Emoji
                || self
                    .buttons_in(TabBarButtonGroup::Symbols)
                    .any(|category| !self.hidden.contains(&category)))
    }

    pub fn buttons_in(&self, group: TabBarButtonGroup) -> impl Iterator<Item = TabBarButton> + '_ {
        self.buttons
            .iter()
            .copied()
            .filter(move |button| button.group() == group)
    }

    pub fn toggle(&mut self, button: TabBarButton) {
        if self.is_visible(button) {
            self.hidden.push(button);
        } else {
            self.hidden.retain(|hidden| *hidden != button);
            if button == TabBarButton::Emoji
                && !self
                    .buttons_in(TabBarButtonGroup::Symbols)
                    .any(|category| self.is_visible(category))
            {
                let category = self
                    .buttons_in(TabBarButtonGroup::Symbols)
                    .next()
                    .unwrap_or(TabBarButton::SymbolEmoji);
                self.hidden.retain(|hidden| *hidden != category);
            }
        }
    }

    pub fn move_section(&mut self, source: TabBarSection, target: TabBarSection) {
        move_item(&mut self.sections, source, target);
    }

    pub fn move_button(&mut self, source: TabBarButton, target: TabBarButton) {
        if source.group() == target.group() {
            move_item(&mut self.buttons, source, target);
        }
    }

    pub(crate) fn normalize(&mut self) {
        complete_order(&mut self.sections, &TabBarSection::ALL);
        complete_order(&mut self.buttons, &TabBarButton::ALL);
        let mut hidden = Vec::new();
        for button in self.hidden.drain(..) {
            if !hidden.contains(&button) {
                hidden.push(button);
            }
        }
        self.hidden = hidden;
    }
}

fn move_item<T: Copy + PartialEq>(items: &mut Vec<T>, source: T, target: T) {
    let Some(source_index) = items.iter().position(|item| *item == source) else {
        return;
    };
    let Some(target_index) = items.iter().position(|item| *item == target) else {
        return;
    };
    let item = items.remove(source_index);
    items.insert(target_index, item);
}

fn complete_order<T: Copy + PartialEq>(items: &mut Vec<T>, defaults: &[T]) {
    let mut ordered = Vec::with_capacity(defaults.len());
    for item in items.iter().chain(defaults) {
        if !ordered.contains(item) {
            ordered.push(*item);
        }
    }
    *items = ordered;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_appearance_config_gets_the_default_layout() {
        let appearance: super::super::AppearanceConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(appearance.tab_bar, TabBarLayout::default());
    }

    #[test]
    fn normalize_preserves_custom_order_and_restores_missing_buttons() {
        let mut layout = TabBarLayout {
            sections: vec![TabBarSection::Tags, TabBarSection::Tags],
            buttons: vec![TabBarButton::Image, TabBarButton::Image, TabBarButton::Text],
            hidden: vec![TabBarButton::Text, TabBarButton::Text],
        };
        layout.normalize();
        assert_eq!(
            layout.sections,
            vec![
                TabBarSection::Tags,
                TabBarSection::Pages,
                TabBarSection::Filters
            ]
        );
        assert_eq!(
            layout
                .buttons_in(TabBarButtonGroup::Filters)
                .collect::<Vec<_>>(),
            vec![
                TabBarButton::Image,
                TabBarButton::Text,
                TabBarButton::File,
                TabBarButton::Link,
                TabBarButton::Unpasted,
                TabBarButton::Pasted,
            ]
        );
        assert_eq!(layout.buttons.len(), TabBarButton::ALL.len());
        assert_eq!(layout.hidden, vec![TabBarButton::Text]);
    }

    #[test]
    fn reordering_is_limited_to_the_same_button_group() {
        let mut layout = TabBarLayout::default();
        layout.move_section(TabBarSection::Tags, TabBarSection::Pages);
        assert_eq!(
            layout.sections,
            vec![
                TabBarSection::Tags,
                TabBarSection::Pages,
                TabBarSection::Filters
            ]
        );
        layout.move_button(TabBarButton::Text, TabBarButton::Pasted);
        let filters = layout
            .buttons_in(TabBarButtonGroup::Filters)
            .collect::<Vec<_>>();
        assert_eq!(filters.last(), Some(&TabBarButton::Text));
        layout.move_button(TabBarButton::Text, TabBarButton::Image);
        assert_eq!(
            layout.buttons_in(TabBarButtonGroup::Filters).next(),
            Some(TabBarButton::Text)
        );
        let before = layout.clone();
        layout.move_button(TabBarButton::Text, TabBarButton::Clipboard);
        layout.move_button(TabBarButton::SymbolEmoji, TabBarButton::Text);
        assert_eq!(layout, before);
    }

    #[test]
    fn visibility_and_order_survive_serialization_and_can_all_be_disabled() {
        let mut layout = TabBarLayout::default();
        layout.move_button(TabBarButton::Emoji, TabBarButton::Clipboard);
        for button in TabBarButton::ALL {
            layout.toggle(button);
        }
        layout.normalize();
        assert!(
            TabBarButton::ALL
                .iter()
                .all(|button| !layout.is_visible(*button))
        );
        let encoded = serde_json::to_string(&layout).unwrap();
        assert_eq!(
            serde_json::from_str::<TabBarLayout>(&encoded).unwrap(),
            layout
        );
        layout.toggle(TabBarButton::Clipboard);
        assert!(layout.is_visible(TabBarButton::Clipboard));
    }
}
