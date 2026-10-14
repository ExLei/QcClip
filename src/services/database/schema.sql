CREATE TABLE binary_blobs (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    sha256 BLOB NOT NULL CHECK(length(sha256) = 32) UNIQUE,
    byte_length INTEGER NOT NULL CHECK(byte_length >= 0),
    relative_path TEXT NOT NULL UNIQUE,
    storage_state TEXT NOT NULL DEFAULT 'pending'
        CHECK(storage_state IN ('pending', 'ready', 'missing', 'corrupt')),
    last_error TEXT,
    created_at INTEGER NOT NULL,
    last_verified_at INTEGER
);

CREATE TABLE application_icon_assets (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    sha256 BLOB NOT NULL CHECK(length(sha256) = 32) UNIQUE,
    relative_path TEXT NOT NULL UNIQUE,
    byte_length INTEGER NOT NULL CHECK(byte_length > 0),
    width INTEGER NOT NULL CHECK(width > 0),
    height INTEGER NOT NULL CHECK(height > 0),
    created_at INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL
);

CREATE TABLE application_icon_cache (
    app_key TEXT PRIMARY KEY NOT NULL,
    app_path TEXT NOT NULL,
    file_size INTEGER NOT NULL CHECK(file_size >= 0),
    modified_at INTEGER NOT NULL CHECK(modified_at >= 0),
    icon_asset_id INTEGER NOT NULL
        REFERENCES application_icon_assets(id) ON DELETE RESTRICT,
    last_checked_at INTEGER NOT NULL
);

CREATE INDEX ix_application_icon_cache_asset
ON application_icon_cache(icon_asset_id);

