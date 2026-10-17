-- 预览只索引固定长度前缀，避免长文本使索引体积膨胀；匹配后仍核验完整原文。
CREATE INDEX IF NOT EXISTS ix_clipboard_formats_text_merge
ON clipboard_formats(substr(preview_text, 1, 128), item_id)
WHERE format_kind = 'text';

CREATE INDEX IF NOT EXISTS ix_clipboard_formats_native_text_hash
ON clipboard_formats(format_namespace, format_key, data_hash, item_id)
WHERE format_namespace = 'windows' AND format_key = 'CF_UNICODETEXT';
