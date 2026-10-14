use super::*;

#[derive(Clone, Copy)]
pub(super) struct ListItemMetrics {
    pub(super) rem_size: Pixels,
    pub(super) row_height: Pixels,
    pub(super) content_height: Pixels,
    pub(super) card_vertical_inset: Pixels,
    pub(super) style: ListItemStyle,
    pub(super) vertical_padding: Pixels,
    pub(super) meta_row_height: Pixels,
    pub(super) preview_font_size: Pixels,
    pub(super) preview_line_height: Pixels,
    pub(super) preview_height: Pixels,
    pub(super) height: ListItemHeight,
    pub(super) adaptive_max_lines: usize,
    pub(super) preview_line_count: usize,
    pub(super) show_meta: bool,
    pub(super) only_show_notes: bool,

}

impl ListItemMetrics {
    pub(super) fn from_rem_size(
        rem_size: Pixels,
        style: ListItemStyle,
        card_item_gap: CardItemGap,
        height: ListItemHeight,
        adaptive_max_lines: u8,
    ) -> Self {
        let vertical_padding = rem_size * ROW_VERTICAL_PADDING_SCALE;
        let preview_font_size = rem_size * PREVIEW_FONT_SCALE;
        let preview_line_height = preview_font_size * PREVIEW_LINE_HEIGHT_SCALE;
        let meta_row_height = rem_size * META_ROW_HEIGHT_SCALE;
        let adaptive_max_lines =
            usize::from(adaptive_max_lines.clamp(ADAPTIVE_MAX_LINES_MIN, ADAPTIVE_MAX_LINES_MAX));
        let preview_line_count = height
            .fixed_preview_lines()
            .unwrap_or(DEFAULT_ADAPTIVE_PREVIEW_LINES.min(adaptive_max_lines));
        let card_vertical_inset = match style {
            ListItemStyle::Seamless => px(0.0),
            ListItemStyle::Card => px(card_item_gap.pixels() / 2.0),
        };
        let metrics = Self {
            rem_size,
            row_height: px(0.0),
            content_height: px(0.0),
            card_vertical_inset,
            style,
            vertical_padding,
            meta_row_height,
            preview_font_size,
            preview_line_height,
            preview_height: px(0.0),
            height,
            adaptive_max_lines,
            preview_line_count,
            show_meta: height.shows_meta(),
            only_show_notes: false,
        };
        metrics.with_preview_line_count(preview_line_count)
    }

    pub(super) fn with_preview_line_count(self, preview_line_count: usize) -> Self {
        let max_preview_lines = self
            .height
            .fixed_preview_lines()
            .unwrap_or(self.adaptive_max_lines);
        self.with_preview_line_count_capped(preview_line_count, max_preview_lines)
    }

    pub(super) fn with_preview_line_count_capped(
        self,
        preview_line_count: usize,
        max_preview_lines: usize,
    ) -> Self {
        let max_preview_lines = max_preview_lines.max(1);
        let preview_line_count = preview_line_count.max(1).min(max_preview_lines);
        let preview_height = self.preview_line_height * preview_line_count as f32;
        self.with_preview_height(preview_height, preview_line_count)
    }

    pub(super) fn with_preview_height(mut self, preview_height: Pixels, preview_line_count: usize) -> Self {
        self.preview_line_count = preview_line_count;
        self.preview_height = preview_height;
        self.content_height = self.vertical_padding * 2.0
            + if self.show_meta {
                self.meta_row_height
            } else {
                px(0.0)
            }
            + self.preview_height
            + px(MAX_ROW_BORDER_WIDTH);
        self.row_height = match self.style {
            ListItemStyle::Seamless => self.content_height,
            ListItemStyle::Card => self.content_height + self.card_vertical_inset * 2.0,
        };
        self
    }

    pub(super) fn resource_preview_height(self) -> Pixels {
        if self.height == ListItemHeight::Adaptive {
            // 自适应项可以比大行高更高，但图片解码缩略图只按大行高限制尺寸。
            let large_preview_lines = ListItemHeight::Large.fixed_preview_lines().unwrap_or(1);
            self.preview_line_height * large_preview_lines as f32
        } else {
            self.preview_height
        }
    }

