use crate::ui::scale::ui_rem;

use gpui::prelude::*;
use gpui::{App, Context, IntoElement, Window, div};
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme as _, Disableable as _};

use crate::{
    i18n,
    windows::dialog::{
        dialog_body, dialog_button, dialog_footer, dialog_header, dialog_primary_button,
    },
};

type SavedCallback = Box<dyn FnOnce(String, &mut App) + 'static>;

pub(crate) struct RenameGalleryImageForm {
    name_input: gpui::Entity<InputState>,
    on_saved: Option<SavedCallback>,
    is_submitting: bool,
    submit_error: Option<String>,
}

impl RenameGalleryImageForm {
    pub(crate) fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        name: String,
        on_saved: impl FnOnce(String, &mut App) + 'static,
    ) -> Self {
        let translator = i18n::translator(cx);
        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(name)
                .placeholder(translator.translate("main.form.gallery_image.name_placeholder"))
        });
        let input_for_focus = name_input.clone();
        window.defer(cx, move |window, cx| {
            input_for_focus.update(cx, |input, cx| input.focus(window, cx));
        });
        Self {
            name_input,
            on_saved: Some(Box::new(on_saved)),
            is_submitting: false,
            submit_error: None,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_submitting {
            return;
        }
        let name = self.name_input.read(cx).value().trim().to_owned();
        if name.is_empty() {
            self.submit_error =
                Some(i18n::translator(cx).translate("main.form.gallery_image.error.empty_name"));
            cx.notify();
            return;
        }
        self.is_submitting = true;
        self.submit_error = None;
        let callback = self.on_saved.take();
        if let Some(callback) = callback {
            callback(name, cx);
        }
        window.remove_window();
    }
}

impl Render for RenameGalleryImageForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().colors;
        let translator = i18n::translator(cx);
        let focus_handle = cx.focus_handle();
        let submit_error = self.submit_error.clone();
        let is_submitting = self.is_submitting;
        div()
            .id("rename-gallery-image-form")
            .w(ui_rem(320.0))
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .track_focus(&focus_handle)
            .child(dialog_header(
                translator.translate("main.form.gallery_image.rename"),
                theme,
            ))
            .child(dialog_body(
                div()
                    .flex()
                    .flex_col()
                    .gap(ui_rem(8.0))
                    .child(
                        Input::new(&self.name_input)
                            .w_full()
                            .disabled(is_submitting),
                    )
                    .when_some(submit_error, |this, message| {
                        this.child(
                            div()
                                .text_size(ui_rem(11.0))
                                .text_color(theme.red)
                                .child(message),
                        )
                    })
                    .child(dialog_footer(
                        vec![
                            dialog_button(
                                "rename-gallery-image-cancel",
                                translator.translate("main.form.cancel"),
                            )
                            .disabled(is_submitting)
                            .on_click(|_, window: &mut Window, _| window.remove_window())
                            .into_any_element(),
                            dialog_primary_button(
                                "rename-gallery-image-confirm",
                                translator.translate("main.form.confirm"),
                            )
                            .disabled(is_submitting)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.submit(window, cx);
                            }))
                            .into_any_element(),
                        ],
                        theme,
                    )),
            ))
    }
}
