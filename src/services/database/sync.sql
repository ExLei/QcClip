CREATE TABLE IF NOT EXISTS sync_items (
    device_id TEXT NOT NULL,
    item_uuid TEXT NOT NULL,
    item_id INTEGER REFERENCES clipboard_items(id) ON DELETE SET NULL,
    source_hash BLOB NOT NULL CHECK(length(source_hash) = 32),
    local_hash BLOB NOT NULL CHECK(length(local_hash) = 32),
    PRIMARY KEY(device_id, item_uuid)
);
CREATE TABLE IF NOT EXISTS sync_watermarks (
    device_id TEXT NOT NULL,
    item_uuid TEXT NOT NULL,
    stream TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    PRIMARY KEY(device_id, item_uuid, stream)
);
CREATE TABLE IF NOT EXISTS sync_receipts (
    device_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    expires INTEGER NOT NULL,
    PRIMARY KEY(device_id, event_id)
);
CREATE TABLE IF NOT EXISTS sync_local_activity (
    slot INTEGER PRIMARY KEY CHECK(slot = 1),
    item_id INTEGER REFERENCES clipboard_items(id) ON DELETE SET NULL,
    favorite INTEGER NOT NULL CHECK(favorite IN (0, 1))
);
CREATE TABLE IF NOT EXISTS sync_sources (
    item_id INTEGER PRIMARY KEY REFERENCES clipboard_items(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL,
    device_name TEXT NOT NULL,
    app_name TEXT
);
