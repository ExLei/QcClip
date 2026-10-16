use crate::ui::scale::ui_rem;

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{App, Context, IntoElement, PromptButton, PromptLevel, Window, div};
use gpui_component::button::ButtonVariants as _;
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme as _, Disableable as _};

use crate::{
    i18n::{self, Locale},
    services::image_library::{ImageLibraryGroup, ImageLibraryService},
    ui::{hover_icon, icon},
    windows::dialog::{
        dialog_body, dialog_button, dialog_footer, dialog_header, dialog_primary_button,
    },
};

type GroupSavedCallback = Box<dyn FnOnce(ImageLibraryGroup, &mut App) + 'static>;
type GroupDeletedCallback = Box<dyn FnOnce(String, &mut App) + 'static>;

pub struct NewGalleryGroupForm {
    name_input: gpui::Entity<InputState>,
    icon_path: Option<PathBuf>,
    existing_icon_path: Option<PathBuf>,
    editing_group: Option<ImageLibraryGroup>,
    input_locale: Locale,
    image_library: ImageLibraryService,
    on_saved: Option<GroupSavedCallback>,
    on_deleted: Option<GroupDeletedCallback>,
    is_submitting: bool,
    is_deleting: bool,
    submit_error: Option<String>,
}

impl NewGalleryGroupForm {
    pub(crate) fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        image_library: ImageLibraryService,
        on_saved: impl FnOnce(ImageLibraryGroup, &mut App) + 'static,
    ) -> Self {
        Self::with_group(window, cx, image_library, None, on_saved)
    }

    pub(crate) fn edit(
        window: &mut Window,
        cx: &mut Context<Self>,
        image_library: ImageLibraryService,
        group: ImageLibraryGroup,
        on_saved: impl FnOnce(ImageLibraryGroup, &mut App) + 'static,
        on_deleted: impl FnOnce(String, &mut App) + 'static,
    ) -> Self {
        let mut form = Self::with_group(window, cx, image_library, Some(group), on_saved);
        form.on_deleted = Some(Box::new(on_deleted));
        form
    }

    fn with_group(
        window: &mut Window,
        cx: &mut Context<Self>,
        image_library: ImageLibraryService,
        editing_group: Option<ImageLibraryGroup>,
        on_saved: impl FnOnce(ImageLibraryGroup, &mut App) + 'static,
    ) -> Self {
        let translator = i18n::translator(cx);
        let initial_name = editing_group.as_ref().map(|group| group.name.clone());
        let existing_icon_path = editing_group
            .as_ref()
            .and_then(|group| image_library.group_icon_path(group));
        let name_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx)
                .placeholder(translator.translate("main.form.gallery_group.name_placeholder"));
            if let Some(name) = initial_name {
                input = input.default_value(name);
            }
            input
        });
        let input_for_focus = name_input.clone();
        window.defer(cx, move |window, cx| {
            input_for_focus.update(cx, |input, cx| input.focus(window, cx));
        });
        Self {
            name_input,
            icon_path: None,
            existing_icon_path,
            editing_group,
            input_locale: translator.locale().clone(),
            image_library,
            on_saved: Some(Box::new(on_saved)),
            on_deleted: None,
            is_submitting: false,
            is_deleting: false,
            submit_error: None,
        }
    }

    fn choose_icon(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_submitting || self.is_deleting {
            return;
        }
        let translator = i18n::translator(cx);
        let dialog = rfd::AsyncFileDialog::new()
            .set_title(translator.translate("main.form.gallery_group.icon_dialog_title"))
            .add_filter("图片文件", &["png", "jpg", "jpeg", "gif", "webp"]);
        let form = cx.entity();
        cx.spawn(async move |_, cx| {
            let Some(file) = dialog.pick_file().await else {
                return;
            };
            let path = file.path().to_path_buf();
            cx.update(|cx| {
                form.update(cx, |this, cx| {
                    this.icon_path = Some(path);
                    this.submit_error = None;
                    cx.notify();
                });
            });
        })
        .detach();
        let _ = window;
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_submitting || self.is_deleting {
            return;
        }
        let name = self.name_input.read(cx).value().trim().to_owned();
        if name.is_empty() {
            self.submit_error =
                Some(i18n::translator(cx).translate("main.form.gallery_group.error.empty_name"));
            cx.notify();
            return;
        }

        let image_library = self.image_library.clone();
        let icon_path = self.icon_path.clone();
        let editing_group = self.editing_group.clone();
        self.is_submitting = true;
        self.submit_error = None;
        cx.notify();
        let task = cx.background_spawn(async move {
            if let Some(group) = editing_group {
                image_library.update_group(&group, &name, icon_path.as_deref())
            } else {
                image_library.create_group(&name, icon_path.as_deref())
            }
        });
        cx.spawn_in(window, async move |this, cx| match task.await {
            Ok(group) => {
                let _ = this.update_in(cx, |this, window, cx| {
                    if let Some(on_saved) = this.on_saved.take() {
                        on_saved(group, cx);
                    }
                    window.remove_window();
                });
            }
            Err(error) => {
                tracing::error!(%error, "保存图库组失败");
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

    fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_submitting || self.is_deleting {
            return;
        }
        let Some(group) = self.editing_group.clone() else {
            return;
        };
        let translator = i18n::translator(cx);
        let title = translator.translate_with_args(
            "main.form.gallery_group.delete_confirm_title",
            &[("name", group.name.clone())],
        );
        let detail = translator.translate("main.form.gallery_group.delete_confirm_detail");
        let answer = window.prompt(
            PromptLevel::Warning,
            &title,
            Some(&detail),
            &[
                PromptButton::cancel(translator.translate("main.form.cancel")),
                PromptButton::ok(translator.translate("main.form.gallery_group.delete")),
            ],
            cx,
        );
        self.is_deleting = true;
        self.submit_error = None;
        cx.notify();
        let image_library = self.image_library.clone();
        cx.spawn_in(window, async move |this, cx| {
            if !matches!(answer.await, Ok(1)) {
                this.update(cx, |this, cx| {
                    this.is_deleting = false;
                    cx.notify();
                })
                .ok();
                return;
            }
            let group_id = group.id.clone();
            let task = cx.background_spawn(async move { image_library.delete_group(&group) });
            match task.await {
                Ok(()) => {
                    this.update_in(cx, |this, window, cx| {
                        if let Some(on_deleted) = this.on_deleted.take() {
                            on_deleted(group_id, cx);
                        }
                        window.remove_window();
                    })
                    .ok();
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        tracing::error!("删除图库组失败：{error}");
                        this.is_deleting = false;
                        this.submit_error = Some(error);
                        cx.notify();
                    })
                    .ok();
                }
            }
        })
        .detach();
    }
}

