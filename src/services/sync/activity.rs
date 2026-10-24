use rusqlite::{Connection, OptionalExtension, params};

use super::{Result, protocol::Mode};
use crate::services::database::Database;

pub struct LatestUpload {
    pub item_id: i64,
    pub mode: Mode,
    pub preview: String,
}

pub(crate) fn record_local(
    connection: &Connection,
    item_id: i64,
    favorite: bool,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO sync_local_activity (slot, item_id, favorite) VALUES (1, ?1, ?2)
         ON CONFLICT(slot) DO UPDATE SET item_id = excluded.item_id, favorite = excluded.favorite",
        params![item_id, favorite],
    )?;
    Ok(())
}

pub fn latest(database: &Database) -> Result<Option<LatestUpload>> {
    let connection = database.connect_read()?;
    Ok(connection
        .query_row(
            "SELECT i.id, a.favorite, substr(COALESCE(i.preview_text, ''), 1, 80)
         FROM sync_local_activity a JOIN clipboard_items i ON i.id = a.item_id
         WHERE a.slot = 1 AND i.capture_state = 'ready'
           AND ((a.favorite = 1 AND EXISTS(SELECT 1 FROM favorites WHERE item_id = i.id))
             OR (a.favorite = 0 AND EXISTS(SELECT 1 FROM clipboard_history WHERE item_id = i.id)))",
            [],
            |row| {
                Ok(LatestUpload {
                    item_id: row.get(0)?,
                    mode: if row.get::<_, bool>(1)? {
                        Mode::ManualFavorite
                    } else {
                        Mode::ManualContent
                    },
                    preview: row
                        .get::<_, String>(2)?
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" "),
                })
            },
        )
        .optional()?)
}