    pub(super) fn with_adaptive_image_dimensions(
        self,
        dimensions: ImageDimensions,
        available_width: Pixels,
    ) -> Self {
        let max_preview_height = self.preview_line_height * ADAPTIVE_IMAGE_PREVIEW_LINES as f32;
        let natural_height = f32::from(available_width).max(1.0) * dimensions.height as f32
            / dimensions.width as f32;
        let preview_height = px(natural_height.clamp(
            f32::from(self.preview_line_height),
            f32::from(max_preview_height),
        ));
        let preview_line_count =
            (f32::from(preview_height) / f32::from(self.preview_line_height)).ceil() as usize;
        self.with_preview_height(preview_height, preview_line_count)
    }

    pub(super) fn for_preview(
        self,
        preview: Option<&FormatPreview>,
        file_count: Option<usize>,
        available_width: Pixels,
        window: &Window,
    ) -> Self {
        if preview.is_some_and(|preview| preview.content_kind == ContentKind::FileList) {
            let preview_height = file_list_preview_height(
                self.height,
                self.rem_size,
                self.adaptive_max_lines,
                file_count,
            );
            let preview_line_count = self.height.fixed_preview_lines().unwrap_or_else(|| {
                (f32::from(preview_height) / f32::from(self.preview_line_height))
                    .ceil()
                    .max(1.0) as usize
            });
            return self.with_preview_height(preview_height, preview_line_count);
        }

        if self.height != ListItemHeight::Adaptive {
            return self;
        }

        if preview.is_some_and(|preview| preview.content_kind == ContentKind::Image) {
            if let Some(dimensions) = preview.and_then(|preview| preview.image_dimensions) {
                return self.with_adaptive_image_dimensions(dimensions, available_width);
            }
            return self.with_preview_line_count_capped(
                ADAPTIVE_IMAGE_PREVIEW_LINES,
                ADAPTIVE_IMAGE_PREVIEW_LINES,
            );
        }

        let preview_line_count = preview
            .and_then(|preview| (!preview.text.is_empty()).then_some(preview.text.as_str()))
            .map(|text| {
                measure_text_lines(
                    text,
                    available_width,
                    self.preview_font_size,
                    self.adaptive_max_lines,
                    window,
                )
            })
            .unwrap_or(DEFAULT_ADAPTIVE_PREVIEW_LINES.min(self.adaptive_max_lines));
        self.with_preview_line_count(preview_line_count)
    }
}

pub(super) fn estimate_text_lines(
    text: &str,
    available_width: Pixels,
    font_size: Pixels,
    max_lines: usize,
) -> usize {
    let max_lines = max_lines.max(1);
    let width_in_em = (f32::from(available_width) / f32::from(font_size)).max(1.0);
    let mut line_count = 1;
    let mut line_width = 0.0;
    for character in text.chars() {
        if character == '\n' {
            line_count += 1;
            line_width = 0.0;
        } else {
            let character_width = if character == '\t' {
                4.0
            } else if character.is_ascii() {
                0.55
            } else {
                1.0
            };
            if line_width > 0.0 && line_width + character_width > width_in_em {
                line_count += 1;
                line_width = character_width;
            } else {
                line_width += character_width;
            }
        }
        if line_count >= max_lines {
            return max_lines;
        }
    }
    line_count.clamp(1, max_lines)
}

pub(super) fn measure_text_lines(
    text: &str,
    available_width: Pixels,
    font_size: Pixels,
    max_lines: usize,
    window: &Window,
) -> usize {
    let text_style = window.text_style();
    let runs = [text_style.to_run(text.len())];
    window
        .text_system()
        .shape_text(
            text.to_owned(),
            font_size,
            &runs,
            Some(available_width),
            Some(max_lines.max(1)),
        )
        .map(|lines| {
            shaped_line_count(
                lines.iter().map(|line| line.wrap_boundaries().len()),
                max_lines,
            )
        })
        .unwrap_or_else(|_| estimate_text_lines(text, available_width, font_size, max_lines))
}

pub(super) fn shaped_line_count(
    wrap_boundary_counts: impl IntoIterator<Item = usize>,
    max_lines: usize,
) -> usize {
    let line_count = wrap_boundary_counts
        .into_iter()
        .map(|wrap_count| wrap_count.saturating_add(1))
        .sum::<usize>();
    line_count.clamp(1, max_lines.max(1))
}

pub(super) fn available_preview_width(window_width: Pixels, style: ListItemStyle) -> Pixels {
    let card_inset = if style == ListItemStyle::Card {
        CARD_HORIZONTAL_INSET * 2.0
    } else {
        0.0
    };
    px((f32::from(window_width) - ROW_HORIZONTAL_PADDING * 2.0 - card_inset).max(1.0))
}

