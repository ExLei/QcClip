use std::collections::HashSet;

use sha2::{Digest, Sha256};

use crate::domain::capture::CapturedClipboard;

use super::CaptureError;

pub const CONTENT_HASH_VERSION: i64 = 2;
const HASH_DOMAIN: &[u8] = b"QcClip/content-hash";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HashedClipboard {
    pub hash_version: i64,
    pub content_hash: [u8; 32],
    pub format_hashes: Vec<Option<[u8; 32]>>,
}

pub fn hash_clipboard(captured: &CapturedClipboard) -> Result<HashedClipboard, CaptureError> {
    let mut identities = HashSet::new();
    let mut successful_formats = Vec::new();
    let mut format_hashes = Vec::with_capacity(captured.formats.len());

    for (index, format) in captured.formats.iter().enumerate() {
        if format.capture_state.has_data() {
            let bytes = format.bytes.as_deref().ok_or_else(|| {
                CaptureError::InvalidFormat(format!(
                    "格式“{}”标记为成功但缺少原始字节",
                    format.format_name
                ))
            })?;
            if !identities.insert(format.stable_identity()) {
                return Err(CaptureError::InvalidFormat(format!(
                    "捕获结果包含重复格式身份“{}/{}”",
                    format.format_namespace, format.format_key
                )));
            }
            let data_hash: [u8; 32] = Sha256::digest(bytes).into();
            successful_formats.push((index, format, bytes));
            format_hashes.push(Some(data_hash));
        } else {
            if format.bytes.is_some() {
                return Err(CaptureError::InvalidFormat(format!(
                    "失败格式“{}”不能携带原始字节",
                    format.format_name
                )));
            }
            format_hashes.push(None);
        }
    }
    if successful_formats.is_empty() {
        return Err(CaptureError::NoCapturedFormats);
    }

    let content_hash = hash_format_set(
        successful_formats
            .iter()
            .map(|(_, format, bytes)| {
                (
                    format.format_namespace.as_str(),
                    format.format_key.as_str(),
                    *bytes,
                )
            }),
    );

    Ok(HashedClipboard {
        hash_version: CONTENT_HASH_VERSION,
        content_hash,
        format_hashes,
    })
}

pub(crate) fn hash_format_set<'a>(
    formats: impl IntoIterator<Item = (&'a str, &'a str, &'a [u8])>,
) -> [u8; 32] {
    let mut formats = formats.into_iter().collect::<Vec<_>>();
    formats.sort_by(|left, right| (left.0, left.1).cmp(&(right.0, right.1)));

    let mut hasher = Sha256::new();
    write_length_prefixed(&mut hasher, HASH_DOMAIN);
    hasher.update((CONTENT_HASH_VERSION as u32).to_be_bytes());
    hasher.update((formats.len() as u64).to_be_bytes());
    for (namespace, key, bytes) in formats {
        write_length_prefixed(&mut hasher, namespace.as_bytes());
        write_length_prefixed(&mut hasher, key.as_bytes());
        write_length_prefixed(&mut hasher, bytes);
    }
    hasher.finalize().into()
}

fn write_length_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

#[cfg(test)]
mod tests {
    use crate::domain::capture::{FormatKind, RawFormat, SourceApplication};

    use super::*;

    fn captured(formats: Vec<RawFormat>) -> CapturedClipboard {
        CapturedClipboard {
            formats,
            primary_format_identity: None,
            preview_candidate: None,
            source_application: SourceApplication::default(),
        }
    }

    #[test]
    fn format_order_and_business_metadata_do_not_change_content_hash() {
        let text = RawFormat::captured(
            "mime",
            "text/plain",
            "纯文本",
            FormatKind::Text,
            b"hello".to_vec(),
            0,
            true,
        );
        let html = RawFormat::captured(
            "mime",
            "text/html",
            "HTML",
            FormatKind::Html,
            b"<b>hello</b>".to_vec(),
            1,
            true,
        );
        let mut first = captured(vec![text.clone(), html.clone()]);
        first.source_application.name = Some("应用 A".to_owned());
        let mut second = captured(vec![html, text]);
        second.source_application.name = Some("应用 B".to_owned());

        assert_eq!(
            hash_clipboard(&first).expect("无法计算哈希").content_hash,
            hash_clipboard(&second).expect("无法计算哈希").content_hash
        );
    }

    #[test]
    fn raw_byte_changes_update_format_and_content_hash() {
        let first = captured(vec![RawFormat::captured(
            "mime",
            "text/plain",
            "纯文本",
            FormatKind::Text,
            b"hello".to_vec(),
            0,
            true,
        )]);
        let second = captured(vec![RawFormat::captured(
            "mime",
            "text/plain",
            "纯文本",
            FormatKind::Text,
            b"world".to_vec(),
            0,
            true,
        )]);
        let first_hash = hash_clipboard(&first).expect("无法计算第一个哈希");
        let second_hash = hash_clipboard(&second).expect("无法计算第二个哈希");

        assert_ne!(first_hash.content_hash, second_hash.content_hash);
        assert_ne!(first_hash.format_hashes, second_hash.format_hashes);
    }

    #[test]
    fn failed_formats_are_excluded_from_content_hash() {
        let text = RawFormat::captured(
            "mime",
            "text/plain",
            "纯文本",
            FormatKind::Text,
            b"hello".to_vec(),
            0,
            true,
        );
        let first = captured(vec![text.clone()]);
        let second = captured(vec![
            text,
            RawFormat::failed(
                "mime",
                "image/png",
                "PNG",
                FormatKind::Image,
                1,
                "读取失败",
                "测试失败",
            ),
        ]);

        assert_eq!(
            hash_clipboard(&first).expect("无法计算哈希").content_hash,
            hash_clipboard(&second).expect("无法计算哈希").content_hash
        );
    }
}
