use gpui::prelude::*;
use gpui::{App, Context, Hsla, Window, div, px, rgb};
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme as _, Disableable as _};

use crate::{
    domain::clipboard::Tag,
    i18n::{self, Locale, Translator},
    services::database::{TagCreateError, TagRepository},
    windows::dialog::{
        dialog_body, dialog_button, dialog_footer, dialog_header, dialog_primary_button,
    },
};

type TagSavedCallback = Box<dyn FnOnce(Tag, &mut App) + 'static>;

#[derive(Clone, Debug)]
enum TagFormError {
    EmptyName,
    NameTooLong,
    DuplicateName(String),
    InvalidColor,
    Other,
}

impl From<TagCreateError> for TagFormError {
    fn from(error: TagCreateError) -> Self {
        if matches!(&error, TagCreateError::Database(_) | TagCreateError::Sql(_) | TagCreateError::SortOrderOverflow) {
            tracing::error!(%error, "创建标签失败");
        }
        match error {
            TagCreateError::EmptyName => Self::EmptyName,
            TagCreateError::NameTooLong => Self::NameTooLong,
            TagCreateError::DuplicateName(name) => Self::DuplicateName(name),
            TagCreateError::InvalidColor => Self::InvalidColor,
            TagCreateError::SortOrderOverflow => Self::Other,
            TagCreateError::Database(_) | TagCreateError::Sql(_) => Self::Other,
        }
    }
}

impl TagFormError {
    fn message(&self, translator: &Translator) -> String {
        match self {
            Self::EmptyName => translator.translate("main.form.tag.error.empty_name"),
            Self::NameTooLong => translator.translate("main.form.tag.error.name_too_long"),
            Self::DuplicateName(name) => translator.translate_with_args(
                "main.form.tag.error.duplicate_name",
                &[("name", name.clone())],
            ),
            Self::InvalidColor => translator.translate("main.form.tag.error.invalid_color"),
            Self::Other => translator.translate("main.form.tag.error.other"),
        }
    }
}

pub struct NewTagForm {
    name_input: gpui::Entity<InputState>,
    selected_color: Option<usize>,
    custom_color: u32,
    input_locale: Locale,
    tag_repository: TagRepository,
    on_saved: Option<TagSavedCallback>,
    is_submitting: bool,
    submit_error: Option<TagFormError>,
}