pub(super) fn item_preview(
    item: &ClipboardListItem,
    display_priority: ClipboardDisplayPriority,
) -> Option<FormatPreview> {
    select_user_visible_previews_with_priority(
        &item.preview_options,
        &display_priority_roles(display_priority),
    )
    .into_iter()
    .next()
}

pub(super) fn resolve_row_height(
    history_id: Option<HistoryItemId>,
    height: ListItemHeight,
    measured_height: Option<Pixels>,
    placeholder_height: Pixels,
    adaptive_item_heights: &mut HashMap<HistoryItemId, Pixels>,
) -> Pixels {
    if height != ListItemHeight::Adaptive {
        return placeholder_height;
    }
    if let Some(measured_height) = measured_height {
        if let Some(history_id) = history_id {
            adaptive_item_heights.insert(history_id, measured_height);
        }
        measured_height
    } else if let Some(history_id) = history_id {
        adaptive_item_heights
            .get(&history_id)
            .copied()
            .unwrap_or(placeholder_height)
    } else {
        placeholder_height
    }
}

impl ListItemMetrics {
    pub(super) fn with_note_display(mut self, only_show_notes: bool) -> Self {
        self.only_show_notes = only_show_notes;
        self
    }

    pub(super) fn shows_only_note(self, note: &str) -> bool {
        !note.is_empty() && (self.only_show_notes || self.height == ListItemHeight::ExtraSmall)
    }

    pub(super) fn for_item(
        self,
        item: &ClipboardListItem,
        display_priority: ClipboardDisplayPriority,
        file_count: Option<usize>,
        available_width: Pixels,
        window: &Window,
    ) -> Self {
        if self.shows_only_note(&item.note_text) {
            if self.height != ListItemHeight::Adaptive {
                return self;
            }
            return self.with_preview_line_count(measure_text_lines(
                &item.note_text, available_width, self.preview_font_size,
                self.adaptive_max_lines, window,
            ));
        }
        let preview = item_preview(item, display_priority);
        let metrics = self.for_preview(preview.as_ref(), file_count, available_width, window);
        if !item.note_text.is_empty() && self.height == ListItemHeight::Adaptive {
            metrics.with_preview_height(
                metrics.preview_height + metrics.preview_line_height,
                metrics.preview_line_count + 1,
            )
        } else {
            metrics
        }
    }
}

#[cfg(test)]
mod note_tests {
    use super::*;

    #[test]
    fn notes_replace_content_only_for_extra_small_or_explicit_setting() {
        for height in ListItemHeight::ALL {
            let metrics = ListItemMetrics::from_rem_size(
                px(16.0), ListItemStyle::Card, CardItemGap::Eight, height, 8,
            );
            assert!(!metrics.shows_only_note(""));
            assert_eq!(metrics.shows_only_note("备注"), height == ListItemHeight::ExtraSmall);
            let notes_only = metrics.with_note_display(true);
            assert!(notes_only.shows_only_note("备注"));
            assert!(!notes_only.shows_only_note(""));
            assert_eq!(notes_only.row_height, metrics.row_height);
        }
    }
}

pub(super) fn file_list_view_for_item<'a>(
    item: &ClipboardListItem,
    display_priority: ClipboardDisplayPriority,
    file_list_views: &'a HashMap<FileListKey, Entity<FileListView>>,
) -> Option<&'a Entity<FileListView>> {
    let preview = item_preview(item, display_priority)?;
    if preview.format_kind != FormatKind::FileList
        || preview.content_kind != ContentKind::FileList
        || preview.image_source.is_none()
    {
        return None;
    }
    file_list_views.get(&FileListKey::new(
        item.item_id,
        &preview.format_namespace,
        &preview.format_key,
    ))
}

pub(super) fn file_list_meta_label(translator: &Translator, file_count: usize) -> String {
    translator.translate_with_args(
        "main.preview.file_list",
        &[("count", file_count.to_string())],
    )
}

pub(super) fn image_meta_label(translator: &Translator, dimensions: Option<ImageDimensions>) -> String {
    let Some(dimensions) = dimensions else {
        return translator.translate("main.preview.image");
    };
    translator.translate_with_args(
        "main.preview.image_dimensions",
        &[
            ("width", dimensions.width.to_string()),
            ("height", dimensions.height.to_string()),
        ],
    )
}
