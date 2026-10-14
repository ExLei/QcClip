use crate::domain::capture::{CapturedClipboard, FormatKind, RawFormat, SourceApplication};

pub struct MockClipboardSource {
    captured: CapturedClipboard,
}

impl MockClipboardSource {
    pub fn text(text: &str) -> Self {
        let format = RawFormat::captured(
            "mime",
            "text/plain",
            "纯文本",
            FormatKind::Text,
            text.as_bytes().to_vec(),
            0,
            true,
        );
        Self {
            captured: CapturedClipboard {
                primary_format_identity: Some((
                    format.format_namespace.clone(),
                    format.format_key.clone(),
                )),
                formats: vec![format],
                preview_candidate: None,
                source_application: SourceApplication::default(),
            },
        }
    }

    pub fn text_with_failed_image(text: &str) -> Self {
        let mut source = Self::text(text);
        source.captured.formats.push(RawFormat::failed(
            "mime",
            "image/png",
            "PNG",
            FormatKind::Image,
            1,
            "读取失败",
            "模拟图片格式读取失败",
        ));
        source
    }

    pub fn capture(&self) -> CapturedClipboard {
        self.captured.clone()
    }
}