impl Render for NewGalleryGroupForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().colors;
        let translator = i18n::translator(cx);
        let focus_handle = cx.focus_handle();
        if self.input_locale != *translator.locale() {
            self.name_input.update(cx, |input, cx| {
                input.set_placeholder(
                    translator.translate("main.form.gallery_group.name_placeholder"),
                    window,
                    cx,
                )
            });
            self.input_locale = translator.locale().clone();
        }
        let icon_name = self
            .icon_path
            .as_ref()
            .or(self.existing_icon_path.as_ref())
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned());
        let icon_label = icon_name
            .unwrap_or_else(|| translator.translate("main.form.gallery_group.choose_icon"));
        let submit_error = self.submit_error.clone();
        let is_submitting = self.is_submitting;
        let is_busy = is_submitting || self.is_deleting;
        let title_key = if self.editing_group.is_some() {
            "main.form.gallery_group.edit"
        } else {
            "main.form.gallery_group.new"
        };

        let mut actions = Vec::new();
        if self.editing_group.is_some() {
            actions.push(
                dialog_button(
                    "gallery-group-delete",
                    translator.translate(if self.is_deleting {
                        "main.form.gallery_group.deleting"
                    } else {
                        "main.form.gallery_group.delete"
                    }),
                )
                .danger()
                .icon(icon("trash", 13.0, theme.danger_foreground))
                .disabled(is_busy)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.confirm_delete(window, cx);
                }))
                .into_any_element(),
            );
            actions.push(div().flex_1().into_any_element());
        }
        actions.extend([
            dialog_button(
                "new-gallery-group-cancel",
                translator.translate("main.form.cancel"),
            )
            .disabled(is_busy)
            .on_click(|_, window: &mut Window, _| window.remove_window())
            .into_any_element(),
            dialog_primary_button(
                "new-gallery-group-confirm",
                translator.translate(if is_submitting {
                    "main.form.gallery_group.saving"
                } else {
                    "main.form.confirm"
                }),
            )
            .disabled(is_busy)
            .on_click(cx.listener(|this, _, window, cx| {
                this.submit(window, cx);
            }))
            .into_any_element(),
        ]);

        div()
            .id("new-gallery-group-form")
            .w(ui_rem(320.0))
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .track_focus(&focus_handle)
            .child(dialog_header(translator.translate(title_key), theme))
            .child(dialog_body(
                div()
                    .flex()
                    .flex_col()
                    .gap(ui_rem(12.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(ui_rem(5.0))
                            .child(field_label(
                                translator.translate("main.form.gallery_group.name"),
                                theme,
                            ))
                            .child(Input::new(&self.name_input).w_full().disabled(is_busy)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(ui_rem(6.0))
                            .child(field_label(
                                translator.translate("main.form.gallery_group.icon"),
                                theme,
                            ))
                            .child(
                                div()
                                    .id("gallery-group-choose-icon")
                                    .h(ui_rem(34.0))
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .gap(ui_rem(7.0))
                                    .px(ui_rem(9.0))
                                    .rounded_md()
                                    .border_1()
                                    .border_color(theme.border)
                                    .cursor_pointer()
                                    .hover(|this| this.bg(theme.secondary_hover))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.choose_icon(window, cx);
                                    }))
                                    .child(hover_icon(
                                        "gallery-group-icon-picker",
                                        "photo",
                                        16.0,
                                        theme.muted_foreground,
                                    ))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .text_size(ui_rem(12.0))
                                            .text_color(theme.muted_foreground)
                                            .child(icon_label),
                                    ),
                            ),
                    )
                    .when_some(submit_error, |this, message| {
                        this.child(
                            div()
                                .text_size(ui_rem(11.0))
                                .text_color(theme.red)
                                .child(message),
                        )
                    })
                    .child(dialog_footer(actions, theme)),
            ))
    }
}

fn field_label(label: String, theme: gpui_component::ThemeColor) -> impl IntoElement {
    div()
        .text_size(ui_rem(12.0))
        .text_color(theme.foreground)
        .child(label)
}