CREATE TABLE clipboard_items (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    origin_kind TEXT NOT NULL DEFAULT 'captured'
        CHECK(origin_kind IN ('captured', 'manual', 'edited', 'imported', 'synced')),
    parent_item_id INTEGER REFERENCES clipboard_items(id) ON DELETE SET NULL,
    hash_version INTEGER NOT NULL CHECK(hash_version > 0),
    content_hash BLOB NOT NULL CHECK(length(content_hash) = 32),
    content_kind TEXT NOT NULL
        CHECK(length(trim(content_kind)) > 0),
    capture_state TEXT NOT NULL DEFAULT 'pending'
        CHECK(capture_state IN ('pending', 'ready', 'corrupt')),
    preview_text TEXT,
    preview_char_count INTEGER CHECK(preview_char_count IS NULL OR preview_char_count >= 0),
    format_count INTEGER NOT NULL DEFAULT 0 CHECK(format_count >= 0),
    total_bytes INTEGER NOT NULL DEFAULT 0 CHECK(total_bytes >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(hash_version, content_hash)
);

CREATE INDEX ix_clipboard_items_capture_state_id
ON clipboard_items(capture_state, id);

CREATE INDEX ix_clipboard_items_parent_item_id
ON clipboard_items(parent_item_id);

CREATE TABLE clipboard_formats (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    item_id INTEGER NOT NULL
        REFERENCES clipboard_items(id) ON DELETE CASCADE,
    format_order INTEGER NOT NULL CHECK(format_order >= 0),
    format_namespace TEXT NOT NULL
        CHECK(length(trim(format_namespace)) > 0),
    format_key TEXT NOT NULL,
    format_name TEXT NOT NULL,
    native_format_id INTEGER CHECK(native_format_id IS NULL OR native_format_id >= 0),
    format_kind TEXT NOT NULL
        CHECK(length(trim(format_kind)) > 0),
    is_primary INTEGER NOT NULL DEFAULT 0 CHECK(is_primary IN (0, 1)),
    storage_kind TEXT NOT NULL DEFAULT 'none'
        CHECK(storage_kind IN ('inline', 'external', 'none')),
    inline_data BLOB,
    blob_id INTEGER REFERENCES binary_blobs(id) ON DELETE RESTRICT,
    byte_length INTEGER NOT NULL DEFAULT 0 CHECK(byte_length >= 0),
    data_hash BLOB CHECK(data_hash IS NULL OR length(data_hash) = 32),
    restore_state TEXT NOT NULL DEFAULT 'pending'
        CHECK(restore_state IN ('pending', 'restorable', 'not_restorable', 'not_applicable')),
    image_width INTEGER CHECK(image_width IS NULL OR image_width > 0),
    image_height INTEGER CHECK(image_height IS NULL OR image_height > 0),
    restore_error_code TEXT,
    restore_error_message TEXT,
    mime_type TEXT,
    encoding TEXT,
    preview_text TEXT,
    preview_char_count INTEGER CHECK(preview_char_count IS NULL OR preview_char_count >= 0),
    preview_content_kind TEXT
        CHECK(preview_content_kind IS NULL OR length(trim(preview_content_kind)) > 0),
    capture_state TEXT NOT NULL
        CHECK(capture_state IN ('captured', 'generated', 'unsupported', 'failed')),
    error_code TEXT,
    error_message TEXT,
    created_at INTEGER NOT NULL,
    CHECK(
        (preview_text IS NULL AND preview_char_count IS NULL AND preview_content_kind IS NULL)
        OR (preview_text IS NOT NULL AND preview_char_count IS NOT NULL AND preview_content_kind IS NOT NULL)
    ),
    CHECK(
        (storage_kind = 'inline' AND inline_data IS NOT NULL AND blob_id IS NULL)
        OR (storage_kind = 'external' AND inline_data IS NULL AND blob_id IS NOT NULL)
        OR (storage_kind = 'none' AND inline_data IS NULL AND blob_id IS NULL)
    ),
    CHECK(
        (capture_state IN ('captured', 'generated')
            AND storage_kind IN ('inline', 'external')
            AND data_hash IS NOT NULL
            AND restore_state IN ('pending', 'restorable', 'not_restorable'))
        OR (capture_state IN ('unsupported', 'failed')
            AND storage_kind = 'none'
            AND data_hash IS NULL
            AND restore_state = 'not_applicable')
    ),
    CHECK(
        is_primary = 0
        OR (capture_state IN ('captured', 'generated') AND restore_state = 'restorable')
    ),
    CHECK(storage_kind <> 'inline' OR length(inline_data) = byte_length),
    UNIQUE(item_id, format_order),
    UNIQUE(item_id, format_namespace, format_key)
);

CREATE UNIQUE INDEX ux_clipboard_formats_primary
ON clipboard_formats(item_id)
WHERE is_primary = 1;

CREATE INDEX ix_clipboard_formats_blob
ON clipboard_formats(blob_id);

CREATE INDEX ix_clipboard_formats_item_content_kind
ON clipboard_formats(item_id, preview_content_kind);

CREATE TRIGGER ck_clipboard_items_start_pending
BEFORE INSERT ON clipboard_items
WHEN NEW.capture_state <> 'pending'
BEGIN
    SELECT RAISE(ABORT, '内容必须从 pending 状态开始捕获');
END;

CREATE TRIGGER ck_clipboard_items_ready
BEFORE UPDATE OF capture_state, format_count, total_bytes ON clipboard_items
WHEN NEW.capture_state = 'ready'
 AND (
    NOT EXISTS (
        SELECT 1 FROM clipboard_formats
        WHERE item_id = NEW.id
          AND capture_state IN ('captured', 'generated')
    )
    OR NOT EXISTS (
        SELECT 1 FROM clipboard_formats
        WHERE item_id = NEW.id
          AND is_primary = 1
          AND capture_state IN ('captured', 'generated')
          AND restore_state = 'restorable'
    )
    OR NEW.format_count <> (
        SELECT COUNT(*) FROM clipboard_formats WHERE item_id = NEW.id
    )
    OR NEW.total_bytes <> COALESCE((
        SELECT SUM(CASE WHEN capture_state IN ('captured', 'generated') THEN byte_length ELSE 0 END)
        FROM clipboard_formats WHERE item_id = NEW.id
    ), 0)
 )
BEGIN
    SELECT RAISE(ABORT, '内容原始格式未完成或统计值不一致');
END;

CREATE TABLE clipboard_history (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    item_id INTEGER NOT NULL UNIQUE
        REFERENCES clipboard_items(id) ON DELETE CASCADE,
    first_seen_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    capture_count INTEGER NOT NULL DEFAULT 1 CHECK(capture_count >= 1),
    paste_count INTEGER NOT NULL DEFAULT 0 CHECK(paste_count >= 0),
    last_pasted_at INTEGER,
    is_pinned INTEGER NOT NULL DEFAULT 0 CHECK(is_pinned IN (0, 1)),
    sort_order INTEGER NOT NULL CHECK(sort_order >= 0),
    last_source_app_name TEXT,
    last_source_app_path TEXT,
    last_source_app_icon_id INTEGER
        REFERENCES application_icon_assets(id) ON DELETE SET NULL,
    updated_at INTEGER NOT NULL,
    CHECK(last_seen_at >= first_seen_at),
    UNIQUE(is_pinned, sort_order)
);

CREATE INDEX ix_clipboard_history_list
ON clipboard_history(is_pinned DESC, sort_order DESC, last_seen_at DESC, id DESC);

CREATE INDEX ix_clipboard_history_source_app_name
ON clipboard_history(last_source_app_name);

CREATE TABLE favorites (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    item_id INTEGER NOT NULL UNIQUE
        REFERENCES clipboard_items(id) ON DELETE RESTRICT,
    is_pinned INTEGER NOT NULL DEFAULT 0 CHECK(is_pinned IN (0, 1)),
    sort_order INTEGER NOT NULL CHECK(sort_order >= 0),
    paste_count INTEGER NOT NULL DEFAULT 0 CHECK(paste_count >= 0),
    last_pasted_at INTEGER,
    added_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(is_pinned, sort_order)
);

CREATE INDEX ix_favorites_list
ON favorites(is_pinned DESC, sort_order DESC, added_at DESC, id DESC);

CREATE TABLE tags (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL CHECK(length(trim(name)) > 0 AND length(name) <= 100),
    normalized_name TEXT NOT NULL UNIQUE
        CHECK(length(trim(normalized_name)) > 0 AND length(normalized_name) <= 100),
    color TEXT NOT NULL DEFAULT '#1677FF'
        CHECK(color GLOB '#[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f]'),
    sort_order INTEGER NOT NULL DEFAULT 0 CHECK(sort_order >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX ux_tags_sort_order
ON tags(sort_order);

CREATE TABLE clipboard_item_tags (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    item_id INTEGER NOT NULL
        REFERENCES clipboard_items(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL
        REFERENCES tags(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(item_id, tag_id)
);

CREATE INDEX ix_clipboard_item_tags_tag
ON clipboard_item_tags(tag_id, item_id);

CREATE INDEX ix_clipboard_item_tags_item
ON clipboard_item_tags(item_id, tag_id);

CREATE TABLE clipboard_item_notes (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    item_id INTEGER NOT NULL UNIQUE
        REFERENCES clipboard_items(id) ON DELETE CASCADE,
    note_text TEXT NOT NULL CHECK(length(trim(note_text)) > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE database_compatibility (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    minimum_read_version INTEGER NOT NULL CHECK(minimum_read_version >= 1),
    minimum_write_version INTEGER NOT NULL
        CHECK(minimum_write_version >= minimum_read_version)
);

INSERT INTO database_compatibility (id, minimum_read_version, minimum_write_version)
VALUES (1, 1, 1);

CREATE TABLE legacy_import_records (
    source_key TEXT PRIMARY KEY NOT NULL,
    item_id INTEGER REFERENCES clipboard_items(id) ON DELETE SET NULL,
    imported_at INTEGER NOT NULL
);
