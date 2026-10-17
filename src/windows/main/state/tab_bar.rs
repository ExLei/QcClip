use super::*;
use crate::{
    config::TabBarButton,
    config_runtime,
    windows::main::tabs::{
        CONTENT_FILTERS, MAIN_TABS, PASTE_FILTERS, SYMBOL_CATEGORIES, TAB_BAR_HOVER_DELAY,
    },
};

impl MainWindow {
    pub(crate) fn cancel_tab_bar_hover(&mut self) {
        self.tab_bar_hover.cancel();
        self.tab_bar_hover_task = None;
        self.tab_bar_hover_hitbox = None;
    }

    pub(crate) fn suppress_tab_bar_hover(&mut self, button: TabBarButton) {
        self.tab_bar_hover.suppress(button);
        self.tab_bar_hover_task = None;
    }

    pub(crate) fn update_tab_bar_hover(
        &mut self,
        button: TabBarButton,
        hitbox: Option<gpui::HitboxId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if button == TabBarButton::Tags {
            self.update_tags_panel_auto_close(window, cx);
        }
        let hovered = window.is_window_hovered()
            && !cx.has_active_drag()
            && config_runtime::clipboard_config(cx).tab_bar_hover_switch
            && hitbox.is_some_and(|hitbox| hitbox.is_hovered(window));
        if !hovered {
            if self.tab_bar_hover.leave(button) {
                self.tab_bar_hover_task = None;
                self.tab_bar_hover_hitbox = None;
            }
            if let Some(filter) = CONTENT_FILTERS
                .iter()
                .chain(PASTE_FILTERS.iter())
                .find(|filter| filter.id.layout_button() == button)
            {
                self.filters.set_hovered(filter.id, false);
            }
            return;
        }
        self.tab_bar_hover_hitbox = hitbox.map(|hitbox| (button, hitbox));
        if !self.tab_bar_hover.enter(button, Instant::now()) {
            return;
        }
        self.tab_bar_hover_task = Some(cx.spawn_in(window, async move |this, cx| {
            let mut delay = TAB_BAR_HOVER_DELAY;
            loop {
                cx.background_executor().timer(delay).await;
                let next_delay = this
                    .update_in(cx, |this, window, cx| {
                        let remaining = this.tab_bar_hover.remaining(button, Instant::now())?;
                        if !window.is_window_hovered()
                            || cx.has_active_drag()
                            || !config_runtime::clipboard_config(cx).tab_bar_hover_switch
                            || !config_runtime::appearance_config(cx)
                                .tab_bar
                                .is_visible(button)
                            || !this.tab_bar_hover_hitbox.is_some_and(|(current, hitbox)| {
                                current == button && hitbox.is_hovered(window)
                            })
                        {
                            this.cancel_tab_bar_hover();
                            return None;
                        }
                        if !remaining.is_zero() {
                            return Some(remaining);
                        }
                        this.tab_bar_hover.take_ready(button, Instant::now());
                        this.tab_bar_hover_task = None;
                        if let Some(tab) = MAIN_TABS
                            .iter()
                            .find(|tab| tab.id.layout_button() == button)
                        {
                            this.select_tab(tab.id, cx);
                        } else if let Some(filter) = CONTENT_FILTERS
                            .iter()
                            .chain(PASTE_FILTERS.iter())
                            .find(|filter| filter.id.layout_button() == button)
                        {
                            if this.filters.set_hovered(filter.id, true) {
                                this.refresh_history(cx);
                            }
                        } else if let Some(category) = SYMBOL_CATEGORIES
                            .iter()
                            .find(|category| category.id.layout_button() == button)
                        {
                            this.select_symbol_category(category.id, cx);
                        } else if button == TabBarButton::Tags {
                            this.cancel_tags_panel_close();
                            this.tags_panel_open = true;
                            cx.notify();
                        }
                        None
                    })
                    .ok()
                    .flatten();
                let Some(remaining) = next_delay else {
                    break;
                };
                delay = remaining;
            }
        }));
    }
}