impl NewTagForm {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        tag_repository: TagRepository,
        on_saved: impl FnOnce(Tag, &mut App) + 'static,
    ) -> Self {
        Self::with_tag(window, cx, tag_repository, on_saved)
    }

    fn with_tag(
        window: &mut Window,
        cx: &mut Context<Self>,
        tag_repository: TagRepository,
        on_saved: impl FnOnce(Tag, &mut App) + 'static,
    ) -> Self {
        let translator = i18n::translator(cx);
        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(translator.translate("main.form.tag.name_placeholder"))
        });
        let name_input_for_focus = name_input.clone();
        window.defer(cx, move |window, cx| {
            name_input_for_focus.update(cx, |input, cx| input.focus(window, cx));
        });
        Self {
            name_input,
            selected_color: Some(0),
            custom_color: 0x1677ff,
            input_locale: translator.locale().clone(),
            tag_repository,
            on_saved: Some(Box::new(on_saved)),
            is_submitting: false,
            submit_error: None,
        }
    }

    fn color_options(theme: gpui_component::ThemeColor) -> [Hsla; 6] {
        [
            theme.blue,
            theme.green,
            theme.red,
            theme.yellow,
            theme.magenta,
            theme.cyan,
        ]
    }

    fn render_color_swatch(
        index: usize,
        color: Hsla,
        selected: bool,
        theme: gpui_component::ThemeColor,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(("new-tag-color", index))
            .h(px(28.0))
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(7.0))
            .border_2()
            .border_color(if selected {
                theme.foreground
            } else {
                theme.border
            })
            .bg(color)
            .shadow_sm()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_color = Some(index);
                cx.notify();
            }))
    }

    fn open_custom_color_picker(&self, window: &mut Window, cx: &mut Context<Self>) {
        let form = cx.entity();
        let initial_color = self.custom_color;
        let result = crate::windows::color_picker::open_color_picker_window(
            Some(window),
            initial_color,
            move |color, cx| {
                form.update(cx, |form, cx| {
                    form.custom_color = color;
                    form.selected_color = None;
                    cx.notify();
                });
            },
            cx,
        );
        if let Err(error) = result {
            tracing::error!("打开自定义颜色窗口失败：{error}");
        }
    }

    fn selected_color_hex(&self, theme: gpui_component::ThemeColor) -> String {
        let color = self
            .selected_color
            .and_then(|index| Self::color_options(theme).get(index).copied())
            .map(gpui::hsla_to_rgba)
            .map(|color| {
                (color.color.red * 255.0).round() as u32 * 0x010000
                    + (color.color.green * 255.0).round() as u32 * 0x000100
                    + (color.color.blue * 255.0).round() as u32
            })
            .unwrap_or(self.custom_color);
        format!("#{color:06X}")
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_submitting {
            return;
        }

        let name = self.name_input.read(cx).value().trim().to_owned();
        if name.is_empty() {
            self.submit_error = Some(TagFormError::EmptyName);
            cx.notify();
            return;
        }

        let color = self.selected_color_hex(cx.theme().colors);
        let repository = self.tag_repository.clone();
        self.is_submitting = true;
        self.submit_error = None;
        cx.notify();
        let task = cx.background_spawn(async move {
            repository.create(&name, &color).map_err(TagFormError::from)
        });
        cx.spawn_in(window, async move |this, cx| match task.await {
            Ok(tag) => {
                let _ = this.update_in(cx, |this, window, cx| {
                    if let Some(on_saved) = this.on_saved.take() {
                        on_saved(tag, cx);
                    }
                    window.remove_window();
                });
            }
            Err(error) => {
                this.update(cx, |this, cx| {
                    this.is_submitting = false;
                    this.submit_error = Some(error);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }
}

impl Render for NewTagForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().colors;
        let translator = i18n::translator(cx);
        let focus_handle = cx.focus_handle();
        if self.input_locale != *translator.locale() {
            let placeholder = translator.translate("main.form.tag.name_placeholder");
            self.name_input.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
            self.input_locale = translator.locale().clone();
        }
        let colors = Self::color_options(theme);
        let selected_color = self.selected_color;
        let custom_color = self.custom_color;
        let submit_error = self
            .submit_error
            .as_ref()
            .map(|error| error.message(&translator));
        let is_submitting = self.is_submitting;
        let mut color_swatches = Vec::<gpui::AnyElement>::with_capacity(colors.len());
        for (index, color) in colors.into_iter().enumerate() {
            color_swatches.push(
                Self::render_color_swatch(index, color, selected_color == Some(index), theme, cx)
                    .into_any_element(),
            );
        }

        div()
            .id("new-tag-form")
            .w(px(320.0))
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .track_focus(&focus_handle)
            .child(dialog_header(
                translator.translate("main.form.tag.new"),
                theme,
            ))
            .child(dialog_body(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(5.0))
                            .child(field_label(
                                translator.translate("main.form.tag.name"),
                                theme,
                            ))
                            .child(
                                Input::new(&self.name_input)
                                    .w_full()
                                    .disabled(is_submitting),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(field_label(
                                translator.translate("main.form.tag.color"),
                                theme,
                            ))
                            .child(
                                div()
                                    .flex()
                                    .w_full()
                                    .items_center()
                                    .gap(px(7.0))
                                    .children(color_swatches)
                                    .child(
                                        div()
                                            .id("new-tag-custom-color")
                                            .h(px(28.0))
                                            .w(px(68.0))
                                            .flex_none()
                                            .px(px(5.0))
                                            .flex()
                                            .items_center()
                                            .gap(px(4.0))
                                            .rounded_md()
                                            .border_1()
                                            .border_color(if selected_color.is_none() {
                                                theme.primary
                                            } else {
                                                theme.border
                                            })
                                            .cursor_pointer()
                                            .hover(|this| this.bg(theme.secondary_hover))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.open_custom_color_picker(window, cx);
                                            }))
                                            .child(
                                                div()
                                                    .size(px(14.0))
                                                    .rounded_full()
                                                    .bg(rgb(custom_color)),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.0))
                                                    .text_color(theme.muted_foreground)
                                                    .child(
                                                        translator.translate(
                                                            "main.form.tag.custom_color",
                                                        ),
                                                    ),
                                            ),
                                    ),
                            ),
                    )
                    .when_some(submit_error, |this, message| {
                        this.child(
                            div()
                                .text_size(px(11.0))
                                .text_color(theme.red)
                                .child(message),
                        )
                    })
                    .child(dialog_footer(
                        vec![
                            dialog_button(
                                "new-tag-cancel",
                                translator.translate("main.form.cancel"),
                            )
                            .disabled(is_submitting)
                            .on_click(|_, window: &mut Window, _| window.remove_window())
                            .into_any_element(),
                            dialog_primary_button(
                                "new-tag-confirm",
                                translator.translate("main.form.confirm"),
                            )
                            .label(if is_submitting {
                                translator.translate("main.form.tag.saving")
                            } else {
                                translator.translate("main.form.confirm")
                            })
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

fn field_label(label: String, theme: gpui_component::ThemeColor) -> impl IntoElement {
    div()
        .text_size(px(12.0))
        .text_color(theme.foreground)
        .child(label)
}
