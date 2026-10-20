use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreviewInformationField {
    ContentType,
    RawSize,
    FileSize,
    Dimensions,
    Name,
    Path,
    Files,
    KnownSize,
    Characters,
    Lines,
    Format,
    Source,
    CapturedAt,
    PasteCount,
    Status,
    AvailableFormats,
    ZoomHint,
    ScrollHint,
    SwitchFormatHint,
}

impl PreviewInformationField {
    pub const ALL: [Self; 19] = [
        Self::ContentType,
        Self::RawSize,
        Self::FileSize,
        Self::Dimensions,
        Self::Name,
        Self::Path,
        Self::Files,
        Self::KnownSize,
        Self::Characters,
        Self::Lines,
        Self::Format,
        Self::Source,
        Self::CapturedAt,
        Self::PasteCount,
        Self::Status,
        Self::AvailableFormats,
        Self::ZoomHint,
        Self::ScrollHint,
        Self::SwitchFormatHint,
    ];

    pub const fn key(self) -> &'static str {
        match self {
            Self::ContentType => "content_type",
            Self::RawSize => "raw_size",
            Self::FileSize => "file_size",
            Self::Dimensions => "dimensions",
            Self::Name => "name",
            Self::Path => "path",
            Self::Files => "files",
            Self::KnownSize => "known_size",
            Self::Characters => "characters",
            Self::Lines => "lines",
            Self::Format => "format",
            Self::Source => "source",
            Self::CapturedAt => "captured_at",
            Self::PasteCount => "paste_count",
            Self::Status => "status",
            Self::AvailableFormats => "available_formats",
            Self::ZoomHint => "zoom_hint",
            Self::ScrollHint => "scroll_hint",
            Self::SwitchFormatHint => "switch_format_hint",
        }
    }

    pub const fn label_key(self) -> &'static str {
        match self {
            Self::ContentType => "settings.personalization.preview_information.content_type",
            Self::RawSize => "main.preview.info.raw_size",
            Self::FileSize => "main.preview.info.file_size",
            Self::Dimensions => "main.preview.info.dimensions",
            Self::Name => "main.preview.info.name",
            Self::Path => "main.preview.info.path",
            Self::Files => "main.preview.info.files",
            Self::KnownSize => "main.preview.info.known_size",
            Self::Characters => "main.preview.info.characters",
            Self::Lines => "main.preview.info.lines",
            Self::Format => "main.preview.info.format",
            Self::Source => "main.preview.info.source",
            Self::CapturedAt => "main.preview.info.captured_at",
            Self::PasteCount => "main.preview.info.paste_count",
            Self::Status => "main.preview.info.status",
            Self::AvailableFormats => "main.preview.info.available_formats",
            Self::ZoomHint => "settings.personalization.preview_information.zoom_hint",
            Self::ScrollHint => "settings.personalization.preview_information.scroll_hint",
            Self::SwitchFormatHint => {
                "settings.personalization.preview_information.switch_format_hint"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PreviewInformationConfig {
    pub fields: Vec<PreviewInformationField>,
}

impl Default for PreviewInformationConfig {
    fn default() -> Self {
        Self {
            fields: PreviewInformationField::ALL.to_vec(),
        }
    }
}

impl PreviewInformationConfig {
    pub fn enabled(&self, field: PreviewInformationField) -> bool {
        self.fields.contains(&field)
    }

    pub fn set_enabled(&mut self, field: PreviewInformationField, enabled: bool) {
        self.fields.retain(|value| *value != field);
        if enabled {
            self.fields.push(field);
        }
    }
}
